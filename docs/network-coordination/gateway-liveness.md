# Liveness-gated internet gateways (mjolnir-mesh-5lw)

Status: design sketch. Supersedes the implicit auto-gateway mechanism of `chj`
(Phase 1) and absorbs `42j` (Phase 2 reachability probe). Tier-1 hardening
(`bii`, the babeld stale-route flush) ships independently and is a prerequisite,
not a replacement.

## The problem this fixes

`chj` made every node render `redistribute ip 0.0.0.0/0 le 0 metric 128`
unconditionally and delegated *all* runtime behaviour to babeld + the kernel
FIB. Gateway presence is implicit (a route happens to be in babeld's table) and
gateway **absence is never positively determined** — a gateway is "gone" only
when babeld times its route out. That is exactly where a stale/orphaned
`proto babel` default lies to the mesh: a no-WAN node re-exports it as a bogus
`0.0.0.0/0`, and the real gateway can learn a proto-babel default over its own
DHCP WAN default and blackhole its egress into the mesh. Field symptom
(2026-07-08): a freshly plugged uplink does not propagate to already-running
nodes; only a reboot clears the FIB.

Root cause in one line: **gateway-ness is a side effect of the FIB, not an
asserted, liveness-gated fact.** "Lack of liveness isn't advertised."

## The idea

Make gateway-ness a **positively-asserted, liveness-gated fact** carried on the
existing ephemeral beacon plane (`e21.9`, `crdt/liveness.rs`), and let **meshd
own both the advertise decision and the withdraw decision** instead of trusting
babeld's implicit redistribution.

Two independent levers, either shippable alone:

### Lever 1 — dynamic advertise (the core fix)

Today `gateway: bool` is captured once at daemon start (`mjolnir-meshd.rs:4784`)
and the `0.0.0.0/0` line is rendered unconditionally. Change it so the babel
reconciler samples **local egress health every tick** and renders the line
**iff** this node is a confirmed local gateway:

```
render 0.0.0.0/0  ⇔  gateway_mode != never
                     ∧ a non-babel kernel default exists
                       whose oif ∉ {br-mesh, mjolnir0}      (buw.7 exclusion)
                     ∧ egress probe healthy                  (42j: ICMP + HTTP-204,
                                                              hysteresis N-up/M-down)
```

`BabelConfigInputs.gateway` becomes a per-tick sampled value, not a constant.
The reconciler already re-renders every 5s and `write_atomic_if_changed` +
procd handle the restart. Consequences:

- A no-WAN node **never renders the line**, so it structurally *cannot*
  re-export a stale proto-babel default. The hijack becomes impossible by
  construction — independent of `bii`'s flush (which remains as defence in
  depth for the crash/interrupted-restart window).
- A node on a dead/captive lease fails the probe → does not advertise →
  no black-hole announce. This is `42j`, folded in here.
- `gateway_mode` knob (`auto|always|never`, UCI `option gateway`) is honoured
  in meshd: `never` = never render; `always` = skip the probe (trust the
  kernel default); `auto` = full gate above.

### Lever 2 — liveness-gated consume (the durable part)

Advertise the local egress fact on the beacon plane and let every node build a
**live-gateway set** it can positively expire.

New ephemeral gossip variant (appended last — wire-safe; old nodes decode-error
and the `GossipSync` recv loop log-and-skips, same as every prior variant, and
old nodes have no gateway logic to miss):

```rust
GatewayBeacon {
    node_id: String,
    incarnation: u64,   // reuse the liveness incarnation (boot wall-clock ms)
    counter: u64,       // reuse the liveness per-boot tick counter
    egress: EgressAd,   // { healthy: bool, cost_hint: u16 }  — cost_hint mirrors
                        // the babel metric headroom (e.g. RTT/hop estimate)
}
```

Keep `LivenessBeacon` **unchanged and still emitted** so liveness of
services/`.mesh`/address-book keeps working with un-upgraded nodes. New nodes
emit both. A `GatewayTracker` (thin wrapper over the same `Seen`/`superseded_by`
logic, or literally a second `LivenessTracker` keyed to gateway ads) marks a
gateway stale via `is_stale(node_id, now)` within `stale_threshold_ms` — a
gateway that stops beaconing is **known gone**, not guessed-gone.

meshd then:
- Keeps babeld as the **forwarding-plane installer** (don't reinvent
  nearest-exit metric math — babeld does it well). babeld still learns/installs
  the actual default route.
- Uses the live-gateway set as an **authoritative overlay**: when a gateway goes
  stale in `GatewayTracker`, meshd proactively `ip route flush`-es any learned
  default toward that origin instead of waiting for babeld's hold-time. This is
  the positive-withdraw signal that closes "lack of liveness isn't advertised".
- Exposes the live-gateway set to status/DNS/ops (`meshctl`, hello) so operators
  can see who the current egresses are and why.

## Why the beacon plane and not an HLC / a CRDT book

Same rationale as `e21.9` (see `lane-staleness.md`): encoding gateway state in a
durable CRDT entry's HLC forces a flash write every anti-entropy tick (the `7bf`
churn). Gateway-ness is recency-of-contact, not a value to converge — it belongs
on the ephemeral, receiver-clock-judged, skew-immune beacon plane. `incarnation`
already handles restart with zero persisted state.

## Migration / wire-compat

1. Ship `bii` (stale-route flush) first — safe on the current fleet, no protocol
   change. **[done: commit ac2f397]**
2. Add `GatewayBeacon` variant (appended last) + `GatewayTracker`, emit-only, no
   behaviour change. Mixed fleet safe (old nodes skip it).
3. Flip Lever 1: gate the render on local egress. Roll node-by-node; a mixed
   fleet just means some nodes still advertise unconditionally (today's
   behaviour) while upgraded ones are correct — monotonic improvement.
4. Flip Lever 2: meshd consumes the tracker and owns proactive withdraw.
5. Once the whole fleet is upgraded, optionally collapse `GatewayBeacon` into
   `LivenessBeacon` (single beacon carries `Option<EgressAd>`).

## Test plan

- Unit: `GatewayTracker` staleness/incarnation reuse mirrors `liveness.rs` tests.
- Render: `render_babeld_conf` gains a `local_egress` gate — a no-WAN node emits
  no `0.0.0.0/0`; a probe-failed node emits none; a healthy gateway emits it.
- Convergence test (like `service_mesh_convergence.rs`): gateway appears →
  consumers see it live within a tick; gateway killed → consumers mark it stale
  within `stale_threshold_ms` and flush the default; partition/rejoin restores.
- On-hardware (the `chj` invariants, now enforced not hoped): no-WAN node never
  exports `0.0.0.0/0` (SIGUSR1 xroute dump); plug/unplug on the gateway with
  other nodes running propagates **without rebooting them**; dead-lease node
  does not become egress.

## Open questions

- Does meshd's proactive flush race babeld's own reinstall? Gate the flush on
  "origin stale AND no live gateway advertises a path" to avoid flapping.
- `cost_hint` vs letting babel's metric fully decide — start with babel deciding
  (cost_hint informational only), promote to a tie-breaker if multi-gateway
  selection misbehaves in the field.
- Interaction with `mode=internet`/`buw.7`: the oif exclusion in Lever 1 must
  also gate what counts as "local egress" for the beacon, else the overlay's own
  uplink gets re-announced into the overlay.
