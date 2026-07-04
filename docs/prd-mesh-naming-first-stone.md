---
title: .mesh Naming — First Stone (DNS responder + service publishing)
created: 2026-07-03
status: validated
scope_tier: mvp
---
# PRD: .mesh Naming — First Stone

## Problem Statement

The mesh has no name layer: reaching a node or service today requires knowing a derived `10.254.x` address or an iroh node-id, which is unusable for a phone at a demo table and clunky even for operators. `hello.mesh` (rp9) and any node-hosted service need a name that resolves the same way whether the target is one radio hop away or across a site, without a registrar, a zone file, or any single point of authority — consistent with the mesh's no-central-authority ethos. This PRD ("first stone") delivers the minimum naming slice needed for the 2026-07-06 DWeb demo: a working `.mesh` DNS responder, the compiled-in `hello.mesh`/`id.mesh` well-knowns, and operator-publishable services (e.g. `wiki.mesh`) that resolve and converge across nodes via the same gossip/CRDT pattern already proven by the `0yb` address book.

## User Personas

**Node operator / fleet maintainer** — Runs and maintains the 4-router field fleet, publishes services onto the mesh, and debugs resolution issues. Goals: publish a name in one command and have it resolve fleet-wide within seconds; see clearly who owns a name and whether a conflict happened. Pain points today: no name layer exists — services are reached by raw IP or not at all. Technical level: high.

**Demo attendee (phone, no app)** — A DWeb attendee joining the mesh AP with a stock phone and browser, no mesh software. Goals: get a working local page (`hello.mesh`) with zero manual configuration, and load a service someone else published (`wiki.mesh`) without knowing an IP. Technical level: low to medium.

**Second operator publishing during a partition** — A fleet maintainer who, during a network split, publishes a name someone else already claimed on the other side of the partition. Goals: understand what happened when gossip re-converges and get an honest, non-silent signal that they lost. Technical level: high.

## User Journeys

**Journey 1 — Phone joins, front desk loads (Demo attendee).** The attendee's phone joins the mesh SSID and DHCP hands out the node's client subnet, including option 114 pointing at `http://hello.mesh`. The phone's browser resolves `hello.mesh`: dnsmasq forwards the query to `127.0.0.1:5335`, the embedded responder answers with the compiled-in well-known rule (this node's own gateway IP, TTL 30s), and the front desk page loads immediately. No manual IP entry, no captive portal wall. Outcome: the phone is online and looking at a live directory within seconds of associating.

**Journey 2 — Operator publishes, a different node's client resolves it (Node operator + attendee).** An operator SSHes to node A over the overlay and runs `meshd publish wiki --port 8080`. The daemon writes a `ServiceEntry v2` record (owner = node A's iroh key, HLC-stamped), persists it to `services.state`, and immediately gossips a `ServicePublishV2` message rather than waiting for the next anti-entropy tick. Within one gossip hop, node B's daemon merges the new record (different name, no conflict) and its embedded responder starts answering `wiki.mesh` with node A's gateway IP. A client on node B's segment requests `http://wiki.mesh`, resolves it locally at node B, and the request routes over babel (802.11s island) or the iroh overlay (`mjolnir0`, cross-site) to node A — DNS never has to know which. Outcome: end-to-end publish-to-load within the 60-second demo acceptance bound, no shared authority involved.

**Journey 3 — Partition conflict, loser experience (Two operators).** During a network partition, an operator at node A publishes `printer.mesh` (HLC t1) and, unaware, an operator at node C publishes `printer.mesh` too (HLC t2 > t1). While partitioned, each island resolves the name locally as if it were the sole owner. When the partition heals and gossip re-converges, `merge_service` sees two different owners for the same name: first-claim HLC wins deterministically everywhere (node A, t1, keeps the name), tie-broken by node-id if HLCs were ever equal. Node C's daemon immediately stops answering `printer.mesh`, `meshd status` on node C shows the name as lost with node A's node-id as the winner, and a subsequent `meshd publish printer` on node C fails with an actionable error naming node A as the current owner. Outcome: no split-brain, no silent data loss — the loser gets an honest, discoverable explanation instead of a mysteriously broken name.

## Success Metrics

