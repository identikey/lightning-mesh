# Philosophical Outcomes of the Architecture

**Status:** Vision / public-facing | **Date:** 2026-07-02

Lightning Mesh's core design decision is one sentence: **heterogeneous link
islands stitched together by L3 routing.** This document states what falls out of
that decision — not the mechanics (see
[network-architecture](../network-coordination/network-architecture.md)) but the
outcomes: what kind of network, and what kind of ownership, the architecture
makes possible. These are consequences of the structure, not aspirations layered
on top of it.

> **Naming:** Lightning Mesh is the public name of the project. Crates and
> binaries keep the `mjolnir-` prefix; the overlay interface is `mjolnir0`.

---

## 1. We are in the lineage of Cerf & Kahn

"Heterogeneous links stitched by a common routing layer" is not a novel bet — it
is the Internet's founding architecture. Cerf and Kahn's catenet (1974) faced
leased lines, packet radio, and satellite links that had to behave as one
network, and answered: don't unify the links, unify the layer above them. IP
became the narrow waist; every link technology became interchangeable plumbing.
It is the only network design that has ever scaled across five orders of
magnitude, and it outlived every physical technology it launched on.

Most mesh systems of the last twenty years bet the other way — make many radios
look like one Ethernet — and the ones that grew large enough (Freifunk on
batman-adv, most visibly) hit the flat-L2 broadcast wall and converged, under
duress, on link islands stitched by L3. Guifi.net and NYC Mesh run BGP between
heterogeneous zones for the same reason. Lightning Mesh starts where the
survivors arrived.

**Outcome:** our scaling limits are administrative (address space, trust,
directory size) — the kind you engineer through incrementally on a live network —
rather than physical (broadcast collapse), the kind that forces a migration.
This is the shape you never have to migrate away from.

## 2. The radio is mortal; the network is not

The L3 overlay — identity, routing, shared state — is the invariant. Everything
below it is whatever each node can do: 802.11s today, ethernet runs, 60GHz
point-to-point, LTE backhaul, QUIC over any internet egress tomorrow. Each is
just a link with a metric.

Consumer mesh products *are* their radio layer: an eero mesh is eeros talking to
eeros over eero's silicon, and the network dies with the product line. Lightning
Mesh has already retired an entire hardware segment (the MikroTik AP/STA
topology) without the architecture noticing — the L3 layer never knew what the
links were.

**Outcome:** genuine future-proofing and interoperability. The mesh outlives any
radio generation; any hardware that can speak the layer joins as an equal; and
independently built meshes can merge by linking at a single node.

## 3. Sovereignty is structural

Each node owns its own routed /24. Therefore: **no node is in a position to be
an authority over another node's segment** — not as policy, as topology.
Broadcast containment is blast-radius containment. A compromised node's L2
attacks (ARP spoofing, rogue DHCP) stop at its own segment because there is no
shared segment to poison. In flat-L2 systems, containment is something you
configure and can misconfigure; here it is something an attacker would have to
build infrastructure to violate.

This is what "symmetric and non-authoritative" means when it is load-bearing:
nobody is in charge, and nobody is *in a position to become* in charge.

**Outcome:** plug-and-play follows directly. Because there is no authority,
**there is no authority to configure** — no controller to designate, no leader
to elect, no DHCP server to bless. A node derives its address from its own
identity, claims its own subnet through the shared state layer, and routes.
Zero-configuration is not a convenience feature; it is what remains when there
is nothing whose existence would need configuring.

## 4. Discovery is a product, not an accident

Flat-L2 meshes get discovery "for free" from mDNS — meaning discovery is an
emergent property of physical adjacency, and it evaporates the moment topology
gets interesting (multi-hop, cross-site, partitioned). Being forced off flat L2
forced us to build discovery as a first-class system: a gossip-replicated CRDT
address book with no coordinator.

That looked like a cost. It is the sleeper asset. It works multi-hop and
cross-site, over the open internet; it survives partitions and merges on rejoin
with no leader election; and it binds names to cryptographic identities rather
than to presence on a wire. Routing is a commodity — Babel is excellent and
replaceable. A partition-tolerant, identity-keyed, mesh-wide directory is not.
As it grows to carry names, services, and membership, it becomes the thing the
network *is*.

## 5. The network is a projection of a set of keys

Every node's mesh address is derived from its cryptographic identity. cjdns and
Yggdrasil pioneered this move and deserve the credit; they dissolved link
islands entirely and paid in routing stretch and lost local L2 semantics.
Lightning Mesh keeps their deepest idea — addresses are identities — and marries
it to the catenet: real islands, locally fast, stitched by ordinary routing.

**The network becomes a projection of a set of keys.** The physical substrate —
which island, which radio, which continent — is routing detail. As it should be.

**Outcomes:**

- **Ownership by key, not by physical access.** A node's owner is whoever holds
  the key, authorized cryptographically rather than by being in the room. The
  mesh manages itself over itself.
- **Trust without trusting the wire.** Connections are end-to-end encrypted
  between identities; every forwarder sees only ciphertext. A trusted network on
  an untrusted substrate — which is what makes mixing your hardware with
  strangers' hardware safe.
- **People next.** Identity-by-key extends from nodes to users (IdentiKey):
  membership and service access answered by keys and webs of trust, not MAC
  filters and shared passwords. This also positions routing-trust hardening
  (route-origin validation: only the identity that hashes to a block may
  announce it) as a natural extension rather than a bolt-on — most meshes cannot
  build it because they have no identity layer.

## 6. Reachability is not a subscription

Publishing a service today means becoming a network wizard (port forwarding,
dynamic DNS, certificates, NAT traversal) or renting the capability back from
whoever enclosed it — a cloud, a tunnel provider, a coordination server.
Rent-seeking on *reachability* is one of the quieter forms of extractive
economics: the infrastructure to reach each other exists, has been enclosed, and
is sold back by subscription.

The wall matters most for services whose point is **being of service** rather
than extracting money — the neighborhood file share, the mutual-aid page, the
community wiki that will never have a business model and shouldn't need one.

On Lightning Mesh, joining *is* reachability: a Raspberry Pi joins and
`wiki.mesh` resolves from every node, locally with no internet and globally over
the encrypted overlay with it. No rented tunnel, no coordination server that
isn't yours.

**Outcome:** the mesh is a decentralized application platform where publishing
is a property of participation, not a product you subscribe to.

---

## Summary

| Structural decision | Philosophical outcome |
|---|---|
| L3 overlay is the invariant; links are plumbing | Network outlives hardware; any device joins as an equal; meshes can merge |
| Each node owns a routed /24 | No node can be an authority over another's segment; containment by topology |
| No authority anywhere | Nothing to configure: genuine plug-and-play |
| Discovery via gossip/CRDT, not the wire | Partition-tolerant, identity-keyed directory — the durable core |
| Addresses derived from keys | Network as projection of keys; ownership by key, not physical access |
| Joining is reachability | Services without rent; a platform for being of service |

## Related

- [DWeb talk source](../talk/dweb-2026-lightning-mesh.md) — the narrative form
  of these ideas.
- [Why decentralized mesh networking](why-decentralized-mesh.md) — motivation
  and system comparisons.
- [Prior art](../network-coordination/prior-art.md) — auditable comparison to
  CeroWrt/AHCP/OpenWISP.
