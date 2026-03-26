# P2P Resilience: Centralization Analysis & Plan

## Current Architecture

mjolnir-mesh uses iroh QUIC endpoints + iroh-gossip for peer discovery, with moq-lite
for pub/sub data streams. The CLI exposes two roles:

```
mjolnir-mesh host --name=<room>   # creates room, prints join ticket
mjolnir-mesh join <ticket>        # joins via ticket
```

Once connected, all peers are functionally equal: each publishes a broadcast
(named by their EndpointId), discovers peers via gossip, and subscribes to
their streams. The asymmetry is entirely in **how the room is bootstrapped**.

---

## Where Centralization Lives

### 1. Ticket embeds a single address (ticket.rs)

```rust
pub struct MeshTicket {
    pub name: String,
    pub addr: EndpointAddr,   // <-- one peer's address
    pub topic_id: [u8; 32],
}
```

The ticket encodes exactly one `EndpointAddr` — the host's. This is the only
bootstrap entry point for gossip. If that peer is unreachable when a new peer
tries to join, the ticket is dead.

**What's already good:** The `topic_id` is deterministic (`blake3(room_name)`),
so any peer in the room could theoretically produce a valid ticket with their
own address. The protocol doesn't require the original host — it just requires
*some* live peer's address.

### 2. Host-only gossip bootstrap (mesh.rs:72-81 vs 113-123)

```rust
// Host: subscribe with no bootstrap peers
self.gossip.subscribe(topic_id, vec![]).await

// Joiner: subscribe with host as sole bootstrap
self.gossip.subscribe_and_join(topic_id, vec![ticket.addr.id]).await
```

The host calls `subscribe()` with an empty bootstrap list (it *is* the
bootstrap). Every joiner calls `subscribe_and_join()` with the host's
EndpointId as the sole bootstrap peer. If the host is gone, gossip join fails.

**What's already good:** iroh-gossip propagates peer addresses transitively.
Once peer B has joined via host A, and peer C joins via A, peers B and C
discover each other through gossip — they don't need A anymore for ongoing
communication. The gossip mesh is self-healing among connected peers.

### 3. No ticket regeneration (mesh.rs:71-110)

Only `host_room()` produces a ticket. There is no `generate_ticket()` or
`invite()` method. A peer that joined via `join_room()` has all the information
needed to produce a ticket (room name, own address, topic_id) but the code
doesn't expose this.

### 4. One room per node, no rejoin (mesh.rs:26-31)

```rust
pub struct MeshNode {
    room: Mutex<Option<Room>>,  // single slot
}
```

A node can hold exactly one room. There's no mechanism to rejoin after
disconnect, re-enter a room the node was previously in, or handle the room
surviving across node restarts.

---

## What Happens in Failure Scenarios

| Scenario | Existing peers | New joiners |
|----------|---------------|-------------|
| Host leaves after peers connected | Survive (gossip + direct QUIC) | Cannot join (ticket dead) |
| Host leaves before any peers join | N/A | Cannot join |
| Non-host peer leaves | Others unaffected | Can still join via host |
| Network partition (host isolated) | Partitioned peers survive in subgroups | Can join host's partition only |
| Host restarts with same identity (IROH_SECRET) | Peers may reconnect via gossip | Ticket still valid (same addr) |
| Host restarts with new identity | Peers unaffected among themselves | Old ticket dead |

The critical gap: **once the host is gone, the room is closed to newcomers**,
even though the room itself is alive and functioning among existing peers.

---

## What Pure P2P Requires (Theory)

A fully decentralized room needs:

1. **Any peer can bootstrap new joiners.** Every participant should be able to
   produce a valid join ticket containing their own address and the room's
   topic_id.

2. **Multi-address tickets (optional but helpful).** A ticket with N addresses
   succeeds if any 1 of N is reachable. More addresses = more resilient
   bootstrap.

3. **Distributed room state.** Currently "the room" is just a gossip topic +
   whoever happens to be subscribed. There's no durable membership list. This
   is fine for ephemeral rooms but means a room can't survive total peer
   departure and later revival.

