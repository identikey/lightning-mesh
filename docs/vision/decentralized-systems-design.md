# The Hard Parts of Having No Authority

**Status:** Vision / public-facing (technical) | **Date:** 2026-07-04

> Companion doc: [Philosophical outcomes of the architecture](philosophical-outcomes.md)
> — what the design *means*. This one is what it *is*: the distributed-systems
> engineering that has to be real for those outcomes to hold. The public talk
> narrative lives in [the DWeb talk source](../talk/dweb-2026-technical-arc.md).

Most of the decentralized-web conversation happens at the application layer —
storage, identity wallets, tokens, protocols for moving content around. Under all
of it sits a quieter question that decides whether any of it actually works
without a boss: **when there is no central authority, how do independent nodes
agree on anything, and how do you stop one actor from pretending to be a
thousand?**

Those are the two hard parts, and they have names. **Total ordering** — agreeing
on *the* sequence of events without a coordinator to declare it. **Sybil
resistance** — making identity cost something, so a resource can't be seized by
fabricating participants. Centralized systems don't solve these problems; they
*define them away*. A SaaS backend has a single database whose transaction log
*is* the total order, and an account system with admins that *is* the identity
gate. Both problems are answered by fiat, by the same authority that decentralization exists to remove.

Take the authority away and both problems come back as first-class engineering.
This document is how Lightning Mesh answers them, why the answers look the way
they do, and why the resulting guarantees are *different* from — not worse than —
the SaaS ones. Different on purpose, because the thing being optimized is
different: not availability-under-a-throat-to-choke, but function-with-no-throat-
to-choke at all.

---

## 1. The guarantee we're trading for