- **Demo acceptance (headline, binary):** a phone joins the mesh SSID and `hello.mesh` loads with zero manual configuration; a service published on node A (e.g. `wiki.mesh`) resolves and its HTTP page loads from a client on node B, end-to-end within 60 seconds of the publish command. Measured live at DWeb; timeframe: 2026-07-06.
- **Convergence bound:** a newly published or unpublished service resolves consistently fleet-wide within one anti-entropy interval (20s) of the initial immediate-gossip broadcast, worst case ~50s (20s anti-entropy + 30s TTL) if the immediate broadcast is somehow missed. Measured by timestamped publish vs. dig/resolve poll across nodes; timeframe: at MVP.
- **Conflict resolution correctness:** in a simulated partition-then-heal test with two owners claiming the same name, all nodes converge on the identical winner (by first-claim HLC, deterministic tiebreak) with zero split-brain answers. Measured by integration test across ≥2 simulated nodes; timeframe: at MVP.
- **Mixed-fleet safety:** an old (pre-upgrade) node receiving the new `ServicePublishV2` gossip variant neither crashes nor corrupts its CRDT state (decode-skip only). Measured by a postcard round-trip / mixed-version integration test; timeframe: before fleet rollout.
- **Resolver correctness:** 100% of `dig`/resolver conformance test cases (well-known A, service A, non-A NODATA, unknown-name NXDOMAIN, SRV/TXT) pass. Measured by an automated test suite against the embedded responder; timeframe: at MVP.

## Functional Requirements

FR1. [MVP] The system shall run an authoritative DNS responder for the `.mesh` zone bound to UDP `127.0.0.1:5335`.
FR2. [MVP] The responder shall answer `A` queries for well-known names with this node's own client gateway IP, TTL 30 seconds.
FR3. [MVP] The responder shall answer `A` queries for published service names with the service's published IP, TTL 30 seconds.
FR4. [MVP] The responder shall return NXDOMAIN for any `.mesh` name that is neither a well-known nor a currently published service.
FR5. [MVP] The responder shall return NOERROR with an empty answer (NODATA), never NXDOMAIN, for non-`A` queries on a name that exists.
FR6. [MVP] The responder shall answer `SRV` and `TXT` queries for published services using the service's port, protocol, and TXT map.
FR7. [MVP] The responder shall include an SOA record in the authority section of negative (NXDOMAIN/NODATA) responses.
FR8. [MVP] The responder shall be a pure projection of in-memory CRDT state with no independent cache, such that a gossip merge is visible on the very next query with no reload step.
FR9. [MVP] The daemon's UCI reconcile shall add `dhcp.@dnsmasq[0].server='/mesh/127.0.0.1#5335'` so dnsmasq forwards `.mesh` queries to the embedded responder.
FR10. [MVP] The daemon's UCI reconcile shall set DHCP option 114 to `http://hello.mesh` (RFC 8910 non-blocking captive-portal affordance).
FR11. [MVP] The daemon's UCI reconcile shall configure dnsmasq to NXDOMAIN the `use-application-dns.net` DoH canary.
FR12. [MVP] The UCI reconcile shall be idempotent, applying no changes when the configuration is already current (same discipline as `reconcile_client_uci`).
FR13. [MVP] The daemon shall never edit dnsmasq's files directly and never send it SIGHUP; configuration changes go through UCI plus an init.d restart only.
FR14. [MVP] The embedded responder shall be bound and answering before the UCI reconcile commits the dnsmasq forward line, so no query is ever forwarded to a not-yet-listening responder.
FR15. [MVP] The system shall maintain a compiled-in reserved well-known name list (`hello.mesh`, `id.mesh`) that is unclaimable and never stored in the CRDT.
FR16. [MVP] The system shall reject any publish or claim attempt on a reserved well-known name with an error stating that the name is reserved and unclaimable.
FR17. [MVP] The system shall define `ServiceEntry` v2 with fields `owner_node_id` (iroh key), `published_at` (HLC), `ip`, `port`, `protocol`, `txt` (map), and `host_mac` as `Option`.
FR18. [MVP] The merge function (`merge_service`) shall resolve a same-owner update with a newer HLC as `Updated`.
FR19. [MVP] The merge function shall resolve a same-owner update with an older or equal HLC as `Unchanged`.
FR20. [MVP] The merge function shall resolve a different-owner claim on the same name as `Conflict`, with the original owner's first-claim HLC winning first-writer-wins, tie-broken by a deterministic node-id comparison, independent of argument order.
FR21. [MVP] The gossip protocol shall carry service publishes as a new, appended `ServicePublishV2` variant with its own dispatch arm, without mutating the schema of the existing `ServiceUpdate` variant.
FR22. [MVP] The daemon shall persist service state to `services.state` via atomic tmp-file-plus-rename on both the anti-entropy tick and on every publish/unpublish.
FR23. [MVP] The daemon shall restore `services.state` on boot and be able to answer for its own previously-published services immediately, before any gossip round-trip.
FR24. [MVP] The daemon shall re-announce its own published services on every anti-entropy tick (~20s), matching the `0yb` address-book pattern.
FR25. [MVP] The daemon shall broadcast a gossip message immediately on publish or unpublish, rather than deferring the announcement to the next anti-entropy tick.
FR26. [MVP] The daemon shall provide a `meshd publish <name> --port <N>` command that creates or updates a service entry.
FR27. [MVP] The daemon shall provide a `meshd unpublish <name>` command that removes a service entry.
FR28. [MVP] The daemon shall expose a write-path IPC mechanism (file-spool or unix socket) so `publish`/`unpublish` can mutate the state of an already-running daemon process, since the existing `status` surface is read-only.
FR29. [MVP] A published service's `A` record shall resolve to the publishing node's client gateway IP.
FR30. [MVP] An unpublish shall gossip a tombstone, and all nodes shall stop answering for that name within one anti-entropy cycle (20s).
FR31. [MVP] An owner shall be able to re-publish a name after unpublishing it.
FR32. [MVP] The `status` command shall show the owning node-id for every known name and list any active conflicts with their winner and loser, including — on the losing node — each lost name together with the winning owner's node-id.
FR33. [MVP] On losing a name conflict, a node shall stop answering that name immediately, without waiting for a further gossip round.
FR34. [MVP] A publish attempt on a name already lost to conflict shall fail with an error naming the current winning owner.
FR35. [MVP] Old (pre-upgrade) nodes receiving the new `ServicePublishV2` gossip variant shall decode-skip it without crashing or corrupting existing state, verified by postcard round-trip tests.
FR36. [Growth] The system shall support device names (e.g. `laptop.mesh`) derived from DHCP lease data via a `/dns/{hostname}` lane.
FR37. [Growth] The system shall apply expiry/staleness handling to service records, unified with the address-book staleness design (bead `e21.9`).
FR38. [Growth] The system shall garbage-collect tombstoned service records rather than retaining them indefinitely.
FR39. [Growth] The system shall notify the operator of a name conflict within 2 seconds of the local merge (target; finalize during Growth design), rather than requiring a manual `status` check.
FR40. [Vision] The system shall reflect `/services/` entries into per-segment mDNS so AirPlay/Bonjour-class clients discover mesh services natively.
FR41. [Vision] The system shall support web-of-trust-based name arbitration as an alternative to pure first-writer-wins.
FR42. [Vision] The system shall support off-mesh dial-by-node-id using the provenance node-id carried in service records.

