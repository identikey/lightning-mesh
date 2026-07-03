# P2P Resilience: Centralization Analysis & Plan

mjolnir-mesh is an Iroh-based mesh with distributed network coordination. Today the
CRDT coordinates **subnet claims** (wired and field-validated); DHCP-lease, DNS, and
service-discovery CRDT lanes are designed but not yet wired (the `e21` service-mesh
phase). This doc analyzes where centralization exists, what the failure modes are, and
the roadmap — the Summary table at the bottom is the honest per-layer status.

See [network-architecture.md](network-architecture.md) for the full architecture and
[gossip-and-crdt.md](gossip-and-crdt.md) for the CRDT data model.

---

## Centralization Analysis

### Ticket-based joining (ticket.rs)

Tickets are the bootstrap mechanism for new nodes entering the mesh. A ticket embeds
one or more `NodeAddr` values (multi-address support is already implemented) — Iroh
node IDs paired with their known addresses. The joiner tries each address until one
succeeds.

```rust
pub struct MeshTicket {
    pub name: String,
    pub addrs: Vec<NodeAddr>,   // multi-address: any live peer works
    pub topic_id: [u8; 32],
}
```

**Remaining centralization:** The ticket must be obtained out-of-band (shared link,
QR code, PSK-derived topic). The topic_id is currently derived deterministically from
the room name; a PSK-based derivation would prevent uninvited nodes from computing the
topic independently.

**Already good:** Multi-address tickets mean no single peer is a required bootstrap
point. Any peer in the mesh can mint a valid ticket using its own `NodeAddr`.

### DHCP coordination (planned — `e21`)

Shipped: each router's stock dnsmasq serves only that node's claimed /24 (subnet
claims prevent range overlap; no lease-level coordination is needed or wired).
Planned: CRDT lease replication so a device's `mac → ip` binding is mesh-wide.

**Centralization:** None either way — there is no single DHCP server today, and the
planned lease CRDT keeps it that way.

### DNS (planned — `e21`)

Design: DNS records replicated via CRDT across all nodes, no single authoritative
server, each node answering `.mesh` queries from its local replica. Not yet wired.

**Centralization:** None structurally, once built. Propagation lag means a freshly
added record may not be visible on all nodes immediately (eventual consistency).

### Routing (shipped)

Subnet ownership is CRDT-synced (`/subnets/{cidr}` claim ledger). Each router
redistributes its own subnet via Babel (`babeld`), which computes loop-free routes
directly over the 802.11s L2 backhaul for same-island peers and over the `mjolnir0`
iroh overlay for cross-site peers. A node going offline causes Babel to withdraw its
routes within seconds; other subnets are unaffected. See `babel-routing.md`.

**Centralization:** None. Routes live in Babel, not the CRDT; only the ownership
ledger is CRDT state (tombstoned on graceful release).

### Service discovery (planned — `e21`)

Design: service registrations CRDT-synced and tied to device leases; when a device's
lease expires, its service entries are cleaned up. Not yet wired.

**Centralization:** None, once built. Any node can answer service discovery queries.

### Gossip transport (iroh-gossip)

iroh-gossip is already fully P2P. Once peers have joined a topic, the gossip mesh is
self-healing — no bootstrap peer is needed for ongoing communication.

**Centralization:** Bootstrap only. The first join requires a known peer address (from
the ticket). After that, gossip propagates peer addresses transitively.

### Iroh relay infrastructure

Iroh uses n0's relay servers for NAT traversal fallback when direct connections fail.
This is an external dependency on n0's infrastructure (or a self-hosted relay).

**Centralization:** Real but bounded. Relay is used only for connection establishment,
not for data. If n0's relays are unreachable, direct connections still work where NAT
allows. Self-hosted relay is supported by Iroh.

---

## Failure Scenarios

