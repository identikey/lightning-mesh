# Babel Routing Integration

**Status:** Architecture decision, shipped and field-validated | **Date:** 2026-05-18, annotated 2026-07-02

> **What changed since this was written (2026-07-02):** babeld peers directly over the
> **802.11s L2 backhaul** (`br-mesh`, rendered `type wireless` with RTT/timestamp
> metrics — `crates/mjolnir-mesh/src/babel/config.rs`), not only over tunnels. The
> per-peer TUN model (§4, §6) is **superseded** by a single overlay TUN `mjolnir0`
> (bead `buw`), and babeld is supervised under procd and cleanly **restarted** on
> config change — babeld 1.13 dies on SIGHUP (bead `2zz`). Internet-gateway
> default-route redistribution (`0.0.0.0/0`) shipped as well.

mjolnir-mesh delegates cross-site IP route computation to **Babel** (RFC 8966, `babeld`). The CRDT layer is reduced to *subnet ownership coordination* — claim collision avoidance and announcement — and stops trying to be a routing table.

This document explains the split, the integration mechanics, and what the daemon still owns.

---

## 1. Why Babel

Earlier drafts of this design used a `/routes/{subnet}` CRDT namespace with a `via_node_id` field and timestamped expiry, hand-rolled in the daemon. That approach was reinventing distance-vector routing without metrics, without loop avoidance, and without the failure modes (count-to-infinity, convergence races) that twenty years of routing-protocol literature has already solved.

Babel:

- Is the routing protocol the OpenWrt mesh community (CeroWrt, Homenet, Freifunk) standardized on after `batman-adv` and `olsr` proved too L2-coupled or too convergence-prone respectively.
- Is loop-free in a single pass (feasibility condition; no transient loops during reconvergence).
- Reconverges in seconds on link failure without count-to-infinity.
- Ships in OpenWrt as `babeld` (~250 KB, written in C, ~5K LoC).
- Supports IPv4-only, IPv6-only, and dual-stack carriage.
- Has explicit support for non-multicast interfaces (point-to-point tunnels) via `neighbour` directives.

Babel routes **everywhere**: between per-node client /24s on the same 802.11s island
(directly over the L2 backhaul, where it routes most efficiently) and across sites over
the iroh overlay. The original "cross-site only" framing assumed a shared-L2 Mode 1
that was never shipped — see `network-architecture.md`.

---

## 2. Lane Split

| Concern | Owner |
|---|---|
| Subnet claim coordination ("Site A owns 10.42.1.0/24") | mjolnir-mesh CRDT (`/subnets/{cidr}`) |
| Subnet announcement to remote peers | babeld (redistributes claimed subnet) |
| Next-hop computation | babeld |
| Linux route install/withdraw | babeld via netlink |
| Peer link liveness (cross-site) | iroh connection state + Babel hello/IHU |
| DHCP, DNS, service discovery | mjolnir-mesh CRDT (unchanged) |

The CRDT no longer stores `via_node_id`, `expires`, or any forwarding information. Subnet entries are pure claim ledger.

---

## 3. CRDT Schema Change

**Old (removed):**

```
/routes/{subnet}  →  { node_id, via_node_id, site, expires }
```

**New:**

```
/subnets/{cidr}  →  { owner_node_id, site_name, claimed_at, hlc }
```

`{cidr}` uses the form `10.42.1.0_24` (slash escaped to underscore for key safety). Only one entry per subnet; conflicts on claim resolve by HLC (first-writer-wins, same rule as IP leases). No expiry field — claims persist as long as the owner participates in gossip; tombstone on graceful release. Crash-recovery uses the owner's gossip presence and last-seen HLC, not a TTL.