## Non-Functional Requirements

NFR1. [Footprint] The DNS responder implementation shall add minimal size to the static aarch64 `mjolnir-meshd` binary; no heavyweight DNS crate (e.g. hickory) may be introduced — evaluate a minimal pure-Rust wire-format crate or hand-rolled parsing against the existing size budget.
NFR2. [Compatibility] Mixed-fleet rollout shall be safe at every point during the update: nodes running the old binary shall NXDOMAIN `.mesh` queries (acceptable degraded behavior) without crashing or corrupting CRDT state, per the appended-gossip-variant discipline.
NFR3. [Latency] The responder shall answer a query from in-memory state in under 10ms on the target mt7986-class hardware, with no I/O on the query path (pure CRDT projection).
NFR4. [Convergence] Name resolution shall converge fleet-wide within one anti-entropy interval (20s) of an immediate-gossip broadcast; worst-case bound (broadcast miss) is anti-entropy interval plus TTL (20s + 30s = 50s).
NFR5. [Ethos/Availability] Name resolution and publishing shall require no central authority, registrar, or single point of failure; every node answers from its own converged CRDT state and functions independently of any other single node being reachable.
NFR6. [Safety] Daemon-driven dnsmasq configuration changes shall go exclusively through UCI plus init.d restart (never direct file edits or SIGHUP), preserving the existing `reconcile_client_uci` discipline and riding the `mjolnir-apply` health-gated rollback path.
NFR7. [Determinism] Conflict resolution (FWW + node-id tiebreak) shall produce the identical winner on every node regardless of gossip arrival order or which node evaluates the merge.

