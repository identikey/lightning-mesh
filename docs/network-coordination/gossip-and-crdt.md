# The Gossip Control Plane & CRDTs — a primer

**Status 2026-07-02:** subnet claims are wired and field-validated on the deployed
4-router fleet; the lease / DNS / service lanes described below are the service-mesh
phase (bead `e21`).

**Who this is for:** anyone showing up to mjolnir-mesh who wants to understand how
the nodes agree on shared facts (who owns which subnet, which device has which IP,
what services exist) **without a central server, a leader, or a database anyone has
to trust.** Read this before `network-architecture.md`, which goes deeper (the archived
`../archive/network-coordination/dhcp-crdt.md` has the full original lease design).

---

## TL;DR

The mesh has no boss. Every node is equal and any node can vanish, reappear, or be
cut off behind a dead radio link at any moment. So we can't keep shared state
(leases, DNS, service lists, subnet ownership) in one place — there *is* no one
place. Instead, each node keeps its **own copy**, nodes **gossip** their changes to
each other over iroh, and the data types are designed so that any two copies, fed
the same set of changes **in any order, with duplicates and gaps**, always end up
**identical**. Those self-reconciling data types are **CRDTs** (Conflict-free
Replicated Data Types). Gossip is the delivery mechanism; CRDTs are what make sloppy,
best-effort delivery safe.

---

## Why CRDTs? (the problem they solve for us)

The project's thesis is a **symmetric, non-authoritative** network: no root node, no
DHCP server, no DNS server, no coordinator. That's not an aesthetic choice — it's the
whole point. A designated authority is a single point of failure *and* a single point
of control, and a mesh thrown into a forest must keep working when any node dies.

But "no authority" collides with a hard fact: the nodes still need to **agree on
things**. Two routers must not hand the same `10.42.5.0/24` to their clients. A laptop
roaming from one node to another should keep its IP. A phone should be able to find a
printer attached three hops away. All of that is *shared state*, and normally you'd
reach for one of two tools — both of which we reject:

- **A central database** (one node holds the truth). That node is now the authority
  and the single point of failure. Rejected.
- **Locks / consensus** (Raft, Paxos — nodes vote before committing). These need a
  quorum to be reachable and round-trips to complete. Under radio churn and network
  partitions — our *normal* operating condition — they stall or block. Rejected.

CRDTs are the third way. A CRDT is a data structure whose **merge operation is
commutative, associative, and idempotent** — math-speak for: it doesn't matter what
order changes arrive in, whether some arrive twice, or whether a partition delivers
them in a lump an hour late. Every replica that has seen the same *set* of changes
computes the same answer, **with no coordination round-trip**. Conflicts are resolved
by a **deterministic local rule** every node applies identically.

