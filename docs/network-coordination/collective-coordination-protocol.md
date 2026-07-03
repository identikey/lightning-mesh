# Collective Coordination: Governing Address-Space Allocation

**Status:** Draft / exploration (substrate-layer governance) | **Date:** 2026-06-10

The subnet CRDT (`/subnets/{cidr}`, see `gossip-and-crdt.md` and `babel-routing.md`) makes the mesh
IPv4 space (`DEFAULT_MESH_SPACE = 10.42.0.0/16`, 65 534 usable; `alloc.rs`) a *shared commons* with
no central authority. Allocation works cleanly while the space is mostly empty. This note is about
the part that needs a defined collective behaviour: **what happens as the space fills up.**

Today this is not urgent — four routers exist and utilisation is ~0%. But it must be sound before a
production fleet, so this captures the governance model and, importantly, separates the **cheap
things worth doing before production** from the **heavier governance that can wait**.

This is **substrate-layer** governance (IP plumbing). Social groups / shared encryption are an
application-layer concern and are deliberately *not* in this repo (see issue `mjolnir-mesh-6t7`,
parked).

---

## 1. The gap: `pick_subnet` returns `None`, "the caller decides"

`alloc::pick_subnet(node_id, claimed, base, target_prefix_len)` picks a deterministic preferred slot
(blake3 of `node_id`) and walks candidates, rejecting any that overlap a claim of *any* size. On
exhaustion it returns `None`, and its own doc comment defers the policy:

> "callers (typically a daemon UI) decide whether to **widen the search, shrink the request, or
> fail loud**."

That deferred decision is the subject of this document. In a non-authoritarian commons there is no
one to break ties, so the response to `None` must be either *deterministic* or *negotiated*.

## 2. "Full" is fragmentation, not a count

Because CIDR blocks nest, a request fails when **no contiguous, aligned slot of the requested size is
free** — which can happen with plenty of total free addresses. Example: a `/16` peppered with
scattered `/30` claims can have >90% of addresses free yet be unable to satisfy a `/22` (which needs
four aligned, contiguous `/24`s clear). So the commons has two distinct pressure signals:

- **Utilisation** — `Σ total_addresses(claim) / total_addresses(base)`.
- **Largest free slot** — the biggest prefix `pick_subnet` could satisfy *right now*. This is the
  fragmentation signal, and it fails first.

Both should be derivable from the `/subnets/` CRDT and gossiped so every node shares the picture.

### Scale reference (`/16` base)

| Per-site claim | Usable hosts | Max such sites in /16 |
|---|---|---|
| `/22` | 1 022 | 64 |
| `/24` | 254 | 256 |
| `/30` | 2 | 16 384 |

Four routers today → effectively empty. The `/24`-per-site ceiling (256 sites) is the figure to
watch for a growing fleet; widening the pool (§3, response B) raises it dramatically.

## 3. Responses to exhaustion

Formalising the allocator's "widen / shrink / fail loud," ordered cheapest-first:

- **A. Shrink the request (auto-downgrade). — SHIPPED.** On `None`, retry with
  `bump_smaller_subnet` until a slot is found or the `/30` floor is hit. Fully
  deterministic, no coordination, no authority. Implemented as
  `alloc::pick_subnet_or_smaller`, called by the daemon's `claim_and_publish` — a node
  no longer hard-fails just because its *preferred* size was unavailable.
- **B. Widen the pool.** `base` is configurable (`DEFAULT_MESH_SPACE` is only the default). A
  production deployment can run `10.0.0.0/8` (≈16 M addresses) or add a second space. This is the
  real scalability lever and is a pure config decision — no protocol needed.
- **C. Reclaim (negotiated).** Recover stale or over-broad claims (§4). Needs coordination, so it is
  the heaviest response and the one to defer until fleet size makes it load-bearing.
- **D. Fail loud / escalate.** Surface to the operator TUI (and/or a human-decision flag) when A–C
  cannot satisfy a genuine need. Never silently mis-allocate.

## 4. Reclamation without an authority (deferred)

Two recoverable conditions, both resolvable without a central decider:

- **Stale claims.** A claim whose owner is no longer live. Liveness is already tracked (Iroh
  connection state + Babel hello/IHU, `network-architecture.md`). Reclaim after a grace period +
  cooldown to tolerate churn.
- **Over-broad claims.** A `/22` serving three devices is hoarding commons space (the original
  "1024 clients but it's just me" case). The collective gossips a *renarrow request*; the holder
  yields or justifies. This needs an etiquette/incentive, not just a mechanism.

Both can be made non-authoritarian by reusing the **endorsement primitive** from
`membership-enrollment.md`: a reclamation/renarrow proposal is countersigned by other members, with
deterministic tie-breaks (the same blake3 preference already in `pick_subnet`) and a cooldown so no
single node acts unilaterally.

## 5. Joining & merging address spaces (federation)

A related path to pressure: two meshes that formed independently each own a coherent space; when
they meet, claims may overlap or fragment. Merging should be **consensual and configurable**:

- A client-level **orientation** — e.g. `join_shared_space: open | ask | never`.
- Later, explicit **per-network opt-in** rather than a blanket policy.
- A merge is a *negotiation* proposed and countersigned by members of both spaces (endorsement
  primitive again), not a unilateral claim.

## 6. Recommendation: cheap now, heavy later

Matching effort to the fact that the space is empty but production is near:

**Do before production (cheap, prevents fatal allocation failures):**
1. ~~**Auto-downgrade on `None`** (response A)~~ — **DONE** (`alloc::pick_subnet_or_smaller` via `claim_and_publish`); allocation degrades gracefully instead of hard-failing.
2. **Pool config knob** (response B) — let a deployment widen to `/8`; document the choice.
3. **Pressure observability** — expose utilisation + largest-free-slot (in the TUI / metrics).

**Defer until the fleet is large (dozens+ of sites):**
4. Stale-claim reclamation (response C / §4).
5. Over-broad renarrowing and countersigned reclamation/merge proposals (§4–5).

## 7. Relationship to existing work

- Builds on: variable-prefix allocation (`f48cece`, `alloc.rs`), the `/subnets/{cidr}` CRDT ledger,
  subnet-claim cooldown, Babel redistribution (`ge {prefix} le {prefix}`).
- Reuses identity/attestation: reclamation, renarrow, and merge proposals are signed/countersigned
  with the same Ed25519-identity + endorsement model as enrollment (`membership-enrollment.md`).

## 8. Open questions

1. What is the unit of "joining a space" — a node, a site, or a whole mesh?
2. Does reclamation/merge need a quorum, or is pairwise countersignature enough?
3. What grace period + cooldown makes stale-claim reclamation safe against transient churn?
4. Should the production default just be `/8` from day one, sidestepping `/16` exhaustion entirely?

---

## References

- Allocator & pool: `crates/mjolnir-mesh/src/alloc.rs`
- Subnet CRDT: `gossip-and-crdt.md`, `babel-routing.md` (original lease design archived at `../archive/network-coordination/dhcp-crdt.md`)
- Liveness & claim cooldown: `network-architecture.md`
- Identity & endorsement primitive: `membership-enrollment.md`
- Application-layer groups (out of scope here, parked): issue `mjolnir-mesh-6t7`
