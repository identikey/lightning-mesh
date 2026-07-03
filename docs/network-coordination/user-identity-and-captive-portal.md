# User Identity & the Captive Portal (IdentiKey on the Mesh)

**Status:** Design spec — decided at the principle level, build sequenced after `0yb`/`e21`
**Bead:** `mjolnir-mesh-rp9` | **Date:** 2026-07-03
**Companion docs:** [membership-enrollment](membership-enrollment.md) (node identity, bead `met`),
[ipv6-addressing-decision](ipv6-addressing-decision.md) (identity lives in keys; IP is access plumbing),
[philosophical-outcomes](../vision/philosophical-outcomes.md) §5 (the network as a projection of keys)

This spec extends identity-by-key from **nodes** to **people**. It is the design
that makes "the network is a projection of a set of keys" user-facing: membership,
service access, and publish rights answered by keys and webs of trust — never by
MAC filters, shared passwords, or a server the protocol depends on.

---

## 1. Governing principles (decided 2026-07-03)

1. **The net is open.** Basic connectivity is plug-and-play and permissionless.
   Identity *unlocks more*; it never gates the on-ramp. Anonymous use is a
   supported mode, not a degraded one.
2. **The protocol verifies only signatures.** No component of the mesh ever
   verifies a server, consults an issuer, or phones home. Every trust decision
   is a stateless, offline check of a signature chain — the same property the
   Papyrus SP-02 spike arrived at, and the same shape the `met` node
   web-of-trust already uses.
3. **Custody is a spectrum the user chooses.** Where the private key lives is a
   user decision on a sovereignty gradient (§3), from "no key at all" to
   "hardware-backed key I alone hold." Services see one uniform thing — a key
   and valid signatures — regardless of which rung the user stands on. Users
   never have to leave their comfort zone unless they want the benefits.