Start honest about what's given up. The CAP theorem says: when the network
partitions (and a radio mesh partitions constantly — a node sleeps, a link drops,
a building's concrete eats a signal), you pick Consistency or Availability. A
bank picks C: refuse the write rather than diverge. A coordinator-based system
*can* pick C because it has a coordinator to refuse on everyone's behalf.

We pick **A**, every time, and we don't get to pick otherwise, because there is
no coordinator to do the refusing. A node cut off from the rest of the mesh must
keep resolving names, keep routing, keep answering DHCP for the phones in the
room — on its own, immediately, with whatever it last knew. Consistency becomes
*eventual*: partitions heal, states converge, and the interesting engineering is
in guaranteeing that convergence is **deterministic** — that two nodes which saw
the same events in different orders end up bit-identical, with no round of "whose
version wins" that needs a human or a leader.

That is the whole game: **strong eventual consistency without consensus.** No
Raft, no Paxos, no leader election, no quorum. If you find yourself electing a
leader, you've reintroduced the authority — and with it the single point of
capture, subpoena, outage, and enshittification that the project exists to route
around.

## 2. Conflict-free replication: making "just merge" true

The tool for AP-with-deterministic-convergence is the **CRDT** (conflict-free
replicated data type). The contract: define your shared state so that the merge
of any two replicas is **commutative, associative, and idempotent**. Get that,
and gossip's three sins — duplicated messages, reordered messages, lost messages
— stop being correctness problems. A duplicate merges to the same state
(idempotent). Out-of-order delivery converges anyway (commutative). A message
that arrives late, after ten others, still lands correctly (associative). You
stop needing reliable, ordered delivery, which is exactly the thing you cannot
get on a flaky multi-hop radio mesh.

Lightning Mesh's shared state is a handful of these, each a keyed map with a
deterministic per-key merge:

| Lane | Key | What it carries | Merge rule |
|---|---|---|---|
| **Subnet claims** | CIDR | which node owns a routed `/24` | first-writer-wins on claim time |
| **Service names** | name | `wiki.mesh` → owner + address + port | owner-bound; first-writer-wins on first-claim |
| **Address book** | node id | a peer's direct/relay addresses | last-writer-wins (self-announced) |
| **User directory** | username | IdentiKey identity records | last-writer-wins |

Every one of them replicates the same way and converges the same way. There is no
schema server, no migration coordinator; a node that predates a lane simply
doesn't understand its messages and skips them (§6). The map grows with the
number of *routers*, not the number of devices — the routing table and the
directory both stay small because nodes announce "I own this block / this name,"
never "here is everything attached to me."

## 3. Total order without a clock you can trust

Merge rules like "first-writer-wins" and "last-writer-wins" smuggle in a word
that does enormous work: **wins**. Wins *by what*? You need a total order over
events generated on machines with no shared clock, no coordinator to hand out
sequence numbers, and wall clocks that disagree by seconds and occasionally run
backwards. This is the total-ordering problem in its rawest form.

The answer is a **Hybrid Logical Clock** (`crdt/hlc.rs`). An HLC stamp is a
triple:

```
(wall_clock_ms, counter, node_id)
```

ordered lexicographically: wall clock first, then a monotonic counter to break
ties within the same millisecond, then the node id as a final, *guaranteed-unique*
tiebreak. Why this shape, and not the alternatives:

- **Not pure physical time.** Clock skew would make ordering a lie, and two events
  in the same millisecond have no order at all. The counter fixes ties; the
  wall-clock component keeps stamps *legible* (a human and a log can tell roughly
  when something happened) and keeps causally-later events ordered after earlier
  ones as clocks advance.
- **Not a pure Lamport clock.** Lamport timestamps give you a causal order but
  drift arbitrarily from real time, so an operator can't read them and they can't
  be reconciled with wall-clock intuitions during debugging.
- **Not vector clocks.** Vector clocks detect concurrency precisely but grow with
  the number of nodes — O(N) state stamped onto every record, unbounded in a mesh
  where nodes come and go forever. HLC is *constant size*.

The load-bearing move is the **`node_id` tiebreak**. Because node ids are globally
unique (they're public keys — see §4), no two distinct events ever compare
*equal*. The order is **total**, not partial: every pair of events has a
definite winner, computable independently on every node, with zero coordination.
That is what makes "first-writer-wins" a function rather than an argument. Two
nodes that receive two conflicting subnet claims in opposite orders both compute
the same winner, because the winner is `min(HLC)` and `min` doesn't care what
order you fold the set in.

**First-writer-wins vs last-writer-wins** is then just which end of the order you
take. Claims and names are *first*-writer-wins: the earliest legitimate claimant
holds the resource, so a later claimant can't preempt an established one (stability
— your `/24` doesn't get yanked because someone rebooted with a fast clock).
Self-announced data like a node's own address is *last*-writer-wins: only the
subject announces its own address, so the newest announcement is authoritative by
construction and there's no conflict arm at all.

## 4. Sybil resistance: the network is a projection of keys

Total ordering makes conflicts *resolve*. It does not make them *fair*. "First
writer wins" is only just if writers can't cheaply manufacture priority — and the
cheapest cheat in any open system is the **Sybil attack**: fabricate a thousand
identities, claim a thousand subnets, squat a thousand names, outvote everyone.
Centralized systems block this at the account layer (phone numbers, KYC, an admin
who bans you). With no admin, identity itself has to be expensive to forge.

Lightning Mesh's answer is structural: **identity is a keypair, and the network's
addresses and claims are derived from it.**

- A node *is* an **iroh node-id** — an Ed25519 public key, a point in a space of
  2²⁵⁶. You don't get one from a registry; you generate one, and its scarcity is
  cryptographic, not administrative.
- A node's overlay address is **`10.254.<blake3(node_id)>/16`** — derived by hash
  from the key. You cannot choose your address; it falls out of your identity. To
  occupy a specific overlay address you would have to find a key that hashes to
  it — a preimage attack, not a configuration option.
- This makes **route-origin validation** a natural extension rather than a bolt-on
  (the address is *derived* from the key in `alloc.rs` via `blake3(node_id)`; the
  matching enforcement — reject a block announcement from any key that doesn't hash
  to it — is not yet wired, but the primitive is already the one the addressing
  uses). Most meshes can't build this at all because they have no identity layer to
  anchor it to; here it falls out of the addressing that already ships.

So the Sybil cost is real where the resource is bound to a key: you can't forge a
node-id, can't spoof another node's derived address, can't announce a route you
don't hash to. **Making a thousand identities is easy; making a thousand
identities that each already own the thing you want is not.**

### The honest frontier

Where the resource is *not* yet bound to a key, Sybil resistance is still partial,
and it's worth stating plainly because it's the live research edge of the project.
**Service names** (`wiki.mesh`) are claimed **trust-on-first-use**: the first
node to claim a name owns it (first-writer-wins on the HLC), and today a
fast-gossiping squatter can win a name it has no moral claim to. HLC-FWW resolves
the *race* deterministically; it does not adjudicate *legitimacy*. That's
acceptable for a conference-hall or neighborhood deployment (the failure is a name
collision, not a security breach) and it's explicitly a way-station.

The upgrade path is the same idea pushed one layer further: **web-of-trust name
arbitration** (bead `e21.5`), where a name binds to an identity and disputes
resolve by trust attestations between keys rather than by who gossiped first.
Identity is what promotes first-writer-wins from "first to *speak*" to "first
*legitimate* claimant." The addressing layer already lives on that principle; the
naming layer is walking toward it. Being clear about the seam is the point — a
decentralized system that pretends it has closed the Sybil problem is lying; one
that shows exactly which resources are key-bound and which are still TOFU is
giving you the threat model straight.

## 5. Gossip: epidemic replication, no coordinator

The CRDTs need a transport that spreads updates to everyone without a central
broker. That's **gossip** — epidemic broadcast over iroh's QUIC overlay
(`crdt/gossip.rs`, `crdt/sync.rs`). A node with news tells its neighbors; they
tell theirs; the update floods the swarm in log time. No node has the whole
picture or needs it.

Two properties make gossip trustworthy here rather than merely fast:

- **Anti-entropy.** Every node, once per interval, re-broadcasts the *full* maps
  it holds — not just deltas. This is the cheap insurance that turns "best-effort
  flood" into "eventually everyone converges": a late joiner, a node that missed a
  packet during a partition, a box that just rebooted — all catch up on the next
  tick with no pull protocol, no "please re-send what I missed" handshake, no
  anti-entropy Merkle-tree dance. The maps are small (§2), so re-sending
  everything is cheaper than the machinery to re-send only the diff. Convergence
  becomes a *standing property* of the tick, not an event you have to orchestrate.
- **Merge-on-rejoin with no election.** When a partition heals, the two sides
  simply gossip their states and merge (§2–3). There is no "primary" to resync
  against, no split-brain to reconcile by hand, because the merge is deterministic
  and symmetric. Two meshes that were never joined can fuse the same way, by
  linking at a single node. Rejoining is not a recovery procedure; it's the
  normal operation running after a gap.

## 6. Evolving the wire with no flag day

A system with no central authority also has no central *deploy*. Nodes run
whatever version their owner last shipped; the fleet is permanently mixed-version.
So the wire format has to tolerate a new node and an old node talking, forever,
with no coordinated upgrade.

The mechanism is deliberately boring and empirically verified (`crdt/gossip.rs`
`mixed_fleet` tests). Messages are a `postcard`-encoded enum; new message types
are **appended** to the enum so existing type tags never shift. An old node
receiving a new message type it doesn't recognize can't decode it — and the
receive loop treats a decode error as **log-and-skip**, not a crash. The new
capability is simply invisible to nodes that predate it, and visible to those that
don't, with no negotiation. A five-year-old box and a nightly build share the same
mesh. This is version tolerance as a property of the decoder, not a compatibility
matrix someone maintains.

## 7. The problem CRDTs *don't* solve — and our answer

Here is the subtle one, and the piece of genuinely new engineering worth the
DWeb crowd's attention. CRDTs are built on **monotonic truth**: facts that, once
true, stay mergeable forever. "Node X claimed this name at time T" is such a fact.
But a whole class of things a live network needs to know are *not* monotonic
truths — they're **fading observations**:

> Is node X *still here*?

Liveness is the opposite of a CRDT fact. It's true, then quietly stops being true,
with no event marking the transition — a dead node sends no "I have died" message;
it simply goes silent. You cannot merge your way to "X is gone," because absence
produces no data to merge. Bolt liveness onto the CRDT — say, re-stamp every
record's clock every few seconds so "the clock stopped moving" means "the owner
died" — and you've corrupted your durable, monotonic truth-store with a
high-frequency ephemeral signal, and (on the cheap flash of a $40 router) you're
rewriting persistent state to storage every few seconds forever, wearing out the
hardware to encode something that was never supposed to be persistent.

So we split the planes. This is the design in
[lane-staleness.md](../network-coordination/lane-staleness.md) (bead `e21.9`), and
the reframe is the whole trick: **liveness is not durable state, so it does not
live in the durable CRDT.** It rides its own **ephemeral** channel:

- Each node emits a tiny **liveness beacon** once per gossip tick — `(node_id,
  incarnation, counter)` — that is *never* merged into a book, *never* persisted,
  *never* relayed. It's authored fresh by the living origin about itself. Silence
  is the death signal; the beacon is the heartbeat that its absence contradicts.
- Receivers keep an **in-memory** map of when they last heard a *newer* beacon
  from each node, and judge staleness by their *own* local clock delta — never by
  any timestamp inside the beacon. That single decision makes the whole plane
  immune to clock skew: we trust remote clocks to *order writes* (the HLC, §3) but
  never to *measure liveness*.
- The beacon is deliberately **weaker than an HLC** — it orders nothing, so it
  sheds the wall clock. The one wrinkle, restart (a rebooted node resets its
  counter to zero and would look like a stale replay), is handled by the
  `incarnation` = boot time: a reboot yields a strictly greater incarnation, so
  the fresh node dominates its own history — with **zero persisted state**, read
  from the system clock at boot.

The payoff is that the durable CRDT goes back to storing only monotonic truth and
only writing to flash when a real fact changes, while liveness — the fading,
per-node, momentary thing — lives entirely in RAM where ephemeral things belong.
Names whose owner has gone silent stop resolving (no black-holing a phone at a
`wiki.mesh` that moved away) but stay in the book, so an owner's return silently
un-stales them. Deletion (`unpublish`) uses **tombstones** with bounded,
resurrection-safe garbage collection — safe precisely because a learned record is
never re-announced by anyone but its owner, so once the owner tombstones it,
nothing in the mesh can resurrect it and the tombstone can be dropped after a
bounded window rather than kept forever.

This is a small idea with a general shape, and it's the kind of thing the DWeb
toolbox is still short on: **separate the monotonic-truth plane (CRDT, persisted,
convergent) from the liveness plane (ephemeral, in-memory, receiver-timed), and
don't let either contaminate the other.** The beacon's `incarnation` is also the
forward-compatible seam to a full **gossip failure detector** (SWIM / phi-accrual,
bead `4hl`), which upgrades liveness from each node's local view to a
partition-robust distributed one — the next rung, deliberately deferred, thread
kept.

## 8. Why these guarantees, and why they're different on purpose

Put beside a conventional SaaS backend, this system looks like it's losing on
every axis a systems-design interview would score:

| | Centralized SaaS | Lightning Mesh |
|---|---|---|
| Consistency | linearizable (single writer) | strong *eventual* (CRDT merge) |
| Ordering | the DB transaction log | HLC total order, per-node computable |
| Identity / Sybil | accounts + admins + KYC | key-derived; TOFU→web-of-trust at the frontier |
| Coordination | a coordinator | none — gossip + deterministic merge |
| Partition behavior | unavailable (CP) | available, converges on heal (AP) |
| Deploy | one flag day | permanently mixed-version, decode-skip |

Every row is a *different choice*, and each one buys the same thing: **there is no
authority to capture.** No transaction log to subpoena, no account system to
compel an admin to ban you from, no coordinator to take offline to take the network
offline, no single vendor whose incentive gradient bends the system toward
extraction over time. The guarantees are weaker where weakness is the price of
having no throat to choke, and they are engineered to be *exactly as strong as
they can be* under that constraint — deterministic convergence, total order,
key-rooted identity, version tolerance.

That is what "egalitarian" means when it's load-bearing rather than decorative.
Every node runs identical software with no special roles. Ordering is decided by
mathematics that every node computes identically, not by a privilege one node
holds. Identity is a keypair anyone can generate, and the resources you can hold
are the ones your key entitles you to. The network doesn't have a center that's
been *hidden*; it has no center that *could* exist. The decentralized web is not
just centralized services with the logo filed off and a token bolted on — it's
systems whose **correctness does not route through anyone's authority**. Total
ordering and Sybil resistance are where that promise is either kept or quietly
broken, and they're where the real work is.

---

## Related

- [Philosophical outcomes of the architecture](philosophical-outcomes.md) — what
  these mechanisms *mean* for ownership, sovereignty, and reachability.
- [Why decentralized mesh networking](why-decentralized-mesh.md) — motivation and
  system comparisons.
- [Lane staleness design](../network-coordination/lane-staleness.md) — the
  ephemeral liveness plane (§7) in full.
- [Mesh naming](../network-coordination/mesh-naming.md) — owner-bound TOFU service
  names (§4) and the web-of-trust frontier.
- Source of record: `crates/mjolnir-mesh/src/crdt/` — `hlc.rs` (§3),
  `gossip.rs`/`sync.rs` (§5–6), `service.rs`/`service_apply.rs` (§4),
  `liveness.rs` (§7).
