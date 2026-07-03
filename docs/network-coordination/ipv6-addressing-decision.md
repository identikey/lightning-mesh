# Decision: IPv6 overlay vs IPv4 subnet claims

**Bead:** `mjolnir-mesh-bsa` · **Status:** DECIDED 2026-07-02 — **stay IPv4; IPv6 rejected as not worth the risk**
**Unblocks:** `e21` (service-mesh architecture pass)

## Decision

**The mesh stays IPv4.** The backhaul collision cliff — the one concrete IPv4
deficiency — is fixed with CRDT collision detection + re-derivation (bead filed),
not by changing address families. Per-service addressing, the requirement that
looked like it forced IPv6, is already handled by **iroh**: services are reachable
by node-id (+ ALPN), which is location-independent, cryptographically bound, and
has no scarcity. IP addresses in this system are an access-edge convenience, not
the service identity layer — so the case for IPv6 collapses to interop, and
interop alone does not pay for the risk.

Revisit triggers (either reopens this decision):
- A **single giant physically-connected 802.11s island** approaching the limits of
  claimed IPv4 space (hundreds of co-located nodes / >65k devices in one fabric).
- A hard **interoperability requirement** with IPv6-first mesh ecosystems
  (LibreMesh et al) where translation at the border is insufficient.

## How we got here

An initial draft of this doc recommended "IPv6 spine / IPv4 edge" (ULA /48 per
mesh, identity-derived node /128s, RFC 9229 v4-via-v6 backhaul next-hops, client
/24s unchanged). It scored well on paper. Adversarial review then surfaced the
following, which reversed the call:

### 1. The service-addressing requirement was misassigned (decisive)

The strongest driver for IPv6 was "every service needs a stable address" (e21).
But the mesh already has a better answer: **iroh node-ids**. A service behind any
node is dialable by public key regardless of where it sits, survives renumbering
and roaming, and needs zero address-space coordination. Putting service identity
in IP addresses would have *duplicated* a layer iroh already provides, in a worse
(scarce, location-coupled) namespace. With R3 gone, IPv6's remaining benefit was
interop — nice-to-have, not requirement-grade.

### 2. ULA + dual-stack: IPv4 wins anyway

RFC 6724 default policy gives IPv4 precedence over ULA `fc00::/7`. Stock clients
(macOS, Windows, Android) reaching a dual-published `.mesh` service would pick
IPv4 essentially always. The client-side v6 lane would carry ~no traffic while
still costing a full second stack — the classic "deployed IPv6 and nothing used
it" failure. Workarounds (AAAA-only mesh names) exist but add fragility.

### 3. v4-via-v6 vs mt76 hardware flow offload (unproven on our silicon)

RFC 9229 support exists in babeld 1.13 + kernel ≥ 5.2, but an IPv4 route with an
IPv6 next-hop is exactly the exotic path that MediaTek `mtk_ppe` hardware offload
on the mt7986 fleet is most likely to mishandle (silent CPU fallback or
misforwarding). Would have required gating the whole migration on offload
validation.

### 4. PMTU black holes over the overlay

IPv6 forbids router fragmentation; it depends on ICMPv6 PMTUD working through a
sub-1500-MTU QUIC-encapsulated path (`mjolnir0`) — the textbook v6 black-hole
setup. Solvable (RA MTU + MSS clamp) but another standing operational hazard.

### 5. Dual-stack operational tax

Every incident becomes "v4, v6, or the interaction?" — with happy-eyeballs masking
failures on clients while operators chase ghosts. The transition state is the
worst state, and it would have been the *permanent* state (client edge was staying
v4 indefinitely). Prior first-hand IPv6 migration attempts hit exactly this.

### 6. The collision cliff has a cheap native fix

The real IPv4 defect: the derived backhaul address `10.254.<blake3(node_id)[0..2]>`
has 16 bits of entropy → ~50 % birthday-collision odds at ~300 nodes, with no
resolution protocol. But the mesh already runs a claim CRDT with deterministic
first-writer-wins conflict resolution for client /24s. The same machinery applies:

- **Detect:** backhaul addresses become gossiped claims (`/backhaul/{addr}`),
  piggybacking on the existing anti-entropy loop (20 s full-map rebroadcast,
  fleet-validated). A collision is a `Conflict{winner, loser}` merge verdict —
  the code path that already exists for subnets.
- **Resolve:** the HLC loser re-derives deterministically (next hash iteration:
  `blake3(node_id || counter)`), rewrites its claim, reconfigures `br-mesh`.
  Same shape as losing a /24 claim.
- **Headroom (optional):** widening the backhaul pool (e.g. CGN `100.64.0.0/10`,
  23 bits → ~50 % at ~3 400 nodes; or `10.128.0.0/9`) makes collisions rarer
  before resolution even runs — a config change, not an architecture change.

Tracked as its own bead (see References).

## What IPv4 costs us (accepted, eyes open)

- **256 sites per mesh** at /24-from-/16 defaults; `10.0.0.0/8` expansion raises
  this to 65 536 sites but is a mesh-wide config change. Accepted: federation
  (`yau`) can also partition address spaces across meshes.
- **Interop** with IPv6-first mesh stacks stays translation-shaped if it ever
  matters. Accepted: not a current requirement.
- **No end-to-end v6 for v6-native clients.** Accepted: no demand signal.

## Options considered (summary of the full pass)

| Option | Verdict |
|---|---|
| A′ — IPv4 + CRDT backhaul-collision resolution (+ optional pool widening) | **ACCEPTED** |
| C — IPv6 spine / IPv4 edge (ULA /48, v4-via-v6) | Rejected: R3 dissolved by iroh; ULA-precedence + offload + PMTU + dual-stack risks not paid for by remaining benefits |
| B — IPv6-only + NAT64/DNS64 | Rejected: breaks v4-only clients; highest migration risk |
| D — IPv4 + NAT between sites | Rejected on ethos: NAT is an implicit authority; service addresses become observer-dependent |

## References

- `network-architecture.md` (shipped routed-/24 model), `gossip-and-crdt.md`
  (claim CRDT + anti-entropy), `babel-routing.md`
- RFC 6724 (address selection — the ULA trap), RFC 9229 (v4-via-v6, unused),
  RFC 4193 (ULA, unused)
- Beads: `e21` (service-mesh pass, now unblocked — service identity = iroh
  node-id), `0yb` (gossip address book), `yau` (federation), backhaul-collision
  bead filed from this decision
