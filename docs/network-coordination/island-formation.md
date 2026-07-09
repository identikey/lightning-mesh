# Auto-Island Formation

**Status:** Design pass (2026-07-06) | Relates to epic `e21` (service-mesh / north-star roaming)

## Why this exists

The shipped model runs one routed `/24` per node even when nodes are co-located. That is
correct at *camp* scale (broadcast containment, O(nodes) routing state) but wrong at *room*
scale: a client roaming between two co-located APs changes subnet → DHCP re-lease → every
connection drops. 802.11r fixes reassociation *latency*, not *IP continuity*; IP continuity
requires the APs to share one L2 broadcast domain.

An **island** is the fix: *a set of nodes that agree to share one client subnet and one
client L2 broadcast domain, so clients roam without changing IP.* babel routes between
islands; iroh stitches sites. This is the mechanism behind `e21`'s north-star ("same SSID
everywhere, devices roam seamlessly"), generalized from the `network-architecture.md`
"Mode 1: Flat Local Island" sketch to be **auto-formed, multi-node, and identity-scoped**.

This is a boundary move, not a teardown: every invariant survives with one word changed —
"each **node** owns a `/24`" → "each **island** owns a `/24`"; "never bridge L2 across
**nodes**" → "never bridge L2 across **islands**". The reasons (containment, O(segments)
state) hold because an island is still a bounded domain.

## Model

- **Backhaul `mesh_id` is global** — peering *semantics*: any two nodes that can hear each
  other on a channel auto-peer, no config. This is what makes merge-on-contact work.
- **Channel is NOT global.** Single-radio 802.11s means one mesh = one channel; a single
  global channel would recreate the flat-broadcast wall at the RF layer. Channel physically
  partitions the backhaul into **RF-reachability islands** (co-channel + in-range). babel
  already stitches across them. RF-island ≈ roaming island (clients only roam between APs in
  RF range of each other).
- **Control plane (CRDT) scopes; data plane forwards.** The CRDT decides membership; the
  data plane (below) does the fast forwarding. This is the [design-principles](design-principles.md)
  §2 boundary — coordination configures the data plane, it does not become it.

## Locked decisions (2026-07-06)