## Scope Boundaries

### In Scope
- The embedded `.mesh` DNS responder on `127.0.0.1:5335` and its dnsmasq/UCI wiring (server line, option 114, DoH canary).
- Compiled-in well-known names (`hello.mesh`, `id.mesh`) resolving to the local node's own gateway IP.
- `ServiceEntry` v2, `merge_service`, and the new appended `ServicePublishV2` gossip variant, reusing the `0yb` self-announce/persist/re-announce/boot-restore pattern.
- A `meshd publish`/`unpublish` CLI plus the write-path IPC needed to reach a running daemon.
- Owner-bound conflict resolution (first-claim HLC, deterministic node-id tiebreak) and its operator-visible provenance/loser UX via `status`.
- Tombstoning on unpublish (propagation only; GC deferred).

### Out of Scope
- Device names / `laptop.mesh` via the lease lane (needs the lease lane; Growth).
- Expiry/staleness handling for stale or offline-owner service records (bead `e21.9`; Growth).
- Tombstone garbage collection (Growth).
- Near-real-time (push) conflict notification beyond `status` (Growth).
- mDNS reflection of `.mesh` services (Vision).
- Web-of-trust name arbitration beyond FWW (Vision; explicitly punted to its own bead per the interview).
- Off-mesh dial-by-node-id (Vision).
- The `hello.mesh` front-desk HTTP server itself (owned by bead `bc7`/rp9; this PRD only guarantees the name resolves to a gateway IP).

## MVP / Growth / Vision Tiers

### MVP
FR1–FR35 — the DNS responder, dnsmasq/UCI wiring, well-known names, `ServiceEntry` v2 + merge, gossip/persistence, publish/unpublish surface, tombstone propagation, provenance/loser UX, and mixed-fleet safety.

### Growth
FR36 — device names via the lease lane.
FR37 — service record expiry/staleness (bead `e21.9`).
FR38 — tombstone garbage collection.
FR39 — conflict notification within 2 seconds of local merge.

### Vision
FR40 — mDNS reflection.
FR41 — web-of-trust name arbitration.
FR42 — off-mesh dial-by-node-id.

## Constraints

- **Binary size budget:** cross-built static aarch64 target; no DNS crate currently in the workspace; must avoid heavy dependencies (hickory is explicitly too heavy). `tokio` and `postcard` are already present and reusable.
- **Mixed-fleet rollout safety:** the fleet updates in stages via `mjolnir-apply` (snapshot → apply → health gate → rollback); the wire format must tolerate old and new binaries running simultaneously during rollout.
- **Quality gates:** `cargo test --workspace` and `clippy` must pass.
- **Demo deadline:** 2026-07-06 (DWeb demo), hard constraint on MVP scope.
- **dnsmasq discipline:** never edit dnsmasq's own files, never SIGHUP it — UCI plus init.d restart only, matching existing `reconcile_client_uci` behavior.
- **No central authority:** consistent with mesh ethos, there is no registrar, zone authority, or arbitration server; all naming facts are CRDT-gossiped, symmetric across nodes.

## Assumptions & Risks

