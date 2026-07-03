# Membership Enrollment (DPP-style)

**Status:** Design proposal (Phase 2 of membership control) | **Date:** 2026-06-10

This document sketches a device-onboarding scheme for mesh routers, modelled on
**Wi-Fi Easy Connect / DPP** (Device Provisioning Protocol) but implemented entirely at
the Iroh/daemon layer rather than at 802.11. It replaces and elaborates the "Phase 2:
Membership CRDT" paragraph in `network-architecture.md` §Security.

The original motivating constraint was MikroTik RouterOS hardware whose wireless stack
does not support real DPP/hostapd; that fleet is retired (2026-07-02, all-OpenWrt now),
but the rationale stands: by moving the *design* up to Layer 3, we get DPP's
scan-a-code-and-trust UX **independent of the radio firmware**. We are provisioning
*mesh membership*, not Wi-Fi credentials.

---

## 1. Why this maps cleanly

Two existing facts do most of the work:

1. **An Iroh `EndpointId` is an Ed25519 public key.** DPP's entire "bootstrapping" phase
   exists to get a device's public key into the configurator out-of-band (the QR code).
   We get that key for free — it *is* the node's identity (`mesh.rs` / `ticket.rs`).
2. **We already have a QR-shaped artifact.** `MeshTicket` (`name@base32(postcard(...))`,
   see `ticket.rs`) is the same envelope we reuse for an enrollment ticket — carrying the
   new node's *own* address instead of a room's bootstrap peers.

The simplification over real DPP: a DPP Connector must carry a separate `netAccessKey`
because the Wi-Fi link layer doesn't know who you are. **We don't need that** — the Iroh
QUIC/TLS peer identity already equals the granted subject, so "prove you hold the granted
key" is automatic on every connection.

---

## 2. Concept mapping

| DPP / Easy Connect | mjolnir-mesh equivalent |
|---|---|
| Enrollee bootstrap public key | New router's Iroh `EndpointId` (Ed25519) |
| QR code (out-of-band bootstrap) | `EnrollmentTicket` = `EndpointId` + `EndpointAddr`s, base32 (reuse `MeshTicket` envelope) |
| Configurator | Any existing member (the **enroller**) — or a phone app delegated to one |
| C-sign-key (configurator signing key) | **Each device's own Iroh key** — every device is its own trust root (see §4) |
| Connector (signed, offline-verifiable credential) | A signed **endorsement** in the `/members/{subject}` CRDT record |
| DPP Authentication (mutual auth via bootstrap keys) | Enroller dials the new node *by the exact `EndpointId` from the QR* over an enrollment ALPN → Iroh TLS proves key possession, no MITM |
| DPP Configuration (hand over SSID/PSK) | Hand over the gossip topic key + bootstrap addrs over that same connection |
| DPP Introduction (peers validate Connectors) | Peers evaluate `/members/{sender}` endorsements against **local trust policy** before accepting that sender's gossip |

---

## 3. Artifacts

```rust
/// QR payload the new node shows on first boot. Same envelope as MeshTicket.
struct EnrollmentTicket {
    subject: EndpointId,          // new node's pubkey == its identity
    addrs:   Vec<EndpointAddr>,   // so the enroller can dial it directly
    // optional: nonce + operator-facing label ("kitchen-AP")
}

/// CRDT record at /members/{subject}. A grow-only set of endorsements.
/// Trust is NOT a property of the record — it is evaluated per-device (see §4).
struct MembershipRecord {
    subject:      EndpointId,
    endorsements: GrowOnlySet<Endorsement>,  // any member may append (countersign)
}

/// One member vouching for one subject. The DPP "Connector" analog.
struct Endorsement {
    issuer:    EndpointId,
    subject:   EndpointId,
    issued_at: u64,           // pass time in; never call Date::now in CRDT logic
    epoch:     u64,           // monotonic; revoke-beats-readd (see §5)
    caps:      Caps,          // optional role/scope (full peer, edge-only, …)
    sig:       Signature,     // issuer.sign(canonical(subject, issued_at, epoch, caps))
}

/// Signed revocation. Gossips, but each device's local blacklist always wins for itself.
struct Revocation {
    subject: EndpointId,
    issuer:  EndpointId,
    epoch:   u64,             // must exceed the endorsement epoch it cancels
    sig:     Signature,
}
```

---

## 4. Trust model — per-device roots + countersignatures

This is a **web of trust**, not a hierarchy. There is no single offline mesh-root CA.

- **Each device holds its own signing key** (its Iroh identity) and is its own trust anchor.
  A device decides, locally, who it will forward for.
- **Local policy lives on the device, not in the CRDT:**

  ```text
  allow:     Set<EndpointId>   // directly trusted; seeded by a QR scan or operator action
  block:     Set<EndpointId>   // never accept — overrides everything, including endorsements
  threshold: K                 // accept a subject if >= K trusted members endorse it
  ```

- **Countersignatures = "other routers vouch for good users."** Any member can append an
  `Endorsement` to a subject's record. A subject gains acceptance as members a given device
  already trusts endorse it.
- **Bootstrapping direct trust:** scanning a new node's QR on device *D* causes *D* to add the
  subject to its own `allow` set (direct trust) and append an endorsement. Other devices that
  trust *D* — or that reach `threshold` trusted endorsers — then accept the subject too.

**Acceptance check** (evaluated locally, on receiving gossip from sender `S`):

```text
if S in block:                          reject
if S in allow:                          accept
endorsers = record(S).endorsements
    .filter(e => verify(e.sig)
              && e.issuer in (allow ∪ already_accepted)
              && e.issuer not in block)
accept  iff  endorsers.count >= threshold
```