(For the planned lanes, "DHCP/DNS" below describes the design's behavior.)

| Scenario | Effect |
|----------|--------|
| Router goes offline | Other routers keep serving DHCP/DNS/routing. CRDT state is fully replicated — no data loss. |
| Network partition | Each partition operates independently with full CRDT state. On rejoin, CRDTs merge automatically. |
| Daemon crash | systemd restarts the daemon. Anti-entropy sync rebuilds any missed CRDT updates from peers on reconnect. |
| All routers restart simultaneously | CRDT state rebuilds from disk (if persisted) or anti-entropy from peers. New joins blocked until at least one router rejoins gossip. |
| Bootstrap peer unreachable | Multi-address ticket provides fallback peers. If all ticket peers are gone, out-of-band re-sharing needed. |
| n0 relay unreachable | Direct connections unaffected. NAT-traversal-dependent connections fall back to relay-less paths or fail. |

The critical remaining gap: **if all peers with valid tickets are offline simultaneously,
new nodes cannot join** until at least one existing peer comes back online with a
reachable address. This is inherent to any ticket-based bootstrap.

---

## What Pure P2P Requires

A fully decentralized mesh needs:

1. **Any peer can bootstrap new joiners.** Every participant should be able to produce
   a valid join ticket. This is already implemented.

2. **Multi-address tickets.** A ticket with N addresses succeeds if any 1 of N is
   reachable. Already implemented in ticket.rs.

3. **Distributed coordination state.** DHCP, DNS, routing, and service discovery all
   use CRDT replication — no central coordinator. Already implemented.

4. **Peer-to-peer discovery without a fixed bootstrap.** For truly infrastructure-free
   operation: DHT (iroh supports mainline DHT), mDNS for local networks, or PSK-derived
   topic IDs that any pre-authorized node can compute independently.

5. **Graceful departure.** When a peer leaves, its CRDT tombstones propagate so the
   rest of the mesh can clean up its leases and routes. Partially implemented via lease
   TTLs.

---

## Implementation Roadmap

### Phase 1: Multi-address tickets — DONE

ticket.rs now carries `Vec<NodeAddr>`. Any peer in the mesh can mint a ticket using
its own node address. Joiners try each address in order.

### Phase 2: Unified join flow (mesh.rs) — DONE

mesh.rs has `enter_room()` which handles both the "first node" case (no bootstrap peers)
and the "joining" case (bootstrap from ticket addrs). The host/join asymmetry at the
protocol level is gone.

### Phase 3: Mesh lib extraction — PLANNED

Extract a generic stream interface from room.rs so the mesh core (gossip, ticket, peer
management) can be used independently of the VPN coordination layer. This enables
embedding the mesh library in other Mjolnir components (e.g., guest agent peer
communication) without taking the full VPN stack.

### Phase 4: subnet-claim coordination — DONE; lease/DNS/service lanes — PLANNED (`e21`)

Subnet-claim CRDT coordination shipped and is field-validated: merge on gossip receive,
plus anti-entropy as a full-claim-map rebroadcast every 20s. The remaining lanes —
lease replication, DNS, service discovery (see gossip-and-crdt.md and the archived
dhcp-crdt.md design) — are the `e21` service-mesh phase:
- Lease TTL expiry + tombstone propagation
- Mesh-wide DNS from CRDT state
- Service registration/discovery

### Phase 5: Route persistence & offline resilience — FUTURE

Store CRDT state to disk so a restarting node can serve DHCP/DNS immediately without
waiting for anti-entropy. Store-and-forward for messages to temporarily offline nodes.
Explore DHT-based room discovery (iroh mainline DHT) to eliminate the ticket
requirement entirely for well-known mesh names.

---

## Summary

| Layer | Centralization | Status |
|-------|---------------|--------|
| Ticket bootstrap | Any peer can mint; multi-addr fallback | Done |
| Subnet claims | CRDT, first-writer-wins, no authority | Done (field-validated) |
| Routing | Babel over 802.11s L2 + `mjolnir0` overlay; CRDT for subnet claims only | Done (field-validated) |
| DHCP lease coordination | CRDT, no single server | Planned (`e21`) |
| DNS | CRDT-replicated | Planned (`e21`) |
| Service discovery | CRDT, tied to leases | Planned (`e21`) |
| Gossip | iroh-gossip, fully P2P | Done |
| NAT traversal | n0 relay (external dep) | Accepted / self-hostable |

The lease/DNS/service CRDT lanes (`e21`) are the remaining gap between the current
implementation and the full design. The gossip, subnet-claim, and routing layers are
deployed and P2P.
