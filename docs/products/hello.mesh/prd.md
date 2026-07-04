---
title: hello.mesh — The Mesh Front Desk
created: 2026-07-04
status: validated
scope_tier: mvp
---
# PRD: hello.mesh — The Mesh Front Desk

## Problem Statement

A key-based mesh is invisible and unusable to the one device that shows up most at a venue: a random phone with a browser and no app. Such a device cannot speak the mesh (no iroh, no gossip), cannot see what nodes or services are present, and has no way to acquire an identity without installing software or surrendering an account to a vendor. `hello.mesh` is the node-hosted **front desk** that bridges that gap: a static web page, served by every node for its own segment, that shows a live directory of who and what is on the mesh and lets a visitor optionally acquire a lightweight cryptographic identity — with no app, no account, and no internet. It is the legacy-device on-ramp that makes "the network is a projection of a set of keys" something a person can watch happen. It matters now because the data plane is field-validated and the DWeb demo (week of 2026-07-06) needs a visible, human-facing surface on top of it.

## User Personas

**Venue attendee (app-less)** — A DWeb conference-goer on a stock phone, browser only, no mesh app. Goals: get online immediately; see what local services and people are on the network; optionally get a lightweight identity to unlock more. Pain points today: hotel-style captive portals that gate and surveil; no way to discover local services; forced account creation. Technical level: low to medium.

**Node operator / host** — The person who brings routers to the venue and runs the mesh. Goals: attendees find the front desk and the directory with zero hand-holding and nothing to configure or print; new routers appear to everyone automatically. Pain points: per-device setup, printed artifacts, captive-portal fragility. Technical level: high.

**Mesh-native / developer user** — Someone with the app or building a service on the mesh. Goals: confirm the directory reflects reality; publish a service and watch it appear across nodes. Mostly consumes native iroh paths; uses `hello.mesh` as a human-readable view. Technical level: high.

## User Journeys

