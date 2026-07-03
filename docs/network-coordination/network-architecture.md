# Network Architecture

**Status (2026-07-02):** the routed-/24 model described in "Why This Scales" below is
shipped and field-validated on a 4-router OpenWrt 802.11s fleet. Sections are annotated
where the original design diverged from what shipped.

## Overview

mjolnir-mesh creates a decentralized mesh network across OpenWrt routers. Each node owns
its **own routed client `/24`**, claimed from `10.42.0.0/16` via the subnet-claim CRDT
(gossip over iroh, HLC first-writer-wins). babeld routes between the /24s over the
802.11s radio backhaul (`br-mesh`), where each node has a derived
`10.254.<blake3(node_id)>/16` backhaul/management address. Routers at different physical
sites connect via the iroh QUIC overlay — a single TUN `mjolnir0` multiplexing all peers
— and route IP traffic between their local subnets.

The operational lesson from deployment: **local mesh traffic routes most efficiently
over the 802.11s L2 island; the iroh L3 overlay is for internet hops and as a first-hop
security gateway.**

```
Site A (802.11s island, babel-routed)      Site B (remote)
 ┌──────────────────────────┐               ┌──────────────────┐
 │  Router-1  Router-2  ... │               │     Router-5     │
 │  10.42.1/24  10.42.2/24  │◄──Iroh QUIC──►│  10.42.5.0/24    │
 │  (one routed /24 each)   │   (mjolnir0)  │                  │
 └──────────────────────────┘               └──────────────────┘
```

---

## Two Modes of Interconnection

### Mode 1: Flat Local Island — FUTURE / OPTIONAL, NOT SHIPPED

> **Status:** design for the North-Star roaming experience (same SSID everywhere,
> devices roam seamlessly, `.mesh` services discoverable locally). It is **not** the
> shipped default — every node runs its own routed `/24` even when co-located, and the
> CRDT hostsfile that would make a shared subnet safe is not implemented (service-mesh
> phase, bead `e21`). See "Why This Scales" for why the routed model is the default.

If a flat island mode is enabled in the future, co-located routers would share a single
subnet:

- Routers detect each other via mDNS (`_mjolnir-mesh._tcp.local`) or Iroh connection latency
  below the local threshold (~5ms round-trip)
- All local routers share a single subnet (e.g., `10.42.1.0/24`)
- Each router runs dnsmasq covering the full shared range; a CRDT hostsfile prevents IP
  conflicts by distributing MAC-to-IP bindings across all nodes
- Devices see one flat broadcast domain and can reach any device on any AP without routing
- mDNS, Bonjour, and AirPlay work natively because they remain on the same broadcast segment
- Roaming between APs is seamless: the device keeps the same IP, and dnsmasq on the new AP
  already has the MAC reservation from the CRDT

### Mode 2: Remote (Via Iroh) — SHIPPED (data plane)

When routers are at different physical locations, they connect through the Iroh QUIC
overlay.

- Each site has its own subnet(s) — every node claims one (e.g. `10.42.1.0/24`)
- Iroh provides NAT traversal, encryption, and relay fallback — no port forwarding required
- The mjolnir-mesh daemon manages a single overlay TUN (`mjolnir0`) on each router and
  encapsulates IP packets into Iroh QUIC datagrams for delivery to the remote peer
- Mesh-wide DNS sync and cross-site mDNS reflection are **planned** (bead `e21`), not shipped

---

## Cross-Site Routing

> **SUPERSEDED (2026-07):** the per-peer TUN model below (`mj-peer-*` /31s in
> `10.255.0.0/16`, ipset/iptables forwarding filters) was replaced by what shipped:
> **babeld peers directly over the 802.11s L2 backhaul** (`br-mesh`, rendered as
> `type wireless` with RTT metrics), and cross-site iroh traffic rides a **single
> overlay TUN `mjolnir0`** that multiplexes all peers (bead `buw`). The per-peer
> tunnel code still exists but is default-off legacy. Kept below for the record.

### Packet flow (superseded per-peer model)

