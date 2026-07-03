# MANET Dynamic Addressing — relevance to the mesh CRDT address list

**Source:** Khatri, Kolhe, Giri, *"Dynamic Address Allocation Algorithm for Mobile Ad hoc
Networks"*, VESIT Mumbai, 2016. arXiv:[1605.00398](https://arxiv.org/abs/1605.00398).

**Reviewed:** 2026-06-16 · **Context:** our CRDT-replicated IP/subnet list across mesh nodes
(`crates/mjolnir-mesh/src/crdt/`, `src/alloc.rs`, `docs/archive/network-coordination/dhcp-crdt.md`).

---

## What the paper actually proposes

A **hierarchical, cluster-head-based dynamic IP allocation scheme** for MANETs that
*deliberately avoids a replicated global address table*:

- **"IP resembles topology"** — IP addresses encode a node's position so routing is easier.
  Identity is decoupled from address (MAC is the stable key; IP is reassignable).
- **Octet-level hierarchy** — a node is at "level k" if its dotted-decimal address has k
  trailing zero octets. A level-k node is the *cluster head* for up to 255 nodes at level
  k-1 (e.g. `10.1.0.0` → `10.1.x.0` → `10.1.x.y`). Caps at 65 536 nodes per /16.
- **Localized allocation, no network-wide broadcast** — a joining node asks a directly
  reachable "allocator"; cost of assignment is independent of network size. No DAD.
- **Soft-state reclamation** — cluster heads expect periodic reports; on silence past a
  threshold they *actively probe* the node and reclaim its address on no-reply (§VII).
- **Recursive lookup** (`FindAddress`) — because there is no global table, finding a node's
  current address costs up to `255·(k+1)` messages worst case (§IX).
- **Partition/merge by re-addressing** — merge two networks by offsetting prefixes or
  "increasing k" to grow the hierarchy and dodge collisions (§X–XI).

## Why most of it is the road we *didn't* take

The paper sits on the **opposite side of a classic tradeoff** from mjolnir-mesh. It minimizes
replicated state at the cost of complex lookup/hierarchy/self-healing protocols. Our CRDT
replicates the full table to every router, making lookup a local map hit and partition-merge
clean, at the cost of O(N) state per node and gossip bandwidth — the right call for our scale
(router hardware, hundreds of devices, real RAM, cheap gossip).

| Dimension | Paper (MANET) | mjolnir-mesh |
|---|---|---|
| Global address table | Avoided — localized cluster tables | Fully replicated CRDT on every router |
| Lookup cost | up to `255·(k+1)` messages | O(1) local map read |
| Allocation authority | one cluster head per pool (serialization point) | leaderless; any router assigns, FWW resolves |
| Reclamation | soft-state timestamps + active probe | lease `expiry` + reaper (subnet claims: **none**) |
| Routing | IP encodes topology | delegated to Babel; Iroh dials by NodeId |

**Not relevant to us, and why:**

- *Octet trailing-zeros hierarchy* — exists to make IP encode routing topology. We don't need
  that: Babel routes, Iroh dials by NodeId.
- *Recursive `FindAddress`* — its whole reason to exist is the *absence* of a replicated
  table. Our CRDT gives every node the table; reintroducing recursive lookup would throw away
  the CRDT's main payoff.
- *MAC/address-equals-identity decoupling* — the paper "discovers" it; we already key leases by
  MAC and have Iroh NodeId as stable identity. Validates our design, adds nothing new.
- *No-broadcast scalability* — the paper's central selling point only bites at thousands of
  mobile nodes. Gossiping every lease to every router is simpler and fine at our scale.

## Takeaways worth keeping

1. **Soft-state reclamation is the one genuine gap (see below).** The paper's
   report → threshold → probe → reclaim loop is exactly the lifecycle our subnet-claim ledger
   is missing.
2. **Pool delegation maps cleanly to a future multi-tier alloc.** A claimed `/22` can become
   the `base` for a nested `pick_subnet` if a site ever sub-allocates to sub-sites. Premature
   now, but `alloc.rs` already composes this way.
3. **Merge-by-re-addressing ≈ our "loser re-picks".** The paper's offset/"increase k" merge is
   conceptually our FWW-loser-rewrites-its-claim, plus widening the mesh base prefix as an
   exhaustion escape hatch (already tracked as knob (B) on `mjolnir-mesh-yau`).

---

## The missing lifecycle: subnet-claim reclamation

This is the one place the paper directly improves on our current design.

**Current state.** `LeaseEntry` (`crdt/lease.rs`) carries `expiry` and is reaped on a timer —
covered. But `SubnetClaim` (`crdt/subnet.rs`) has **no expiry and no liveness field by
design** — only `cidr`, `owner_node_id`, `site_name`, `claimed_at`. There is a
`SubnetClaimRelease` gossip message (`dhcp-crdt.md` §8.2) for *graceful* release, but no
mechanism for *ungraceful* departure.

**The leak.** A router that is decommissioned, reflashed, or dies without emitting
`SubnetClaimRelease` holds its subnet (a `/24`, `/22`, or `/16`) **forever** in the
`/subnets/` ledger. `alloc::pick_subnet` treats every entry in `claimed` as live, so the
abandoned block is never reused. Over time this fragments and fills `10.42.0.0/16` — the exact
exhaustion the recent governance work (`867bbdd` auto-downgrade, `mjolnir-mesh-eh3`,
`mjolnir-mesh-yau`) is trying to keep sound. `pick_subnet -> None` can fire while real
utilization is near zero.

**The fix (paper's pattern, adapted).** Add soft-state to the claim and reclaim on confirmed
absence:

1. Add a `last_seen: HLC` (or `renewed_at` + `lease_ttl`) to `SubnetClaim`; owners re-assert
   their claim on a heartbeat (piggyback on existing gossip/anti-entropy — no new traffic at
   small scale).
2. A claim whose `last_seen` is older than a threshold is *suspect*, not immediately free.
3. Before reclaiming, attempt a liveness probe of `owner_node_id` (we have NodeId reachability
   via Iroh) — the paper's "send intended message, delete on no-reply" step. This avoids
   evicting a router that is merely partitioned.
4. On confirmed absence, drop/tombstone the claim so `pick_subnet`'s `claimed` set excludes it
   and the block returns to the free pool. Reclamation should itself be a CRDT-safe operation
   (countersigned or FWW-tombstoned) so a healing partition can't resurrect a stale claim.

**Scope note.** `mjolnir-mesh-yau` already lists "stale-claim reclamation" under *DEFER until
dozens+ of sites* — at 4 routers, utilization is ~0% and the leak is harmless. This note does
not change that call; it documents *why* the lifecycle is missing and *what* the fix is, so the
deferred work is well-specified when scale justifies it. Tracked as the bead filed alongside
this doc.