See [gossip-and-crdt.md](gossip-and-crdt.md) for the current data model (the original merged model is archived at [dhcp-crdt.md §2](../archive/network-coordination/dhcp-crdt.md#2-data-model)).

---

## 4. Tunnel Interface Model — SUPERSEDED

> **Superseded (2026-07):** shipped is a **single overlay TUN `mjolnir0`** that
> multiplexes all iroh peers, with the daemon dispatching per destination (bead `buw`).
> The per-peer `mj-peer-*` /31 tunnels below still exist in the code but are
> default-off legacy. Same-site routing doesn't ride tunnels at all — babeld peers
> directly on `br-mesh`.

The original per-peer design: each Iroh QUIC connection to a remote peer surfaces as a numbered point-to-point TUN interface managed by the daemon:

```
mj-peer-<short_node_id>  type tun
  local  10.255.<a>.<b>/31   # /31 point-to-point link addressing
  remote 10.255.<a>.<c>/31   # peer's end
```

The `10.255.0.0/16` link space is reserved for inter-router tunnel addressing — it is **not** assigned to any device subnet and is never advertised into the mesh.

**Lifecycle:**

| Event | Daemon action |
|---|---|
| Iroh connect to peer | Create `mj-peer-<id>` TUN, assign /31 addrs, hand interface name to babeld |
| Iroh disconnect | Tear down `mj-peer-<id>`; babeld notices link death within hello interval |

Each per-peer interface is independent. babeld treats them as ordinary point-to-point routing links.

> **RESOLVED:** single shared overlay TUN (`mjolnir0`) with daemon-side per-peer
> dispatch won (bead `buw`). The multiplexing turned out tractable, and one interface
> is what shipped; per-peer TUNs are legacy, default-off.

---

## 5. babeld Configuration

Per-router config, rendered by the daemon (`crates/mjolnir-mesh/src/babel/config.rs`)
and supervised by procd, which watches the file and restarts babeld on change. What
ships (shape, not verbatim):

```
# L2 backhaul: babeld speaks directly on the 802.11s bridge, with RTT-based
# metric so a congested/multi-hop radio path costs more than a clean one
interface br-mesh type wireless enable-timestamps true \
    rtt-min 10 rtt-max 120 max-rtt-penalty 150 rtt-decay 42

# Overlay TUN (cross-site iroh traffic), same RTT machinery, tunnel-tuned
interface mjolnir0 type wireless enable-timestamps true ...

redistribute ip 10.42.1.0/24 allow      # this router's claimed subnet
redistribute ip 0.0.0.0/0 le 0 metric 128   # only on internet-gateway nodes
redistribute local deny
redistribute deny

# never advertise the transport blocks (10.254/16 backhaul, 10.255/16 legacy links)
```

Notes vs. the original draft: interfaces are `type wireless` with babeld's
timestamp/RTT metric (validated live — `rttcost` tracks measured `rtt`), not `type
tunnel`; nodes with WAN uplink redistribute a default route so the mesh picks the
best internet gateway; and `redistribute proto static/kernel deny` lines were dropped —
real babeld 1.13 rejects them, the bare `redistribute deny` is the idiomatic form.

The daemon **reads** `/subnets/{cidr}` for *its own* claim and writes the matching `redistribute ip ... allow` line. Remote subnets are not redistributed by this router — they will be advertised by their owner over Babel.

---

## 6. Cross-Site Packet Flow (Revised)

> **Superseded detail:** the per-peer `mj-peer-*` interfaces in this walk-through are
> now the single `mjolnir0` overlay TUN (§4); same-island traffic never enters a TUN —
> it is babel-routed straight over `br-mesh`. The step-3 point (kernel routing table,
> not a CRDT) is unchanged and is the part that matters.

```
Alice (10.42.1.50, Site A, Router-1) → wiki.mesh (10.42.2.30, Site B, Router-5)

1. Alice resolves wiki.mesh → 10.42.2.30 via local dnsmasq (CRDT-synced DNS)
2. Alice sends to 10.42.2.30
3. Router-1 kernel: 10.42.2.0/24 is in Babel's installed route table → mj-peer-eeff0011
4. Daemon reads packet from mj-peer-eeff0011 → wraps in an Iroh QUIC datagram (one IP packet per datagram, unreliable, no retransmit — TCP/app layer handles loss) → Router-5
5. Router-5 daemon: receives datagram → writes payload to its mj-peer-aabbccdd → kernel delivers to 10.42.2.30
6. Return path symmetric, Babel having advertised 10.42.1.0/24 from Router-1
```

What changed: step 3 used to consult a hand-rolled "route table CRDT." Now it consults the kernel's regular routing table, which babeld populates.

---

## 7. Failure Modes Babel Now Handles

| Failure | Old design | New design |
|---|---|---|
| Remote router dies cleanly | Iroh disconnect → daemon walks `/routes/` and removes entries | Iroh disconnect → daemon tears down `mj-peer-<id>` → babeld marks route unreachable within hello interval, withdraws |
| Remote router dies silently | Heartbeat gossip timeout (90s) → manual route removal | Babel IHU timeout (default 16s) → automatic withdrawal |
| Link flaps | Routes thrash via TTL expiry | Babel hysteresis + feasibility condition damps thrash |
| Path becomes available via better neighbor | No mechanism — single via_node_id per subnet | Babel reconverges on shortest feasible path |
| Two paths to same subnet (multi-homed site) | Not supported — single entry per subnet | Babel handles equal-cost or metric-preferred selection |

The 90-second heartbeat gossip and explicit route TTL machinery from the old design are **deleted**, not migrated. Babel's hello/IHU timers replace them.

---

## 8. What the Daemon Still Owns

Routing is delegated to Babel. Subnet **ownership** is not:

- **Claim arbitration** when two routers boot simultaneously and both prefer the same /24. CRDT FWW on `/subnets/{cidr}` decides; the loser picks the next free /24 (same logic as before, just on the renamed namespace).
- **Subnet claim broadcast** on first-boot so other routers see the claim before they pick the same range. This is gossip, not Babel — Babel cares about reachability, not address-space ownership.
- **Tunnel interface lifecycle** (create/destroy TUN on Iroh connect/disconnect).
- **babeld config regeneration** when the local subnet claim changes (rare — typically once at first-boot per site).
- **DHCP/DNS/service-discovery CRDT** — completely unchanged.

If the daemon dies, babeld continues forwarding existing routes correctly. New tunnel interfaces won't appear until the daemon restarts, but in-flight traffic survives.

---

## 9. IPv4-Only Posture

mjolnir-mesh runs Babel in IPv4-only mode for now:

```
# babeld.conf
random-id true
```

(no `-6` flag; advertises only `ip` redistribute lines, not `ipv6`)

Babel was originally an IPv6-leaning protocol but RFC 8966 codifies dual-stack and IPv4-only operation. Single-stack IPv4 is fully supported and is what we ship. (Whether to move mesh addressing to IPv6 — the /24-claim model hands out a limited resource — is an open decision, bead `bsa`.)

When IPv6 is added later, Babel gains carriage for it without a protocol swap — same daemon, same config, additional `redistribute ipv6` lines.

---

## 10. Dependencies and Packaging

- **babeld** — OpenWrt package (`opkg install babeld`). ~250 KB installed. No Rust binding required; the daemon renders the config and procd supervises babeld, cleanly **restarting** it when the config file changes. (Never SIGHUP: babeld 1.13 dies on SIGHUP — bead `2zz`.)
- **No new Rust crates.** The integration is config-file generation + process supervision.

---

## 11. Open Questions

1. ~~**TUN-per-peer vs shared TUN** (§4).~~ **RESOLVED:** single shared overlay TUN (`mjolnir0`) shipped; per-peer tunnels are legacy, default-off.
2. **Babel security**: Babel HMAC (RFC 8967) authenticates messages between routers. Iroh's QUIC layer already authenticates and encrypts the tunnel; running Babel HMAC on top is defense-in-depth but adds key distribution. Defer to a security pass.
3. **Babel metric tuning for tunneled vs direct links**: tunnel rxcost should be higher than direct LAN rxcost so direct paths are preferred when both exist. Default `96` for tunnels is a starting point; needs measurement.
4. **Babel + roaming /32 host routes** (the "seamless cross-site roaming" milestone deferred in [network-architecture.md](network-architecture.md)): Babel handles host routes natively, so this milestone becomes "have the daemon write a /32 redistribute line when a device roams across sites." Easier than the old plan.

---

## 12. References

- **Babel protocol**: RFC 8966 (replaces RFC 6126).
- **Babel HMAC**: RFC 8967.
- **babeld**: <https://www.irif.fr/~jch/software/babel/> — Juliusz Chroboczek's reference implementation.
- **OpenWrt package**: <https://openwrt.org/packages/pkgdata/babeld>
- **Related docs**:
  - `gossip-and-crdt.md` — CRDT data model and gossip layer (current)
  - `network-architecture.md` — cross-site packet flow and subnet allocation
  - `../archive/network-coordination/dhcp-crdt.md` — original lease-CRDT design (archived)