- **[Assumption] dnsmasq's `server=/mesh/#port` forwarding behaves as expected on the fleet's OpenWrt version**, including graceful handling of truncated/large replies (EDNS0/OPT or TCP fallback). Impact if false: resolution silently fails for some clients. Mitigation: keep responses ≤512B where possible; test against the actual fleet's dnsmasq version before the demo.
- **[Assumption] DHCP option 114 actually surfaces on demo phones (iOS/Android).** Impact if false: attendees must be told to browse to `hello.mesh` manually. Mitigation: validate on representative demo devices ahead of 2026-07-06; treat the affordance as best-effort, not load-bearing.
- **[Risk] Android Private DNS (DoT) bypasses the local resolver entirely; the DoH canary only covers Firefox.** Impact: some Android clients cannot resolve `.mesh` at all. Mitigation: document as a known limitation; instruct demo staff to disable Private DNS on demo devices if needed.
- **[Assumption] 30s TTL is acceptable post-roam, and immediate-gossip-on-publish keeps the worst-case publish-to-resolve bound at anti-entropy interval plus TTL (~50s).** Impact if false: stale answers linger longer than expected. Mitigation: the immediate-broadcast requirement (FR25) exists specifically to avoid depending on the tick interval alone.
- **[Assumption] The postcard appended-variant decode-skip pattern, proven by the `0yb` mixed-fleet rollout, holds for `ServicePublishV2` too.** Impact if false: old nodes could crash or corrupt state during rollout. Mitigation: dedicated mixed-version round-trip tests (FR35) before fleet rollout, not just unit tests of the new variant in isolation.
- **[Risk] Offline-owner services resolve to a black hole until staleness (bead `e21.9`) lands.** Impact: a node that goes offline leaves its published names resolving to an unreachable IP with no expiry. Mitigation: documented as an accepted MVP limitation; not a demo blocker given the short demo window.
- **[Risk] The write-path IPC mechanism (file-spool vs. unix socket) is undecided**, which could affect the publish command's responsiveness and daemon integration complexity. Impact: implementation churn if the wrong mechanism is picked first. Mitigation: default to the file-spool pattern already used elsewhere in the daemon unless a spike shows a unix socket is materially simpler for this near-synchronous use case.
- **[Risk] `hello.mesh` before a `/24` is claimed has undefined behavior** (stock alias vs. refuse). Impact: early-boot or misconfigured-node front-desk requests could behave unexpectedly. Mitigation: pin the answer explicitly (see Open Questions) before implementation.

## Open Questions

- Tombstone wire format: what does a tombstone gossip message/record look like, distinct from a live `ServiceEntry`?
- SOA field values (MNAME, RNAME, serial, refresh/retry/expire/minimum) for negative-answer authority sections — what values make sense for a CRDT-backed, authority-less zone?
- Responder observability: is query logging needed for demo-day debugging, and if so, where does it go (syslog, in-memory ring buffer, `status` extension)?
- Publish IPC mechanism: file-spool (matching the existing daemon pattern) vs. unix socket — which fits the near-synchronous `meshd publish` UX better?
- `hello.mesh` before a `/24` is claimed: answer the stock `192.168.1.1` alias, or refuse the query? Needs an explicit decision before implementation.
- How are locally-published services restored and re-owned correctly after a daemon restart (distinguish "this node's own services to restore" from "other nodes' services learned via gossip" in `services.state`)?

## Existing System Context

- **CRDT modules (`crates/mjolnir-mesh/src/crdt/`):** `subnet.rs` (live, subnet claims), `peer_addr.rs` (0yb, live — the pattern to reuse: self-announced entries, LWW same-owner merge, ~20s anti-entropy re-announce, atomic tmp+rename persistence to `addrbook.state`, boot restore, `status` printing), `service.rs` (v1 schema — hostname/ip/port/protocol/txt/host_mac — defined but never wired into the apply loop), `dns.rs` (v1, lease-derived, unwired), `merge.rs` (pure merge functions plus a `Conflict{winner, loser}` type), `gossip.rs` (the `GossipMessage` enum, including the legacy `ServiceUpdate` variant that must not be mutated), `sync.rs` (the `GossipTransport` seam), `hlc.rs` (hybrid logical clock).
- **Daemon (`src/bin/mjolnir-meshd.rs`, ~3700 lines):** `reconcile_client_uci` (~line 1921) enforces UCI-only discipline with an idempotence check (`lan_uci_is_current`) — the pattern the new dnsmasq/DNS reconcile should follow; an anti-entropy tick loop; a dispatch loop for gossip messages; a `MemoryLookup` address-book feed; a `status` subcommand for read-only inspection (no write path exists today).
- **No DNS crate dependency exists yet** in the workspace; `crates/` currently contains only `mjolnir-mesh` and `mjolnir-meshctl` (voice/media was extracted to the separate `mjolnir-voice` repo).
- **`crates/mjolnir-hello` does not exist yet** (bead `bc7` epic) — until it ships, `hello.mesh` will resolve correctly to the gateway IP, but nothing is guaranteed to be listening on HTTP there; this PRD's job ends at correct name resolution.
- **Design reference:** `docs/network-coordination/mesh-naming.md` (the e21 design pass this PRD implements) and `docs/network-coordination/gossip-and-crdt.md` (how facts converge generally); decisions tracked as beads `e21.1`/`e21.2`.