4. **Peer-to-peer discovery without gossip bootstrap.** For truly
   infrastructure-free operation, peers could discover each other via:
   - DHT (iroh supports this via mainline DHT integration)
   - mDNS/local network broadcast
   - Out-of-band signaling (QR code, shared secret, etc.)

5. **Graceful departure.** When a peer leaves, it should hand off bootstrap
   responsibility — or all peers should already be capable of it.

---

## Plan: Getting There With What We Have

The good news: iroh and iroh-gossip already provide most of the primitives.
The changes are mostly in mjolnir-mesh's usage of them, not in the underlying
transport.

### Phase 1: Any Peer Can Mint Tickets

**Goal:** Remove the host/joiner asymmetry for ticket generation.

**Changes:**

- Add `MeshNode::generate_ticket(&self, room_name: &str) -> Result<String>`
  that works for any peer currently in a room. It uses the peer's own
  `EndpointAddr` + the deterministic topic_id.

- The `host` command is then just: create room + generate_ticket + print it.

- Add an `invite` CLI command (or just print the ticket periodically) so any
  peer can share a working ticket at any time.

- Optionally: `Room` stores the room name so `generate_ticket` doesn't need
  it passed in.

**Complexity:** Small. The ticket format doesn't change. `MeshTicket` already
has everything needed — we just need to construct it from any peer's state.

### Phase 2: Multi-Peer Bootstrap

**Goal:** Tickets survive individual peer failure.

**Changes:**

- Extend `MeshTicket` to carry a `Vec<EndpointAddr>` instead of a single addr.
  The joiner tries each address until one succeeds.

  ```rust
  pub struct MeshTicket {
      pub name: String,
      pub addrs: Vec<EndpointAddr>,  // any live peer works
      pub topic_id: [u8; 32],
  }
  ```

- When generating a ticket, a peer can include addresses of other known peers
  from its `Room.peers` set (it learns their `EndpointAddr` via gossip).

- `join_room` iterates `ticket.addrs` and passes all their EndpointIds to
  `subscribe_and_join()` as bootstrap peers.

**Complexity:** Medium. Requires ticket format change (versioned format or
backwards-compatible encoding). Gossip already supports multiple bootstrap
peers.

### Phase 3: Unified Host/Join Flow

**Goal:** Eliminate the host/join distinction at the protocol level.

**Changes:**

- Replace `host` and `join` with a single command:
  ```
  mjolnir-mesh room <name> [--bootstrap <ticket>]
  ```

- Without `--bootstrap`: creates a new room (equivalent to current `host`).
- With `--bootstrap`: joins existing room (equivalent to current `join`).
- In both cases, the node immediately becomes a full peer that can mint tickets.

- Under the hood, both paths call the same `Room::new()` — the only difference
  is whether `subscribe()` or `subscribe_and_join()` is called on gossip.

**Complexity:** Small refactor of CLI + mesh.rs. The underlying room logic
doesn't change.

### Phase 4: Room Persistence (Future)

**Goal:** A room can survive total peer departure and be revived.

This is a larger architectural question and may not be needed for MVP. Options:

- **Sticky rooms via DHT:** Publish room metadata to iroh's DHT keyed by
  topic_id. Any peer can discover the room by name without a ticket.

- **Seed peers:** Designate long-lived nodes (not centralized servers, but
  known-stable peers) that stay subscribed to rooms and serve as reliable
  bootstrap points.

- **Room state on disk:** Persist the room name + last-known peer addresses
  so a restarting node can attempt to rejoin automatically.

These are out of scope for now but the Phase 1-3 changes don't preclude them.

---

## Summary

| Phase | What | Effort | P2P Impact |
|-------|------|--------|------------|
| 1 | Any peer mints tickets | Small | Eliminates single-point bootstrap failure |
| 2 | Multi-addr tickets | Medium | Tolerates N-1 bootstrap peer failures |
| 3 | Unified room command | Small | Removes artificial host/join distinction |
| 4 | Room persistence | Large | Survives total peer departure (future) |

Phases 1 and 3 can be done together as a single refactor. Phase 2 is
independent. All three use existing iroh/gossip primitives — no new
dependencies or protocols needed.