```
Alice (10.42.1.50, Router-1 at Site A) → Bob's server (10.42.2.30, Router-5 at Site B)

1. Alice sends to 10.42.2.30 (or bob-server.mesh resolved via DNS)
2. Router-1 kernel: 10.42.2.0/24 is in babeld-installed route table: dev mj-peer-<router5_id>
3. Daemon reads packet from mj-peer-<router5_id> → encapsulates → sends via Iroh to Router-5
4. Router-5 daemon: decapsulates → writes to its mj-peer-<router1_id> → kernel delivers to 10.42.2.30
5. Return traffic follows the same path in reverse
```

In the shipped model the packet flow is the same shape, but step 2 resolves to either
a next hop on `br-mesh` (same-island, pure L2 forwarding under babel) or to `mjolnir0`
(cross-site), and the daemon dispatches per-destination inside the single overlay TUN.

What survives from the original design: forwarding decisions are made by the kernel
from babeld-installed routes — the daemon does not maintain its own forwarding table.

See [babel-routing.md](babel-routing.md) for the full Babel integration spec, including babeld config, failure modes, and the rationale for delegating routing to a battle-tested protocol.

---

## Subnet Claim Coordination (CRDT)

The CRDT no longer holds a routing table. It holds a **subnet ownership ledger** used only to prevent two routers from claiming the same /24 at first boot:

```
/subnets/10.42.1.0_24  → { owner_node_id: "router1_nodeid", site_name: "site-a", claimed_at: <hlc> }
/subnets/10.42.2.0_24  → { owner_node_id: "router5_nodeid", site_name: "site-b", claimed_at: <hlc> }
```

When a router claims a subnet, it writes one entry and reconfigures babeld to redistribute that subnet. Babel handles announcement, propagation, and route installation to all peers. The CRDT is *not* consulted for forwarding decisions.

Conflicts on `/subnets/` (two routers claim the same /24) resolve via HLC first-writer-wins, same rule as IP-lease conflicts. The loser picks the next free /24 and rewrites its claim.

When a router goes offline:
- Babel marks its routes unreachable within its hello interval and withdraws them
  (loss of hello/IHU on `br-mesh`, or iroh disconnect for overlay peers)
- The `/subnets/` entry persists (the subnet is still *claimed*, just not reachable). On the owner's reboot, Babel re-announces; on a graceful permanent departure, the daemon tombstones the entry.

No heartbeat gossip, no route-TTL refresh, no daemon-side stale-route reaping. Babel handles all of that.

---

## Subnet Allocation for Remote Sites

When a router determines it is starting a new isolated site (no local peers detected within the
detection window), it claims a subnet from the mesh address space. The operator picks the subnet
**size**; the allocator picks the **slot**. Larger requests (smaller prefix lengths) are for
larger sites — a /24 fits 254 devices, /22 fits ~1 000, /20 fits ~4 000, /16 fits ~65 000.

The size is configurable per router. The expected UX is a TUI selector where arrow keys step
the prefix one bump at a time (`/24 ↔ /23 ↔ /22 ↔ …`), with a label showing the resulting IP
count. Backed by `mjolnir_mesh::alloc::{pick_subnet, bump_larger_subnet, bump_smaller_subnet,
usable_hosts}`.

1. Read the CRDT `/subnets/` prefix to enumerate already-claimed subnets (at any size).
2. Operator chooses a target prefix length (default /24 if unconfigured).
3. Compute the preferred slot from a deterministic hash of the router's Iroh NodeId. Hash bytes
   modulo `2^(target_prefix - base_prefix)` index into the candidate slots at the chosen size.
4. Walk slots from the preferred index. Reject any candidate that **overlaps** an existing claim
   of any size — a candidate /22 is rejected if it contains a claimed /24, and a candidate /24 is
   rejected if a containing /22 is already claimed. (CIDR blocks form a tree: overlap reduces to
   `a.contains(b) || b.contains(a)`.)
5. Write to CRDT: `/subnets/{cidr} → { owner_node_id, site_name, claimed_at }`.
6. Reconcile OpenWrt UCI (`network.lan.ipaddr`) to the claimed /24 and restart dnsmasq
   via init.d — stock dnsmasq then serves the right pool. (The daemon never edits
   dnsmasq files directly, and never SIGHUPs it; see `reconcile_client_uci`.)