> **The notebook analogy.** Imagine a group where everyone keeps their own notebook
> and, whenever they meet, tells the others what they wrote. If two people wrote
> conflicting entries, they follow a rule agreed in advance ("earliest timestamp
> wins") so both cross out the same one. No one is in charge, people can leave and
> come back, messages can be missed — yet given enough mingling, every notebook ends
> up identical. That's a CRDT, and the mingling is gossip.

This is what makes "runs on any hardware, no authority, heals after partition"
actually true instead of a slogan.

---

## The shared state: what we synchronize, and why

Four kinds of facts live in the CRDT. Each is a small serializable record
(`crates/mjolnir-mesh/src/crdt/`), keyed in its own namespace:

| Fact | Type / key | Why it must be shared | Status |
|---|---|---|---|
| **Subnet claims** | `SubnetClaim` @ `/subnets/{cidr}` (`subnet.rs`) | Stop two routers from claiming the **same client `/24`** at first boot. First-writer-wins. | **Wired today** |
| **DHCP leases** | `LeaseEntry` @ `/devices/{mac}` (`lease.rs`) | Every router knows every device's `mac → ip`, so a client keeps its address as it **roams between nodes** and no two nodes hand out the same IP. | Schema defined; wiring planned |
| **DNS** | `DnsEntry` @ `/dns/{hostname}` (`dns.rs`) | Derived from leases so a name like `laptop.mesh` **resolves anywhere** in the mesh, not just on its home router. | Schema defined; wiring planned |
| **Services** | `ServiceEntry` @ `/services/{name}` (`service.rs`) | mDNS-style announcements (`printer._ipp._tcp → ip:port + TXT`) so a phone on node A **discovers a printer on node D** across the mesh, even though mDNS multicast never crosses a routed hop. | Schema defined; wiring planned |

**Honest status:** the gossip *wire format* already carries all four (see
`GossipMessage` below), and the merge logic for subnet claims is live. The daemon's
apply loop currently merges **only** subnet claims; leases / DNS / services are
defined and round-trip-tested but not yet applied end-to-end. The remaining wiring is
tracked as a service-discovery work item — see the trackers in beads. This doc
describes the design; the table tells you what's load-bearing right now.

> **Note what is *not* in the CRDT: routes.** Babel computes and installs routes (see
> `babel-routing.md`). A `SubnetClaim` says "*I own this `/24`*" for boot-time
> coordination; it is **not** a routing table. Keeping these separate is deliberate —
> the CRDT is for *agreement*, Babel is for *reachability*.

---

## How it works (the mechanics)

### 1. Telling time without a clock you can trust — the HLC

Cheap routers boot with wrong clocks and never perfectly agree. To order events we
use a **Hybrid Logical Clock** (`hlc.rs`): a triple of `(wall_clock_ms, counter,
node_id)` compared in that order. The counter advances when wall-clock readings tie,
and `node_id` is the final tiebreaker. Lower HLC = "earlier writer." It gives us a
**total order that's good enough for conflict resolution** without NTP or synchronized
clocks.

### 2. The merge rule — deterministic, local, identical everywhere

`merge.rs` is a **pure function**: given the local entry (if any) and an incoming one,
it returns one of:

- `Inserted` — we'd never seen this key; take it.
- `Updated` — same owner, newer HLC; refresh.
- `Unchanged` — duplicate or older; ignore (this is why **re-delivery is harmless**).
- `Conflict { winner, loser }` — two different owners claimed the same thing →
  **first-writer-wins** on HLC (`resolve_subnet_conflict`).

The key property, asserted by the tests in `merge.rs`: **two nodes seeing the same
pair of records reach the same verdict regardless of argument order.** No vote, no
lock — just arithmetic both sides perform identically. That's the entire trick that
lets gossip be unreliable.

### 3. The gossip layer — best-effort, and that's fine

Changes travel as a `GossipMessage` enum (`gossip.rs`) — `LeaseUpdate`,
`LeaseRelease`, `DnsUpdate`, `ServiceUpdate`, `SubnetClaimUpdate`,
`SubnetClaimRelease` — serialized with **postcard** (a compact binary format). All
nodes subscribe to **one well-known gossip topic** (derived from a fixed string,
`blake3("mjolnir/mesh/crdt/v0")`), bootstrapped from the node roster.

Delivery is **best-effort on purpose**. The receive loop (`GossipSync::run` in
`sync.rs`) does `recv → decode → apply`, and:

- a **malformed** payload is logged and skipped — it never kills the loop;
- a **lost** message just means you converge later, when the next update or
  anti-entropy pass carries the same fact;
- a **duplicate** is absorbed by the merge function as `Unchanged`.

Because the CRDT merge tolerates all three, the transport is allowed to be dumb. We
don't pay for reliability we don't need.

### 4. A clean seam: the substrate stays transport-agnostic

`sync.rs` defines `GossipTransport` (a tiny `broadcast(bytes)` / `recv() -> bytes`
trait) and `GossipSync` (the postcard framing + dispatch loop) — and it is
deliberately **iroh-free**. The library knows only "raw bytes in, raw bytes out"; the
concrete **iroh-gossip** implementation lives in the daemon binary. This mirrors the
`DatagramConn` pattern used for the data plane. The payoff: the CRDT logic is unit-
testable with an in-memory mock transport (it is), and the substrate isn't welded to
one network stack.

---

## Why iroh carries the gossip

Gossip needs a way for node A to actually *send bytes to* node B. We use
[iroh](https://iroh.computer) for that, and it's a strong fit for four reasons:

1. **Identity = a public key.** Every node is an Ed25519 keypair; its **node id is its
   public key**. You dial a *node*, not an IP. That identity is stable across reboots,
   address changes, and which radio link it's currently on — exactly what a CRDT's
   `node_id` and a roster want.
2. **End-to-end encryption, for free.** Every iroh connection is mutually
   authenticated and encrypted. The control plane is private and tamper-evident with
   no extra work. (Contrast: 802.11s SAE encrypts each **radio hop**, so an
   intermediate forwarding node sees plaintext — fine for a trusted single hop, *not*
   end-to-end. For anything crossing untrusted multi-hop, iroh's e2e is the property
   you want. See the `iroh-use-policy` memory / `auu` notes.)
3. **Dial *anywhere*, from one endpoint.** The same `iroh::Endpoint` reaches a peer
   **LAN-direct** when it's a few radio hops away (no relay, no internet — and because
   a node's backhaul address is *derivable* from its node id, often with no lookup at
   all) **or across the internet** via hole-punching / relays when it isn't. Some
   services we need to reach won't be on the local mesh; iroh makes "local peer" and
   "remote peer" the same call.
4. **It rides our own routed underlay.** Same-site, the gossip QUIC travels over the
   Babel-routed `10.254.x` backhaul — the mesh carries its own control plane; no
   dependency on the public internet for the network to coordinate.

So iroh gives the gossip layer **authenticated identity + encryption + reach-anywhere
transport** as a package, while the `GossipTransport` seam keeps that dependency at
arm's length from the data types.

---

## End-to-end walk-through: a subnet claim

1. Router **C** boots, picks a free client block `10.42.7.0/24`, and writes a
   `SubnetClaim { cidr, owner_node_id: C, claimed_at: HLC }` locally.
2. C publishes `SubnetClaimUpdate` → postcard bytes → `GossipTransport::broadcast` →
   iroh-gossip topic → out over the encrypted backhaul.
3. Each peer's `GossipSync::run` receives it, decodes it, and calls
   `merge_subnet_claim` against its own copy. New key → `Inserted`. Everyone now knows
   C owns that `/24`.
4. Suppose router **D**, partitioned at the time, *also* claimed `10.42.7.0/24`. When
   the partition heals and the two updates meet, **both** C and D run the identical
   first-writer-wins rule on the HLCs and **independently pick the same winner**. The
   loser sees `Conflict`, backs off, and re-claims a different block. No negotiation
   message is ever exchanged — they just compute the same answer.

That convergence-without-coordination, surviving a partition, is the whole reason the
CRDT machinery exists.

---

## Where to look next

**Code** (`crates/mjolnir-mesh/src/crdt/`): `hlc.rs` (clocks), `merge.rs` (the rule),
`subnet.rs` / `lease.rs` / `dns.rs` / `service.rs` (the records), `gossip.rs` (wire
enum), `sync.rs` (transport seam + dispatch loop). Roster/bootstrap: `roster.rs`. The
iroh-gossip transport impl and apply loop live in `src/bin/mjolnir-meshd.rs`.

**Deeper docs**: `../archive/network-coordination/dhcp-crdt.md` (the lease/DHCP design
in full — archived; design reference for `e21`),
`network-architecture.md` (how this sits under iroh + Babel), `babel-routing.md` (why
routes are *not* in the CRDT), `radio-backhaul-and-discovery.md` (how nodes find each
other so gossip can flow multi-hop), `collective-coordination-protocol.md` and
`membership-enrollment.md`.
