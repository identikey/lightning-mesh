# Collective Coordination: Address-Space Arbitration & Federation

**Status:** Draft / exploration (substrate-layer governance) | **Date:** 2026-06-10

The DHCP/subnet CRDT (`/subnets/{cidr}`, see `dhcp-crdt.md` and `babel-routing.md`) makes IPv4
address space a *shared commons* with no central authority. That works while claims are disjoint.
This note is about the cases where it does not — **deadlock, over-allocation, and joining/merging
independently-formed address spaces** — and how a non-authoritarian collective decides in those
cases.

This is **substrate-layer** governance (IP plumbing). It is deliberately *not* where social groups
or shared encryption live — that is the overlay, see `guilds-overlay-vision.md`.

---

## 1. The problem: a commons needs an arbitration story

In an authoritarian network the authority decides allocation and everyone complies. Our model has
no authority — allocation is coordinated by CRDT claims with a cooldown (`network-architecture.md`
§subnet claim). That is the right default, but a commons still needs a defined behaviour for when
two participants' interests collide and there is no one to break the tie.

## 2. Scenarios

**a. Over-allocation under variable prefixes.** Variable-prefix allocation (commit `f48cece`) lets
a site claim a prefix sized to its needs. But a node that claims a generous block ("my LAN supports
1024 clients") when it is in fact a single device is hoarding commons space — artificial scarcity.
The collective may need a way to ask it to relinquish or renarrow.

**b. Joining / merging address spaces.** Two meshes that formed independently each own a coherent
space. When they meet — or a node wants to join another's space — their claims may overlap or
fragment. Merging is a *negotiation*, not a unilateral claim.

## 3. Client orientation (opt-in)

Joining someone else's address space should be **consensual and configurable**:

- A client-level **orientation** — e.g. `join_shared_space: open | ask | never` — expressing
  default openness to joining a shared space.
- Later, an interface to **opt in to specific networks** explicitly (per-network consent) rather
  than a blanket policy.

This mirrors the web-of-trust stance from `membership-enrollment.md`: participation is a local
decision, not something imposed.

## 4. Arbitration approaches (sketch — undecided)

No authority means tie-breaks must be either *deterministic* or *negotiated*:

- **Deterministic tie-break** — already used for /31 link addressing (blake3 of the sorted peer
  pair, `tun/link.rs`). Cheap, no round-trip, but cannot weigh need.
- **Cooldown + late-join yield** — already used for subnet claims; could extend to "newer or
  over-broad claim yields to established narrow need."
- **Negotiated renarrowing / backpressure** — a node observing over-allocation gossips a request to
  renarrow; the holder responds. Needs an etiquette/incentive, not just a mechanism.
- **Countersigned merge proposals** — joining/merging two spaces is proposed and countersigned by
  members of both, reusing the endorsement primitive from `membership-enrollment.md`.

## 5. Relationship to existing work

- Builds on: variable-prefix subnet allocation (`f48cece`), subnet-claim cooldown, the
  `/subnets/{cidr}` CRDT ledger, Babel redistribution.
- Reuses identity/attestation: merge proposals and consent can be signed/countersigned with the
  same Ed25519-identity + endorsement model as enrollment.

## 6. Open questions

1. Is over-allocation a real problem in practice, or does variable-prefix + abundant RFC1918 space
   make it moot until federation? (Likely defer aggressive reclamation.)
2. What is the unit of "joining a space" — a node, a site, or a whole mesh?
3. Does merge negotiation need a quorum, or is pairwise countersignature enough?

---

## References

- Subnet / DHCP CRDT: `dhcp-crdt.md`, `babel-routing.md`
- Identity & endorsement primitive: `membership-enrollment.md`
- Overlay groups (explicitly out of scope here): `guilds-overlay-vision.md`