7. Add a `redistribute ip {cidr} allow` line to the rendered babeld config; babeld is
   cleanly **restarted** (procd watches the config file — babeld 1.13 dies on SIGHUP,
   bead `2zz`).

The two-phase approach (derive then check) is optimistic: hash-based derivation makes collisions
rare, and the CRDT resolves the uncommon case where two routers happen to prefer the same slot.

### Claim Cooldown

A router that determines it needs a new subnet waits before writing the claim to the
CRDT (`claim_cooldown.rs`), giving gossip from existing claimants time to arrive so the
node doesn't claim a /24 that is already taken. (The original rationale — abandoning
the claim to join a shared subnet in Mode 1 — is future/flat-island only.)

If two routers at different sites claim overlapping subnets (rare — hash derivation across the
slot count at the chosen prefix makes collision probability `~1/N` where N is the slot count):
- FWW resolves: lower HLC wins the `/subnets/` entry
- The loser has zero or very few devices (it just started) — it picks the next free slot at its
  configured prefix length, rewrites its claim, and updates babeld redistribute config
- Clients on the losing range simply re-DHCP after the UCI reconcile moves the pool
  (active deauth was designed but never built)
- If no free slot exists at the requested prefix length, the daemon automatically
  retries at smaller sizes (`alloc::pick_subnet_or_smaller`) before failing loud —
  see `collective-coordination-protocol.md`

### Late Local Peer Discovery — FUTURE (flat-island mode only)

In the shipped model a late-discovered co-located peer is a non-event: each node keeps
its own /24 and babel routes between them. Only in a future flat-island mode would a
router relinquish its claim (tombstone `/subnets/{cidr}`), drop its `redistribute` line
(babeld restart), and re-point dnsmasq at the shared subnet.

---

## Local Peer Detection — DESIGN ONLY

> **Status:** not shipped. There is no latency probe, UDP broadcast probe, or automatic
> mode selection — there is no mode to select, since every node runs the routed-/24
> model. Shipped discovery is **derived-address seeding + gossip**: a peer's backhaul
> address is computable from its node id (`10.254.<blake3(node_id)>`), and the gossip
> address book (bead `0yb`) carries the rest. mDNS is link-local bootstrap only.

The original design, kept for the flat-island future: a router entering a venue
determines whether it is joining an existing local cluster or starting a new remote
site, using multiple signals in parallel:

| Method | Signal | Reliability |
|---|---|---|
| Iroh connection latency | Round-trip < 5ms → same LAN | High for wired, variable for WiFi |
| mDNS | `_mjolnir-mesh._tcp.local` announcement | Requires mDNS-capable network |
| UDP broadcast probe | Send to 255.255.255.255 on mesh port, wait for reply | Works on flat L2 |
| Manual config | `--local-peers=nodeA,nodeB` | Authoritative override |

Detection uses a 10-second window after startup. If any signal identifies a local peer, Mode 1
applies. If no local peers are found within the window, the router proceeds as a new remote site.

When a local peer is confirmed:
- The router does not claim a new subnet
- It bridges into the peer's existing L2 segment
- dnsmasq is configured with the peer's subnet range; the CRDT hostsfile ensures no IP conflicts
- The router announces itself to the local cluster via the shared CRDT

---

## SSID Guidance

In the shipped model each node's client AP fronts its own routed `/24`; client L2
segments are **never bridged across nodes** (see "Why This Scales"). Nodes may share an
SSID — clients then re-associate to the nearest AP and re-DHCP onto that node's /24
(an L3 roam; sessions break). The North-Star seamless-roaming experience (same SSID,
same IP across APs) belongs to the future flat-island mode / `/32` host-route work
described under Roaming below.

---

## Address Space

The default mesh address space is `10.42.0.0/16`, providing:
- 65,534 usable host addresses
- Up to 256 independent /24 subnets (one per remote site)
- 254 devices per /24 at a single site

For deployments that expect more devices at a single venue, the site subnet can be widened:
- `/23` — 510 devices
- `/20` — 4,094 devices
- `/16` — 65,534 devices (useful for large events sharing one physical network)

For federations requiring more than 256 sites, the mesh address space can be expanded to
`10.0.0.0/8` at deployment time, supporting up to 65,536 /24 subnets across 16 million addresses.
Address space configuration is set in the CRDT root document and read by all nodes on join.

