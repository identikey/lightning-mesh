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

**No mixed-fleet constraint** (decision 2026-07-08: 4 prototype nodes, all
upgrade together — see `bd memories mixed-fleet`). So don't add a second
variant; extend `LivenessBeacon` itself to carry the egress fact:

```rust
LivenessBeacon {
    node_id: String,
    incarnation: u64,          // boot wall-clock ms (unchanged)
    counter: u64,              // per-boot tick sequence (unchanged)
    egress: Option<EgressAd>,  // NEW: Some(..) iff this node is a live local
                               // gateway this tick. { healthy: bool, cost_hint: u16 }
}
```

One beacon, one tick, one tracker — gateway-ness and node liveness share fate by
construction (you can't be "live but with a stale gateway status"). Extend
`LivenessTracker` to stash the last-accepted `egress` alongside `Seen`, exposing
`live_gateways(now) -> impl Iterator<Item=(&str, EgressAd)>` that filters out
`is_stale` origins. A gateway that stops beaconing is **known gone** within
`stale_threshold_ms`, not guessed-gone.

meshd then:
- Keeps babeld as the **forwarding-plane installer AND the data-path authority**
  (don't reinvent nearest-exit metric math — babeld does it well). babeld learns,
  installs, and withdraws the actual default route; meshd never deletes one.
- **Exposes** the live-gateway set to the front desk / ops: written into
  `directory.json` (`gateways`) each anti-entropy tick and logged. This is the
  positively-determined presence/absence — "internet via N gateways", and a
  gateway that stops beaconing drops out on its own.
- Does **NOT** proactively flush routes on beacon-staleness (see the build-order
  note): gossip/meshd liveness ≠ babel data-path liveness, so a flush could cut
  working internet. Withdrawal stays babeld's job.

## Why the beacon plane and not an HLC / a CRDT book

Same rationale as `e21.9` (see `lane-staleness.md`): encoding gateway state in a
durable CRDT entry's HLC forces a flash write every anti-entropy tick (the `7bf`
churn). Gateway-ness is recency-of-contact, not a value to converge — it belongs
on the ephemeral, receiver-clock-judged, skew-immune beacon plane. `incarnation`
already handles restart with zero persisted state.

## Build order

No mixed-fleet dance — all 4 nodes upgrade together, so flip each piece for the
whole fleet at once.

1. Ship `bii` (stale-route flush) first — defence in depth for the
   crash/interrupted-restart window. **[done: commit ac2f397]**
2. `EgressAd` + local-egress detection (kernel non-babel default, oif exclusion,
   probe stub); extend `LivenessBeacon` with `egress: Option<EgressAd>` and
   `LivenessTracker` to stash/expose it. Emit-only, no behaviour change yet.
   **[done: commit be99c38 (7z5) — crdt::egress + tracker `live_gateways()`]**
3. Flip Lever 1: gate the `0.0.0.0/0` render on local egress. **This fixes the
   field bug** (a no-WAN node can no longer re-export a stale default).
   **[done: commit 0c5564e — `read_default_routes`/`local_egress` gate both
   reconcilers; beacon now advertises real egress. aarch64-musl cross-build
   verified.]**
4. Lever 2 — meshd consumes `live_gateways()` and **surfaces** it (NO route
   manipulation). **[done]** `write_directory_projection` reads
   `liveness.live_gateways(now)` each anti-entropy tick and writes the live set
   into `directory.json` (`gateways: [{node_id, cost_hint}]`, additive/hello-safe)
   plus a `debug!` line; the front desk can show "internet via N gateways".

   **The proactive-withdraw idea was deliberately DROPPED as unsafe** (decided
   2026-07-08). The liveness beacon rides meshd's gossip plane; the default route
   is owned by **babeld**, a separate process on a separate transport. They
   diverge: if meshd crashes or gossip partitions on a gateway while babeld keeps
   routing, the gateway stops beaconing (looks "stale") but internet still flows.
   Having meshd `ip route del` the default on beacon-staleness would then CUT
   WORKING INTERNET, and deleting a route babeld still believes in just makes
   babeld re-add it (flap). babel stays the data-path authority; meshd's
   liveness-gated role is advertise-side (Lever 1) + observability (this), never
   route deletion. Fast multi-gateway failover is already babel's job (metric +
   hop cost). If a positive-withdraw is ever wanted, it must be gated on the
   DATA path (babel's own route state), not the gossip beacon.
5. Fold in `42j` — reachability probe. **[done]** `gateway_probe_task` runs an
   HTTP-204 captive/connectivity check (busybox `wget -S`, several `generate_204`
   endpoints) every 30s and drives `GATEWAY_PROBE_HEALTHY` through a **fail-open**
   `ProbeHysteresis` (starts healthy; demotes only after 3 consecutive *confirmed*
   failures; a probe that can't run is "no evidence", never a demotion). `local_egress`
   reads it into `EgressAd.healthy`, and both render gates now require
   `local_egress().healthy` — so a dead/captive lease stops advertising `0.0.0.0/0`.
   `option gateway 'always'` sets `MJOLNIR_GATEWAY_NO_PROBE=1` to bypass the probe
   for a working-but-unreachable-to-204 uplink. Fail-open is the safety property:
   worst case is a dead uplink advertising slightly longer (today's behaviour),
   never a probe bug cutting a live gateway off the air.

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