1. **Global backhaul `mesh_id`; per-island channel** (not one global channel).
2. **Client space = `10.0.0.0/8`** (allocator already supports it), **default island
   subnet = `/24`** (~254 hosts). Rationale: because an island grows by *accreting another
   prefix onto the same VNI* (decision #3), the initial size need not fit the largest
   possible room — so we optimize for the common case (most islands are small: a lone node
   or a 2–3 AP room) and for block count. A `/8` holds 65 536 `/24`s vs 16 384 `/22`s — 4×
   more islands, far less waste per small island. And a **1-node island is exactly today's
   shipped `/24`-per-node** model, so the island design is a strict generalization of the
   field-validated data plane, not a departure. Downgrade to smaller than `/24` only under
   extreme fragmentation (`alloc.rs` `pick_subnet_or_smaller`).
3. **Growth = accrete a second, discontiguous prefix onto the same VNI — never renumber.**
   Renumbering breaks the "IP survives" invariant we built islands to protect. The new
   prefix is a **secondary subnet on the island's existing L2/VNI**, so growth is
   transparent to roaming (existing clients keep their IP; new clients draw from the newer
   prefix) and does **not** change broadcast containment — the containment boundary is the
   VNI/island, not the subnet. dnsmasq serves multiple ranges; babel advertises both. We do
   **not** guide fleets to tune subnet-size knobs; they claim a `/24`, and grow by adding a
   prefix.
4. **Island membership is an *authorized set*, not a physical RF fact** — see data plane
   and "Why not batman" below. (Leans hard on the identity pass;
   [identity-peering-requirements](identity-peering-requirements.md) R2c.)
5. **Data plane = VXLAN / EVPN-lite over babel, with CRDT-driven membership** (option (c)
   below).
6. **Backhaul security:** shared-secret backhaul is trusted-only; **open backhaul is gated
   on authenticated babel (`661`) + identity-authorized CRDT writes** — see
   [identity-peering-requirements](identity-peering-requirements.md) "Gating open backhaul".
7. **Ad-hoc plug-in uses a signed capability beacon** for compatibility/trust; incompatible
   or untrusted peers degrade to **L3-gateway + NAT** (same path as foreign-mesh interop).
8. **Channel is a coordination job:** per-island selection + inter-island graph-coloring via
   the CRDT; changes are **scheduled cutovers with health-gated rollback** (the mjolnir-apply
   pattern applied to RF) plus **scan-and-rejoin** for stranded/booting nodes; DFS radar
   forces a move to a pre-agreed backup channel.

## Data plane: why (c) VXLAN / EVPN-lite

The requirement: island members share one client broadcast domain (roaming), but that L2
must **not** extend to non-members (containment + security). Over a *shared* backhaul, that
means scoping an L2 to a subset of reachable nodes. Three candidates were considered:

- **(a) RF-island-native bridging** — bridge the client VLAN over the local 11s segment;
  containment = the RF-island boundary. Simplest, no encapsulation. **Fails** the moment a
  global `mesh_id` puts a stranger's node in RF range on the same channel: bridging leaks
  client L2 to it. Works only if RF membership == trusted membership, which needs VLAN/filter
  anyway.
- **(b) batman-adv scoped to members** — see "Why not batman" below.
- **(c) VXLAN / EVPN-lite over babel** — **chosen.**

### What (c) means (for readers new to VXLAN/EVPN)

- A **VNI** (VXLAN Network Identifier) names the island's virtual L2. Each member node is a
  **VTEP** (VXLAN Tunnel EndPoint).
- A client Ethernet frame is **encapsulated** in UDP/IP and **routed by babel** over the
  backhaul to the *member* VTEPs — so it rides the L3 underlay we already have, and never
  bridges raw client L2 over the air.
- **BUM traffic** (Broadcast, Unknown-unicast, Multicast — ARP, DHCP, ND, mDNS) is
  **head-end replicated** to the member set (send one copy per member VTEP).
- **Membership is an explicit CRDT list** (VNI + authorized member node ids) — clean,
  identity-gated, and exactly the "authorized set" of decision #4.
- **EVPN-lite** = we already hold the MAC→IP bindings in the CRDT hostsfile, so we can do
  **ARP/ND suppression**: a member answers ARP locally from the CRDT instead of flooding
  BUM. This cuts the dominant broadcast cost.
- **One VNI can carry several IP subnets.** Prefix accretion (decision #3) adds secondary
  subnets to the *same* VNI; ARP/ND/BUM span the VNI regardless of subnet, which is why
  subnet size is an allocation-granularity choice, not a containment choice.

### Edges to watch (the unfamiliar parts)

- **MTU:** encapsulation adds ~50 bytes; set client-facing MTU or clamp so encap frames fit
  the backhaul MTU (avoid fragmentation).
- **BUM replication cost:** O(members) per broadcast — negligible at room scale (single-digit
  members), and ARP/ND suppression removes most of it.
- **Membership churn:** VTEP set changes on node join/leave and on island re-formation; the
  CRDT is the source of truth, so churn is a gossip update, not a reconfiguration.
- **Reputation dependency:** revoking a compromised member's VNI access needs the
  reputation/revocation hook ([identity-peering-requirements](identity-peering-requirements.md)
  R5). Until it lands, membership authorization is **manual** — an accepted limitation.

### Relationship to the CRDT hostsfile (Mode-1 sketch)

The shared-`/22` + CRDT-hostsfile + anycast-gateway idea from `network-architecture.md`
Mode 1 is a **lease-coordination layer**, not an L2 mechanism. On its own it collapses into
per-client `/32` mobility routes (the thing we rejected — flooding babel, convergence gap on
every roam). It rides *on top of* (c): the CRDT hostsfile supplies the MAC→IP data that both
pins leases mesh-wide and powers ARP suppression. Lease coordination + (c)'s scoped L2 =
seamless roaming without `/32` explosion.

## Why not batman-adv

batman-adv is the obvious "make many APs look like one L2" tool, and we deliberately do not
use it — as the island data plane or anywhere. The reasons, in priority order:

1. **Unauthenticated fabric → cannot secure untrusted peers.** batman has no per-identity
   membership; anyone on the fabric is a member. That makes it **trusted-only**, which is a
   *strict subset* of what we need — our whole premise is safe peering with *untrusted*
   nodes on a shared backhaul. An authorized-VNI overlay supports the untrusted-peer case;
   batman structurally cannot. We choose the superset.
2. **Physical, fuzzy membership fights identity-gating.** "Whoever is on the fabric" is the
   opposite of the authorized-set model ([identity-peering-requirements](identity-peering-requirements.md)
   R2c). Scoping batman to a member subset over a shared broadcast backhaul needs per-island
   VLANs or point-to-point links anyway — no simpler than an overlay.
3. **L2 broadcast scaling** — the original objection: batman floods across the L2 domain;
   this is the flat-mesh wall the architecture exists to avoid (`philosophical-outcomes.md`
   §1). Scoped-to-a-room it is tolerable, but it buys nothing (c) doesn't.
4. **Less observable than L3** (`prior-art.md` §5–6) — a standing objection to L2 mesh
   protocols in this project.
5. **Reintroduces an L2 mesh protocol the L3-invariant architecture deliberately omitted.**
   The whole design is "heterogeneous links stitched by L3"; batman is a competing L2 mesh
   layer under that.

Net: batman would cap islands at *trusted-only*; (c) supports *untrusted-peer* islands with
per-identity authorization. Same roaming benefit, strictly more capability.

## Near-term path (single trusted fleet)

Most of this document is the *permissionless* design. For a **single-owner, trusted fleet**
(the current reality), the hard machinery is unnecessary — every node is trusted, so there
is nothing to scope out. The cheap wins, in effort order:

1. **Verify the shipped 802.11r path on hardware** — the FT config already exists
   (`setup-wireless.sh`, `FT_KEY`/`r0kh`/`r1kh`) but is untested. Lowest-effort roaming win.
   (beads `2km`, `0pv`)
2. **Trusted-simple island = `e21` Mode 1, no VXLAN, no identity.** Co-located trusted nodes
   in one RF-island share **one `/24` over the existing `br-mesh` backhaul + the CRDT
   hostsfile (MAC→IP uniqueness) + 802.11r**. This is option (a) RF-island-native bridging —
   rejected *only* for the untrusted case, and correct here. Delivers seamless roaming with
   machinery mostly already planned in `e21`. VXLAN/EVPN-lite (option (c)) waits until there
   are untrusted peers to scope.
3. **Channel hygiene by hand** — non-DFS 5 GHz backhaul (already in the `BACKHAUL_BAND=5g`
   work); if more than one co-located cluster, manually assign different channels. No
   graph-coloring automation needed at fleet scale.
4. **Client-bounce as a diagnostic** — logging which client MACs bounce between which nodes
   identifies roaming-adjacent node sets (where to form islands). A cheap manual precursor to
   auto-formation.

**Deferred until permissionless / untrusted peers exist:** VXLAN/EVPN-lite data plane,
identity-gated membership authz, capability beacon, reputation layer, `/8` migration (a
single fleet fits in `10.42.0.0/16` = 256 `/24`s), cross-fleet merge + collision guard,
channel graph-coloring automation.

## Open questions

- **Membership signals** — how an island's member set is proposed and agreed: RF proximity
  / link quality (batman-style TQ or RSSI on the backhaul), **observed client-bounce** (a
  client flapping between nodes A and B proves they are roaming-adjacent), wired adjacency.
- **Per-island DHCP authority** — the subnet-claim CRDT gives the island one `/22`; which
  member serves DHCP, or how members coordinate ranges (CRDT hostsfile already gives
  MAC→IP uniqueness). Relates to `e21`.
- **VNI membership authorization + revocation** — blocked on the identity pass (`rp9`,
  `met`) and the reputation layer (R5).
- **Channel coordination details** — the graph-coloring assignment algorithm, DFS backup
  channel, and the cutover health-gate/rollback mechanics.
- **Backhaul address-space ceiling** — `10.254.0.0/16` (16-bit host) hits ~50%
  birthday-collision odds near 300 nodes; re-derivation salt handles individual collisions
  but it is an administrative ceiling for large/permissionless deployments. IPv6 backhaul was
  ruled out (iroh 1.0 surfaces private IPv4, not IPv6-ULA, as mDNS candidates).
- **Cross-fleet merge + collision guard** — negotiating a shared CRDT between different-owner
  fleets with bad-actor mitigations; the client-space collision guard when two fleets meet.

## Cross-references

- [design-principles](design-principles.md) — the no-config criterion this all serves
- [identity-peering-requirements](identity-peering-requirements.md) — the security needs
  islands surfaced
- [network-architecture](network-architecture.md) "Mode 1" — the flat-island precursor
- [prior-art](prior-art.md) §5–6 — foreign-mesh interop / the L3-NAT degradation path
- Beads: `e21` (epic), `2km` (FT keys across islands), `0pv` (roaming validation)