---

## Why This Scales: No Shared L2 Broadcast Domain

The design deliberately keeps each router's clients in their **own routed `/24`** and routes
between them at L3 with Babel — rather than bridging every node into one large L2 cloud. This
is the single most important scaling decision, and it is what separates this design from
flat-mesh systems (e.g. `batman-adv`-based meshes such as the default LibreMesh cloud), which
put a whole area into one L2 broadcast domain.

Three properties fall out of it:

- **Broadcast containment.** ARP, DHCP-discover, and mDNS multicast stay inside a single
  node's `/24` — never flooded mesh-wide. Flat-L2 meshes must flood broadcast across the whole
  cloud; that traffic (and the storm risk) grows with the mesh and is the classic ceiling those
  designs hit. Here it is bounded per node, independent of mesh size.
- **Route aggregation.** Babel carries **one `/24` per node** — routing state is `O(nodes)`,
  not `O(clients)`. A node advertises its subnet, not its individual devices, so convergence
  stays cheap as the client count grows.
- **No L2 loop surface, and provable L3 loop-freedom.** There is no spanning L2 segment for a
  frame to loop in — `br-mesh` carries only the mesh backhaul and is never bridged into the
  client domains. And Babel is loop-free *by construction* (its feasibility condition forbids
  installing a route that could form a loop, and avoids transient loops during reconvergence —
  no count-to-infinity; see `babel-routing.md`). IP TTL is the final backstop for any transient.
  So the two failure modes that make large flat meshes fragile — broadcast storms and forwarding
  loops — are structurally absent, not merely mitigated.

The discipline that preserves this: **never bridge a client segment across nodes.** Keep each
`/24` node-local and routed. Bridging client L2 across routers to "extend the LAN" hands back
exactly the mesh-wide broadcast domain and loop surface the design removes.

This is the same axis as the routing-metric and roaming choices: pushing the L3 boundary out to
each node buys broadcast containment, loop safety, and quality-aware routing; the price is that
seamless roaming becomes an L3 problem (see below) rather than a free L2 property.

---

## Roaming Across Sites

What ships today: a device moving between nodes re-associates and re-DHCPs onto the new
node's `/24` — TCP sessions break, new sessions work immediately. No lease/DNS CRDT is
consulted (that lane is the `e21` service-mesh phase). The design below is the roadmap:

```
1. Device disconnects from Site A's WiFi (Router-1)
2. Device connects to Site B's WiFi (Router-5)
3. [Future] Router-5's dnsmasq checks the CRDT hostsfile
4a. Device's MAC has a binding at 10.42.1.x — offer a new IP from Site B's range (10.42.2.x)
    DNS entry updated via CRDT. TCP sessions break; new sessions work immediately.
4b. Offer the same IP (10.42.1.50) — Router-5 redistributes a /32 host route
    via babeld, Router-1 withdraws its /32 advertisement. TCP sessions survive.
```

Option (b) enables seamless cross-site roaming by letting Babel carry per-device /32
routes — natively supported by babeld but deferred for operational simplicity.

### Fast roaming (802.11r) and the L2/L3 split

802.11r (Fast BSS Transition) is **orthogonal** to the subnet design: it speeds up the *radio
re-association* — pre-computing the keys so a client re-associates to the next AP in
milliseconds instead of running a full 4-way / EAP handshake — which is what makes handoff fast
enough for latency-sensitive traffic like VoIP. It works on both bands and with SAE (FT-SAE) and
WPA2-PSK (FT-PSK). So **yes, 802.11r still works** — but *what* it buys depends on the mode:

- **Flat-L2 island (future mode: one SSID, one subnet, `mesh_fwding=1`):** 802.11r gives
  **fully seamless roaming** — a fast L2 handoff *and* the client keeps its IP, because every AP
  shares the subnet. Nothing else is needed. This is the right mode for dense client roaming
  (events), and the reason a flat island remains attractive for that use case.
