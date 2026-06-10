# Guilds: Cryptographic Group Overlay (Vision)

**Status:** Vision — deferred, sequenced after substrate (NOT yet planned) | **Date:** 2026-06-10

Once the **substrate** is settled — every node has an IP, a basic mDNS-style name, and an Iroh +
HTTP transport — we can build our own sub-networks as **cryptographic groups ("guilds")** on top of
it, instead of re-deriving identity and trust at every legacy layer (DHCP → DNS → TLS → app auth):
the "strange loop" of protocols heaped on one another.

This is a direction note, not an implementation plan. Its job is to fix the *altitude* and the
*principle* so the work is sequenced correctly.

---

## 1. Two strata

| Stratum | Owns | Where it lives |
|---|---|---|
| **Substrate** | IP allocation, naming, Iroh/HTTP transport, address-space governance | `mjolnir-mesh` (router lib) + `collective-coordination-protocol.md` |
| **Overlay (guilds)** | Group membership, shared encryption, shared comms space | **Above** the router lib — `mjolnir-node` or a shared `identity/groups` crate |

The router routes ciphertext and nothing more. **No group crypto runs on the RouterBOARD.**

## 2. The principle: key on identity, not location

The legacy stack re-derives trust at every layer because none can name a stable cryptographic
identity. We already fixed the bottom: an **Iroh `EndpointId` is the identity; signatures are
attestation** (`membership-enrollment.md`). Guilds continue that line. The one discipline that keeps
this from becoming a *new* strange loop:

> A guild's membership and encryption are keyed on **cryptographic identity, never on IP/subnet.**

IP stays pure plumbing; all identity / membership / encryption live in one coherent layer.

## 3. What a guild is

A guild is **a set of attested identities + a shared encryption context + a shared comms space** —
"almost like their own certificate." Concretely:

- **Membership** — the web-of-trust / endorsement model from `membership-enrollment.md`, scoped to
  the guild.
- **Comms space** — a guild-scoped gossip topic (the machinery already exists for rooms).
- **Shared encryption** — a group key under which guild content is encrypted (see §5).

### Through-line: Room → Guild

`mjolnir-node` already has a `Room` (ephemeral, open, a gossip topic seeded by a `MeshTicket`). A
**guild is a persistent Room with attested membership and a group key** — same gossip-topic
machinery + the enrollment layer + group encryption. Rooms are the unattested ancestor of guilds.

## 4. Layer placement & footprint

Guilds encrypt *content* among endpoints, so they belong at the endpoint/application layer, not in
the router lib. This also dodges a hard constraint: lattice proxy-recryption on a MikroTik
RouterBOARD is a non-starter. If guild crypto lives at the node layer, the router never touches it.

## 5. Recrypt as the group-encryption layer

`~/work/IdentiKey/recrypt` (proxy recryption) is the natural mechanism, and the model lines up:

- **First-class `Group` abstraction** already designed (the "Signal meets Dropbox" group-sharing
  plan): batch add/remove of members, canonical group keys, per-group atomicity.
- **The right cost curve for a shared space:** revocable sharing that is **O(1) in bulk data** (one
  ciphertext regardless of group size) and O(N) only in small recryption keys — add/remove a member
  without re-encrypting content.
- **Threshold-signed decryption capabilities** — resonates directly with the
  countersignature/threshold web-of-trust in `membership-enrollment.md`. Both projects are
  independently converging on threshold/multi-sig trust.
- **Pluggable PRE backends:** lattice/OpenFHE (post-quantum, heavy) vs **EC/classical (light)**.
  Anything mesh-adjacent should target the **EC backend**.

**Caveat — Recrypt is an early-stage sibling, not a dependency yet.** Its Rust crates are largely
skeletal (the OpenFHE FFI is the big unbuilt lift; the Python prototype is the reference). The
realistic plan: **align the model now, integrate the code when both are ready**, and let the mesh's
group needs inform Recrypt's EC-backend API while it is still malleable. Do not take a hard
dependency today.

## 6. Sequencing

1. **Now** — identity & attestation (`membership-enrollment.md`): the load-bearing first step of
   this vision. Nothing here is wasted.
2. **Near** — substrate governance (`collective-coordination-protocol.md`).
3. **Later** — guilds overlay, **after** the substrate is solid, co-timed with Recrypt's release.

## 7. Open decisions

1. **Home for guilds** — `mjolnir-node` vs a new shared `identity/groups` crate consumed by both
   node and mesh.
2. **Recrypt integration boundary** — the EC-backend API surface; what the mesh needs vs what
   Recrypt provides.
3. **Guild vs mesh membership** — is a guild always a subset of mesh members? Can a guild span
   multiple meshes? (Identity-keying makes spanning natural.)
4. **Group-key rotation / revocation** semantics, and how they reconcile with the membership
   revocation epoch model.

---

## References

- Identity / membership primitive: `membership-enrollment.md`
- Substrate governance: `collective-coordination-protocol.md`
- Recrypt: `~/work/IdentiKey/recrypt` — `docs/architecture.md` (group-sharing plan),
  `docs/threat-model.md` (revocable group sharing, O(1) bulk), `docs/pre-backend-traits.md`
  (backend hierarchy)