The critical property — identical to DPP's Connector — is that this check is **offline**:
a peer validates a node purely from signed CRDT records, without contacting the enroller.
The QR (out-of-band) is what bootstraps trust in the key; everything after chains to it
cryptographically.

### Trade-offs of the web-of-trust choice

- **No key-custody single point of failure.** Nothing to keep in a vault; losing one device
  does not compromise the mesh's root of trust.
- **Trust is subjective.** "Is X a member?" is answered *per router*. Device A may accept X
  while device B does not (e.g. B has X blocked, or doesn't trust X's endorsers). For a mesh
  this is arguably a feature — each router governs who it forwards for — but it means there is
  no single global membership answer. Tooling/diagnostics must report membership relative to a
  viewpoint.
- **Transitive-trust blast radius** is bounded by policy: the strictest setting counts only
  directly-`allow`ed endorsers (depth 1); looser settings count any already-accepted member
  (web-of-trust transitivity). Start strict.

---

## 5. Revocation & blacklist

- **Local blacklist is immediate and absolute** for the device that sets it — no propagation
  required, `block` overrides all endorsements.
- **Signed `Revocation` records gossip** so other devices can honor them. A device applies a
  revocation if it trusts the issuer (same policy as endorsements); otherwise advisory.
- **Revoke must beat re-add.** Do not rely on wall-clock ordering. A revocation at `epoch = N`
  permanently dominates any endorsement with `epoch <= N`; legitimate re-admission requires a
  fresh endorsement at `epoch > N`, which only a valid issuer can sign. This collapses the
  conflict into signature-checkable terms rather than clock-checkable ones.
- CRDT semantics: endorsements are **add-wins** (grow-only set); revocation is **remove-wins**
  gated by `epoch`.

---

## 6. Enrollment flow

```
New router (Enrollee)            Operator + Enroller (a Member)         Mesh CRDT
─────────────────────           ──────────────────────────────        ─────────
1. boot, gen Iroh key
2. show EnrollmentTicket QR ───── scan ──►
                                3. add subject to local `allow`
                                4. dial Enrollee by EndpointId
                                   over `mjolnir/enroll/0` ALPN
   ◄═══ Iroh QUIC/TLS handshake ═══►   (DPP "Authentication": TLS proves
                                        Enrollee holds the QR'd key)
                                5. sign Endorsement{subject=enrollee}
                                6. append to /members/{enrollee} ──────► endorsement replicates
                                7. send over the enroll conn:
                                   { gossip_topic_key, bootstrap addrs } ── DPP "Configuration"
8. join gossip with topic_key
9. peers receive enrollee's gossip ─► evaluate /members/{enrollee}
   endorsements vs local policy ─► accept once threshold met.
(other members may countersign later to raise the enrollee above
 more devices' thresholds.)
```

Step 4's dial-by-exact-EndpointId is the DPP Authentication phase for free: because the QR
delivered the key out-of-band, Iroh's TLS guarantees the enroller is talking to the holder of
that key — no MITM.

---

## 7. Reuse vs. new

- **Reuse:** the `MeshTicket` base32/postcard envelope (clone into `EnrollmentTicket`);
  `EndpointId` as the public key; the `/members/{node_id}` namespace; the
  `address_lookup.add_endpoint_info()` seeding pattern (`mesh.rs:93`) so the enroller can dial
  back from the QR's addrs.
- **New:** an `mjolnir/enroll/0` ALPN handler (the direct enroller↔enrollee channel);
  endorsement/revocation sign + verify; local trust policy (`allow`/`block`/`threshold`)
  storage and evaluation; the gossip-receive hook that gates on `/members/{sender}`.

**Signing API (confirmed, iroh 0.96.1 — `iroh-base/src/key.rs`):** `EndpointId` is a type
alias for `PublicKey`, so signature verification is callable directly on a node's identity:

```rust
use iroh::{SecretKey, EndpointId, Signature, SignatureError};

let sig: Signature = secret_key.sign(canonical_bytes);            // SecretKey::sign(&[u8]) -> Signature
issuer_id.verify(canonical_bytes, &sig)?;                          // EndpointId(=PublicKey)::verify -> Result<(), SignatureError>
```

`Signature` implements `serde::{Serialize, Deserialize}` (a fixed-length byte tuple), so it
serializes into the `postcard` CRDT record with no extra plumbing.

Relationship to Phase 1 (PSK topic gating, `network-architecture.md` §Security): the
enrollment connection is exactly how the PSK-derived topic key is delivered securely, so this
layers on top of Phase 1. Once grant-gating is enforced on the gossip-receive path, the PSK
becomes optional.

---

## 8. Open items

1. **Threshold default `K`** and whether `caps` (role/scope) ship in v1 or are deferred.
2. **Where the enroller runs** — on a router, or a delegated phone app that holds a key the
   routers `allow`.
3. ~~Confirm iroh 0.96 signing API~~ — **resolved** (see §7): `SecretKey::sign` /
   `EndpointId::verify` exist in iroh-base 0.96.1 and `Signature` is serde-serializable.
4. **Revocation propagation policy** — honor revocations from any trusted issuer, or require a
   threshold of revokers (symmetry with endorsement threshold).

---

## References

- Membership control roadmap: `network-architecture.md` §Security → Future Work
- Identity & gossip: `crates/mjolnir-node/src/mesh.rs`
- Ticket envelope (reused for `EnrollmentTicket`): `crates/mjolnir-node/src/ticket.rs`
- Wi-Fi Easy Connect / DPP: Wi-Fi Alliance, <https://www.wi-fi.org/access>