4. **No custom software required.** Every rung below full self-custody works
   with a stock browser. The IdentiKey app is the *best* experience, not the
   *required* one (it also doesn't fully exist yet).
5. **One trust machinery.** User identity reuses the `met` web-of-trust CRDT
   shapes (endorsements, revocations, per-device allow/block/threshold). Users
   are one more kind of subject, not a second system.

## 2. Identity model — the Papyrus/identikey-core attestation chain

Adopted from Papyrus (SP-02 spike + collaboration PRD FR1–FR7), which is being
ported into `identikey-core` on its own track:

- A **user identity** is an Ed25519 keypair (the IdentiKey root).
- A **device** holds its own keypair (iroh `EndpointId` for mesh-speaking
  devices; a portal-session key for legacy devices, §4).
- The identity key signs an **attestation over the device key**:
  `sig(identity, canonical(device_pubkey, issued_at, epoch, caps))` — a
  verifiable, offline-checkable link from *person* to *device*.
- Verification anywhere in the mesh is a stateless signature check. No RP
  server, no JWKS fetch, no issuer contact. (WebAuthn/passkeys were evaluated
  and rejected for exactly this reason: a passkey is bound to a DNS-domain
  Relying-Party ID and platform-vendor infrastructure — structurally
  centralized. See Papyrus `docs/spikes/SP-02-webauthn-tauri-passkey/report.md`.)
- Growth path (Papyrus FR30): per-device keys HD-derived from the identity
  root, so multi-device is a derivation, not a re-enrollment.

**CRDT shape.** A `/users/{identity_pubkey}` record mirrors
`/members/{subject}`: a grow-only set of device attestations plus
endorsements from other identities/nodes, with the same epoch-gated
revocation semantics (`membership-enrollment.md` §5). Node-trust policy
(allow/block/threshold) evaluates user endorsements identically. Guilds
(bead `6t7`, Papyrus FR34) become sets of identity keys later — same record
shapes.

## 3. The custody spectrum

The user-facing core of the design. Every rung yields "a key + signatures" to
the rest of the system; rungs differ only in who holds the private key and
what a compromise costs.

| Rung | Who holds the key | UX | What it's for |
|---|---|---|---|
| **0 · Anonymous** | Nobody — no key | Connect and surf; nothing to set up | Permissionless baseline. A right, not a fallback. |
| **1 · Ephemeral key** | The device/browser session, throwaway | One tap: "continue with a temporary identity" (Web3 temp-wallet pattern) | Session continuity, ephemeral service use, venue demos. Discarded without ceremony. |
| **2 · Node-custodied session key** | The router, on the device's behalf, bound to the lease | Zero-install; honest label: a **guest badge**, not an identity | Legacy devices that need a stable-for-the-visit handle. |
| **3 · Custodial identity** | A key manager the user *consciously chose to trust* as their signing authority, fronted by standard service-based auth every device supports (username/password/OIDC → the custodian signs on your behalf) | Ordinary web login; nothing to install | "Identity for free" for every service, without self-custody burden. The hosted `identikey-core` stack is *one such custodian* — never a protocol dependency. A custodian outage degrades *its* users' signing, not the mesh. |
| **4 · Self-custodied** | The user's own hardware — IdentiKey app / hardware-backed device key (Secure Enclave / TPM per SP-02) | Scan a QR at the portal (§4); biometric-gated signing | Full sovereignty: endorse others, publish services, own `.mesh` names, multi-device via HD derivation. |

Users can climb: an ephemeral or custodied identity can be *upgraded* by
cross-signing from a higher-rung key (the old key endorses the new, service
bindings migrate). Climbing down is just abandonment.

## 4. The captive portal — an enrollment surface, not a wall

Per the IPv6 decision: IP is access plumbing, and the portal is the
**legacy-device bridge for identity**, exactly as the HTTP gateway is the
legacy bridge for services. Each node runs the portal for its own /24
(sovereignty is structural — no shared portal authority).

**Flow on connect (any device):**

1. Device associates, gets a lease, has internet (rung 0). No wall.
2. First HTTP hit gets the familiar portal interstitial — dismissible —
   offering the identity rungs:
   - **"Just browse"** → done (rung 0).
   - **"Temporary identity"** → browser generates a throwaway keypair in
     `localStorage`/WebCrypto (rung 1), or the node mints and custodies one
     bound to the lease (rung 2) for devices whose browsers can't.
   - **"Sign in"** → standard web auth against the user's chosen custodian
     (rung 3); the custodian returns an attestation binding this
     session/device to the user's identity key.
   - **"Scan with IdentiKey"** → portal shows a QR carrying a challenge +
     the node's `EndpointId`; the app signs with the user's real key and
     delivers the attestation over the mesh (mirrors the `met` enrollment
     handshake, roles reversed) (rung 4).
   - **Operator guest QRs** (venue mode): pre-minted, scoped, expiring guest
     identities printed on paper — scan to join as that guest. Composes with
     rungs 1–2.
3. The node writes the resulting attestation into `/users/…` (or a
   lease-scoped ephemeral record for rungs 1–2) and binds the device's
   IP/lease to the identity for the visit.

**Honesty requirements.** The portal page is served by the node over plain
HTTP inside the mesh (no hosted TLS domain exists, and must not need to).
Rung 2 keys are custodied by the node — the UI must say so ("guest badge").
Rung 1 browser keys are as strong as the browser profile. Neither may be
presented as self-sovereign identity.

## 5. What identity unlocks

- **Named presence**: your entry in the gossip address book (`0yb`) carries
  your identity, not just a lease — reachable by name across islands and
  sites.
- **Service publishing** (`e21`): a `.mesh` name claim is a signature by an
  identity key; name ownership is key ownership. Publish rights on a node are
  a caps grant in the endorsement, evaluated by that node's local policy.
- **Service ACLs**: services gate by key/guild with one verification path,
  regardless of the visitor's custody rung.
- **Cross-site reach**: your identity travels with you — the same key that
  joined at home is recognized at the venue, at whatever trust level local
  nodes' policies grant it.
- **Later — routing trust** (`661`): route-origin validation ("only the
  identity that hashes to / holds the CRDT claim on a block may announce it")
  reuses this exact attestation machinery. Most meshes cannot build this;
  we get it because the identity layer exists.

## 6. Sequencing & open items

**Build order:** after `0yb` (gossip address book — identity records ride the
same directory) and the `e21` architecture pass (services are what identity
unlocks; specifying ACLs before the service model lands would be
speculative). The `identikey-core` port of Papyrus's key-based auth proceeds
on its own track; this spec consumes its `identikey-client` crate
(attestation create/verify, secure storage) as a dependency.

**Open items for the build-phase design pass:**

1. Rung-1 browser key mechanics: WebCrypto Ed25519 availability across stock
   mobile browsers vs. a tiny portal-served signer; what happens on
   private-browsing eviction.
2. Custodian protocol (rung 3): the attestation-request/response wire format a
   custodian implements — must be simple enough that a self-hosted custodian
   is an afternoon project (that's the anti-lock-in test).
3. Lease↔identity binding lifetime and re-auth cadence; roaming a binding
   across nodes within an island (dovetails with the 802.11r/FT key spike,
   bead `9o3`-class hunch).
4. Guest-QR minting UX and scoping vocabulary (time, bandwidth, service set).
5. Whether rung-2 node-custodied keys are worth shipping at all once rung-1
   WebCrypto is proven — prefer fewer rungs if one is redundant.
6. Privacy: ephemeral identities must not be linkable across visits unless
   the user upgrades them deliberately; address-book records for rungs 1–2
   are island-local, never gossiped mesh-wide.

**DWeb demo slice** (when build begins): portal on each node, all rungs
present (anonymous, temp key, QR handshake; custodian mocked), identity
visible in the address book, one key-gated capability — publish `wiki.mesh` —
exercised live from a phone. The moment "projection of a set of keys" becomes
something the audience watches happen.