- **L3-per-hop / cross-site (per-node `/24`, `mesh_fwding=0`, or separate sites):** 802.11r
  still makes the *radio* handoff fast, but it is **necessary, not sufficient** — the client
  lands on a different subnet, so its IP would change and active sessions break unless the IP is
  made portable. That is the job of the `/32`-host-route-over-Babel + CRDT-lease mechanism
  (Option b above): the new node already holds the client's `MAC→IP` reservation from the
  gossiped lease, offers the *same* IP, and advertises a `/32` so traffic follows the device.

So **802.11r and the `/32`/CRDT roaming are complementary layers, not alternatives**: 802.11r for
a fast radio handoff, the `/32` route + CRDT lease for IP continuity. In the flat-island default,
802.11r alone is enough; in the L3 mode it remains available and useful, paired with per-device
`/32` roaming to preserve the address.

---

## Security

Traffic between sites is secured at the transport layer by Iroh:

- All Iroh connections use QUIC with TLS 1.3 — encryption is mandatory and cannot be disabled
- Router identity is bound to the Iroh NodeId (Ed25519 keypair); membership enforcement is
  planned, not yet implemented (see Future Work below)
- IP forwarding on each router is restricted to known mesh subnets via iptables rules — arbitrary
  external traffic cannot be injected through the tunnel
- No open relay: the Iroh relay servers are used only for NAT traversal handshake, not for
  sustained packet forwarding between routers

### End-to-end vs per-hop confidentiality

Where Iroh carries the **data plane** (cross-site tunnels today; optionally same-site too — see
the single-overlay-TUN work, `buw`), confidentiality is **end-to-end between the two router
daemons**: every packet rides inside a QUIC / TLS 1.3 connection, so no intermediate node — not
even another mesh router relaying the datagrams — can read it.

Contrast the radio backhaul. 802.11s with SAE encrypts each **radio hop** independently: a
multi-hop frame is decrypted and re-encrypted at every forwarding node, so an intermediate mesh
router *does* see plaintext. That is fine for a single trusted hop, but it is **not** end-to-end.

So: cross-site traffic over the Iroh overlay is confidential even from the routers relaying it;
native 802.11s same-site traffic is only hop-by-hop confidential. Routing same-site over the
overlay as well (the `buw` "U1" option) extends end-to-end confidentiality to every hop, at the
cost of QUIC encapsulation on the local radio link. This property only holds where Iroh is the
data plane — it is a reason to *prefer* the overlay where confidentiality from intermediate
nodes matters.

### Future Work: Membership Control

**Current gap:** Any Iroh node that knows the gossip topic can join the mesh and inject data. There is no membership enforcement.

**Phase 1 (MVP):** Pre-shared key (PSK) configured on each router. The gossip topic is derived from `blake3(b"mjolnir/mesh/" || psk)` instead of a static name, preventing unauthorized joining. Simple but key rotation requires touching every router.

**Phase 2:** Membership CRDT (`/members/{node_id}`) with signed enrollment invitations. Any existing member can invite a new node by signing its public key. Peers validate membership before accepting gossip messages. Revocation via CRDT tombstone.

---

## Relationship to Iroh and IPv4

The two layers have complementary roles:

**Iroh provides:**
- Encrypted QUIC transport between routers (the tunnel substrate)
- Global router identity via NodeId
- Peer discovery (n0 discovery service, gossip-based mesh discovery)
- NAT traversal (hole-punching with relay fallback)
- CRDT replication via gossip

**IPv4 provides:**
- Standard LAN connectivity for consumer devices and applications
- Routing abstraction: cross-site IP reachability without application changes
- DNS: familiar hostname resolution for devices on the mesh

Neither layer is aware of the other's internals. IPv4 packets are opaque payloads from Iroh's
perspective; the mjolnir-mesh daemon is the bridge that reads from the TUN device and writes to
the Iroh stream (and vice versa). This clean separation means Iroh can be upgraded or replaced
without touching the routing logic, and the IPv4 topology can be reconfigured independently of
the underlying transport.

---

## References

- Gossip/CRDT primer (current): `gossip-and-crdt.md`
- Babel routing integration: `babel-routing.md`
- Archived original designs (lease CRDT, dnsmasq file management, shared-L2 overview):
  `../archive/network-coordination/dhcp-crdt.md`,
  `../archive/network-coordination/dnsmasq-integration.md`,
  `../archive/network-coordination/mesh-network-coordination.md`