**Journey 1 — Join and browse (Venue attendee).** The attendee connects to a mesh AP and opens their browser. `hello.mesh` (or, until name resolution lands, the node's LAN IP) loads with no internet present. The page shows the node they're on and a directory of neighboring nodes currently on the mesh. They tap "just browse" and are online — no account, no wall. Outcome: immediate, permissionless connectivity plus a view of the local network.

**Journey 2 — Acquire a soft identity (Venue attendee).** The same attendee taps "create an identity." The browser generates an Ed25519 keypair in pure JavaScript (the page is an insecure context, so WebCrypto is unavailable), stores it locally, requests a challenge, signs it, and submits it. The page confirms the identity and shows an honest label that this is a soft, browser-held key. Their identity appears in this node's directory. Outcome: a one-tap, self-sovereign-ish identity with no install, honestly scoped.

**Journey 3 — Plug in a router (Node operator).** Mid-event, the operator plugs in another router. It joins the mesh and, within seconds, appears in the directory on every attendee's open `hello.mesh` page. Outcome: the mesh's growth is visible live, with zero configuration.

**Journey 4 — A friend comes online, a remote service is found (the gold).** Two attendees are on *different* nodes of the mesh. One creates an identity at their node's front desk; within seconds it appears in the other attendee's directory, on a different node, reached across the 802.11s island (and, across sites, over the iroh overlay). Separately, someone publishes a service (e.g. `wiki.mesh`) at one node; it becomes visible and reachable from every other node's directory. Outcome: the mesh makes distant people and services present — the actual product, and the demo's headline moment. A directory that showed only the local node would be a Raspberry Pi in a closet; this is what makes it a mesh.

## Success Metrics

- **Demo landing (headline, binary):** at DWeb, on stage, an identity or service created at one node becomes visible in the `hello.mesh` directory at a *different* node — reached across the 802.11s island — with no internet; and a newly plugged-in node appears live. Target: cross-node propagation observed end-to-end in a single run; timeframe: week of 2026-07-06. (Cross-site propagation over the iroh overlay is a stretch within the same run.)
- **Cross-mesh propagation latency:** an identity or service created at node A appears in `directory.json` at node B within 15 seconds across the 802.11s island. Measured by timestamped creation vs. remote API poll; timeframe: at MVP.
- **Page load:** initial `hello.mesh` load completes in under 2 seconds over local WiFi on the target aarch64 router (mt7986-class). Measured with browser devtools / `curl` timing; timeframe: at MVP.
- **Directory freshness:** a node join or leave is reflected in `/api/directory` within 10 seconds. Measured by timestamped join/leave vs. API poll; timeframe: at MVP.
- **Concurrency:** a single node's front desk serves at least 50 concurrent browser clients (static page + directory poll) with zero 5xx responses. Measured by a load-generation script; timeframe: at MVP.
- **Offline:** 100% of MVP functionality works with zero internet and zero upstream DNS. Measured by running the full demo on an air-gapped mesh; timeframe: at MVP.

## Functional Requirements

FR1. [MVP] The system shall serve a static HTML/CSS/JS bundle over HTTP from each node.
FR2. [MVP] The system shall load and execute the bundle with no internet connectivity, referencing no external hosts.
FR3. [MVP] The system shall render a directory of mesh-wide neighbors (all nodes currently present across the mesh, not only the local node) obtained from the directory API.
FR4. [MVP] The system shall display the local node's own summary (node id, claimed client subnet, backhaul address) as a "you are here" header.
FR5. [MVP] The system shall provide an anonymous path that grants full page and directory access without creating any identity.
FR6. [MVP] The system shall generate an Ed25519 keypair in the browser using a pure-JavaScript implementation (not WebCrypto) on explicit user request, and store it in browser-local storage.
FR7. [MVP] The system shall obtain a fresh server challenge and submit a created identity (public key, signature over the challenge, optional label) to the server.
FR8. [MVP] The system shall display a persistent label indicating that a browser-generated identity is soft custody (extractable by the serving node) and not equivalent to app or hardware custody.
FR9. [MVP] The system shall expose `GET /api/directory` returning the neighbor list (and services when available) as JSON.
FR10. [MVP] The system shall expose `GET /api/node` returning the local node's summary as JSON.
FR11. [MVP] The system shall expose `GET /api/health` returning a liveness response for the deploy health-gate.
FR12. [MVP] The system shall expose `GET /api/challenge` returning a single-use nonce.
FR13. [MVP] The system shall expose `POST /api/identity` that validates the submitted signature against the issued challenge and, on success, writes the submission to the ingest spool directory.
FR14. [MVP] The server shall obtain directory data by reading the daemon-written `directory.json` projection rather than deriving mesh state itself.
FR15. [MVP] The server shall run as a service independent of the mesh daemon, such that a node can run the mesh without the front desk.
FR16. [MVP] The mesh daemon shall periodically write a read-only `directory.json` containing the neighbor projection (from the address book and subnet claims) using atomic replacement.
FR17. [MVP] The `directory.json` document shall include a schema version field.
FR18. [MVP] The front desk shall be reachable by the node's LAN IP address independent of `.mesh` name resolution.
FR19. [MVP] The server shall hold no user private key and shall perform no verification of any server, certificate authority, or remote issuer.
FR20. [MVP] The mesh daemon shall replicate published services across the mesh via gossip and include every mesh-wide service in `directory.json`.
FR21. [MVP] The front desk shall render mesh-wide published services in the directory, each with name, address, and port.
FR22. [MVP] The mesh daemon shall ingest identity submissions from the spool directory and replicate them into the `/users` gossip records so an identity created at one node appears in other nodes' directories across the 802.11s island.
FR28. [MVP] The directory shall present identities and services that originate at remote nodes (not only the local node), sourced from converged gossip state.
FR23. [Growth] The system shall advertise the front-desk URL via the RFC 8910 DHCP Captive Portal API option so the OS presents a non-blocking affordance.
FR29. [Growth] The mesh daemon shall replicate identities and services across sites over the iroh overlay so they appear in remote-site directories.
FR24. [Vision] The system shall support signing in against a user-chosen custodial signing service (rung 3) via an OAuth/OIDC-style redirect.
FR25. [Vision] The system shall accept hard-custody identities from a browser extension (rung 1e) or native app (rung 4) linked via an on-screen QR, ticket, or NFC.
FR26. [Vision] The system shall issue a stateless, service-verifiable token to other `.mesh` origins so a single browser identity is usable across mesh sites.
FR27. [Vision] The system shall support an operator-selected gated mode in which connectivity requires presenting an accepted identity rung.

## Non-Functional Requirements

NFR1. [Performance] Initial page load completes in under 2 seconds over local WiFi on the target aarch64 (mt7986-class) router.
NFR2. [Performance] Directory changes (node join/leave) are reflected in the API within 10 seconds.
NFR3. [Scalability] A single node's front desk sustains at least 50 concurrent browser clients without 5xx errors.
NFR4. [Availability] All MVP functionality operates with zero internet connectivity and zero upstream DNS.
NFR5. [Footprint] The `mjolnir-hello` binary with embedded assets adds no more than 10 MB to the router image (target; to be measured — see Open Questions).
NFR6. [Security] The server never holds a user private key and never verifies a server/CA/issuer; all trust is offline signature checking.
NFR7. [Maintainability] The daemon integration is limited to two additive, non-breaking file seams (a written `directory.json` and a read spool directory); no changes to the daemon's existing data-plane behavior.
NFR8. [Portability] The server builds and runs as a standalone workspace binary depending only on the iroh-free `mjolnir-mesh` library types, with no dependency on the daemon's iroh stack.

## Scope Boundaries

### In Scope
- A SvelteKit static-site-generated (SSG) frontend bundle, served over plain HTTP, offline-capable.
- A new `crates/mjolnir-hello` server binary serving the embedded bundle plus a read-only JSON API and one identity-ingest endpoint.
- A **mesh-wide** directory: neighbors, identities, and services that originate at any node, sourced from converged gossip state (across the 802.11s island).
- Anonymous access and one-tap soft (browser-held, pure-JS) rung-1 identity, propagated mesh-wide via `/users` gossip.
- Mesh-wide service replication and rendering (the focused gossip/registry slice of `e21` needed for the directory).
- Two additive daemon seams: a written `directory.json` projection and a read identity spool directory.
- LAN-IP reachability independent of `.mesh` name resolution.

### Out of Scope
- `.mesh` name resolution / DNS propagation and the RFC 8910 advertisement (owned by a separate track; FR23 depends on it).
- Cross-*site* replication over the iroh overlay (FR29) as a committed MVP item — pursued as a demo stretch, since gossip already spans sites, but not a gate.
- The full `e21` service-mesh architecture beyond what the directory needs (conflict-resolution depth, ACLs).
- Custodial sign-in (FR24), browser-extension and app hard-custody linking (FR25), cross-origin tokens for other `.mesh` sites (FR26), and gated mode (FR27).
- Non-extractable WebCrypto keys (unavailable in the plain-HTTP insecure context).

## MVP / Growth / Vision Tiers

### MVP
- FR1–FR2 — Serve a static, offline-capable bundle over HTTP.
- FR3–FR4 — Render the neighbor directory and the local node summary.
- FR5–FR8 — Anonymous access; one-tap soft browser identity with honest custody labeling.
- FR9–FR13 — The read-only JSON API plus challenge and identity-ingest endpoints.
- FR14–FR15 — Server reads `directory.json`; runs independently of the daemon.
- FR16–FR17 — Daemon writes the versioned `directory.json` projection.
- FR18 — LAN-IP reachability without `.mesh` resolution.
- FR19 — No private-key custody and no server/CA verification.
- FR20–FR21 — Mesh-wide services replicated via gossip and rendered in the directory.
- FR22 — Spool ingest replicating identities into mesh-wide `/users` gossip (the "friend comes online" gold).
- FR28 — Directory presents remote-origin identities and services from converged gossip.

### Growth
- FR23 — RFC 8910 non-blocking affordance advertisement.
- FR29 — Cross-site (over-iroh) replication into remote-site directories (demo stretch; likely near-free since gossip already spans sites).

### Vision
- FR24 — Custodial (rung 3) OAuth/OIDC sign-in.
- FR25 — Hard-custody (rung 1e extension / rung 4 app) linking via QR/ticket/NFC.
- FR26 — Stateless cross-origin tokens for other `.mesh` sites.
- FR27 — Operator-selected gated mode.

## Constraints

- **Hardware/OS:** OpenWrt on aarch64 (mt7986-class); the server is a small static binary running under procd, with config via the existing `/etc/config/mjolnir` UCI convention.
- **Insecure context:** the front desk is served over plain HTTP, which is not a secure context; WebCrypto, Service Workers, and WebTransport are therefore unavailable, so MVP identity uses pure-JS Ed25519 only.
- **Offline-first:** the venue may have no internet; the bundle references no external hosts.
- **Daemon stability:** `mjolnir-meshd` must remain untouched except for the two additive, non-breaking file seams.
- **Timeline:** the DWeb demo the week of 2026-07-06 is a hard deadline. The MVP is scoped so that cross-mesh propagation *across the 802.11s island* is the committed target and cross-*site* over iroh is a stretch. This is a deliberate tension: mesh-wide propagation is the product (a node-local directory is worthless), so it is in MVP even though it pulls the `/users` gossip path (`rp9`) and a service-registry slice (`e21`) onto the critical path.
- **Critical-path dependency (foundations):** MVP now rides on gossiped `/users` records and gossiped service records. The gossip/CRDT *transport* is already built and field-validated (subnet claims converge mesh-wide, including cross-site), so the remaining work is record-type definition + spool ingest + directory wiring — not new distributed-systems transport.
- **Cross-track dependency:** `.mesh` name resolution is designed and owned by a separate track; MVP must not block on it (hence FR18).

## Assumptions & Risks

- **[Assumption] The daemon can cheaply project its live gossip snapshot to `directory.json`.** Impact if false: the read seam needs a heavier mechanism. Mitigation: the daemon already snapshots and atomically persists claims and the address book; reuse that pattern. (Low risk — validated by existing `persist_*` code.)
- **[Risk — data] The neighbor list may be sparse until multi-hop discovery (`0yb`) is complete.** Impact: a thin directory at the demo. Mitigation: the address book is already seeded and persisted today; validate directory density on the real fleet before the demo.
- **[Risk — critical path] Mesh-wide propagation is now MVP and depends on unbuilt record types (`/users` per `rp9`; a service-registry slice per `e21`).** Impact: if these slip, the headline gold (a friend appearing across the mesh, a remote service found) does not land. Mitigation: the gossip transport is already validated mesh-wide, so scope the demo to island-local propagation over that proven layer and treat the record-type + wiring work as the focused critical path; cut cross-site (FR29) to a stretch. This is the single biggest schedule risk and should be de-risked first with a spike that gossips one new record type end-to-end.
- **[Risk — scope] The service slice needed for the directory could pull in the full `e21` architecture.** Impact: scope creep. Mitigation: scope to replicate + render service entries only; defer conflict-resolution depth and ACLs.
- **[Risk — dependency] `.mesh` resolution is a separate track that could slip.** Impact: the front desk is reachable only by LAN IP, weakening the `hello.mesh` UX. Mitigation: FR18 keeps MVP functional by IP; signage can carry the IP as a fallback.
- **[Risk — technical] Binary size with embedded assets plus the HTTP library on aarch64 is unmeasured.** Impact: image bloat. Mitigation: measure early; fall back to serving assets from a staged directory if the embed exceeds NFR5.
- **[Assumption] Pure-JS Ed25519 with `crypto.getRandomValues` works across stock mobile browsers in an insecure context.** Impact if false: no soft identity on some devices. Mitigation: `getRandomValues` is available in insecure contexts; verify on representative venue devices before the demo.

## Open Questions

- What is the daemon's write cadence for `directory.json` (on-change vs. fixed interval), and does the server stat-poll the file or does the daemon signal freshness?
- What is the actual binary-size cost of `rust-embed` plus the chosen HTTP library on the target, and does it fit NFR5?
- For the spool-ingest write path (Growth), does the daemon watch the directory (inotify) or sweep on a timer, and what are the validation and de-duplication rules?
- **Decided (2026-07-04):** cross-mesh propagation is required (node-local is out), and the demo is scoped to the **802.11s island first** — cross-*site* over iroh (FR29) is Growth/stretch, not a demo gate. No longer open.
- What is the smallest spike that proves one new gossip record type (`/users`) propagating end-to-end across two nodes, to de-risk the critical path first?
- Is the 50-concurrent-client target (NFR3) realistic on the target router hardware, or should it be recalibrated after a first measurement?

## Existing System Context

- **`mjolnir-meshd`** (`crates/mjolnir-mesh`, `daemon` feature) — the OpenWrt router daemon: iroh transport, gossip, CRDT. It persists `claims.state` and the address book as postcard binaries via atomic `persist_claims`/`persist_addr_book`; it has no HTTP server today. A `status` subcommand provides a read-only inspection precedent. **The CRDT gossip layer is field-validated to converge mesh-wide, including cross-site over the iroh overlay** (subnet claims propagate across the fleet today) — the substrate the new `/users` and service records ride, so cross-mesh propagation reuses proven transport rather than building new.
- **`mjolnir-mesh` library** — exposes iroh-free CRDT types (`AddrBook`/`PeerAddrEntry`, `SubnetClaim`, `ServiceEntry`, `DnsEntry`); `mjolnir-hello` will depend on these for decoding and serving state without pulling the daemon's iroh stack.
- **OpenWrt integration** — procd services, `/etc/config/mjolnir` UCI configuration, and `uhttpd` available on the box (not used; the front desk is self-serving).
- **Design docs & tracking** — `docs/network-coordination/hello-mesh-service.md` (this build's implementation design), `docs/network-coordination/user-identity.md` (the identity spec, bead `rp9`), and beads epic `bc7` with children `gad` (frontend), `bl2` (server), `avs` (`directory.json` seam), and `p6u` (spool ingest).
