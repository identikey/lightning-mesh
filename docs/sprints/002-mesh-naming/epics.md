# Sprint 002 — .mesh Naming First Stone: Epics & Stories

PRD: `docs/prd-mesh-naming-first-stone.md` (MVP = FR1–FR35). ADRs: `architecture-decisions.md` (D-001…D-006).
Ceremony: standard (lean story template — stubs below carry Story + ACs + test_tier; flagged stories get full specs in `stories/`).
Beads mode: ON — beads is the source of truth once materialized; this file is the planning narrative.

Dependency spine: E2 (lane substrate) ⊥ E1.1–1.3 (responder) can proceed in parallel; E1.4 (UCI) after E1.1–1.2; E3 after E2; E4 last.

> **BASELINE NOTE (2026-07-03, post-planning audit):** bead `7jb` (bc7 session) shipped a v1
> service-gossip lane before this sprint starts: `ServiceUpdate` variant, LWW `merge_service`,
> `ServiceBook` + anti-entropy + persist (commit 37befe0). E2 is therefore an **upgrade**, not
> greenfield: S2.1 replaces LWW with owner-bound semantics, S2.2 appends v2 variants at the
> current enum tail (after `UserUpdate`; `ServiceUpdate` itself is fleet-decoded and frozen),
> S2.3 ports the existing wiring — `services.state` already exists with v1 content — to the v2
> book. Sprint-001 landing (2026-07-04) also shipped the versioned `directory.json` projection
> and `mjolnir-hello` on gateway:80, which pins S3.1's response shape and confirms the
> option-114 path. Details in the bead notes.

Bead mapping: S1.1–S1.4 → `e21.1.1`–`e21.1.4` · S2.1–S2.4 → `e21.2.1`–`e21.2.4` ·
S3.1–S3.3 → `e21.2.5`–`e21.2.7` · S4.1–S4.3 → `e21.6`–`e21.8`.

---

## E1 — Embedded .mesh DNS responder + well-known names + dnsmasq wiring
Maps bead `e21.1`. FR1–FR16, FR29. Decisions: D-001, D-003, D-005.

### S1.1 Responder skeleton on 127.0.0.1:5335 (simple-dns)
Add `simple-dns` (daemon feature), bind UDP 127.0.0.1:5335, parse queries, dispatch by name class, default NXDOMAIN. Tolerate/ignore EDNS0 OPT; responses ≤512B.
AC: malformed packet never panics; unknown `.mesh` name → NXDOMAIN with SOA authority (D-005); responder task starts before UCI reconcile runs (FR14 ordering hook exposed).
complexity: medium · test_tier: smoke

### S1.2 Well-known names (hello.mesh, id.mesh)
Compiled-in reserved list; A answers = own client gateway IP, or 192.168.1.1 pre-claim (D-003); TTL 30s; NODATA (never NXDOMAIN) for non-A on existing names.
AC: FR2, FR5, FR15 pass as unit tests; reserved list rejected at merge and at publish (FR16 hook for E2/E3).
complexity: small · test_tier: smoke

### S1.3 Service answers as pure CRDT projection
A/SRV/TXT for published services straight from the in-memory service store; no cache (FR8); SRV/TXT built from port/protocol/txt map (FR6).
AC: a store mutation is visible on the immediately following query; FR3, FR6 unit-tested.
complexity: medium · test_tier: smoke

### S1.4 dnsmasq/UCI wiring (server line, option 114, DoH canary) — FLAGGED
Extend the UCI reconcile: `dhcp.@dnsmasq[0].server='/mesh/127.0.0.1#5335'`, DHCP option 114 = `http://hello.mesh`, `server='/use-application-dns.net/'`; idempotence check (no restart when current); responder bound BEFORE commit (FR14); UCI+init.d only (FR13/NFR6).
AC: FR9–FR14 pass; re-run reconcile with current config → zero restarts; misordered start impossible by construction.
Risk: touches fleet-wide client DNS — a bad line breaks ALL resolution, not just .mesh; must ride mjolnir-apply health gate.
complexity: medium · test_tier: thorough

---

## E2 — /services/ CRDT lane: schema, merge, gossip, persistence
Maps bead `e21.2` substrate. FR17–FR25, FR30–FR33, FR35. Decisions: D-004, D-006.

### S2.1 ServiceEntry v2 + owner-bound merge_service — FLAGGED
v2 fields (owner_node_id, published_at HLC, ip, port, protocol, txt, host_mac: Option). merge_service: same-owner newer-HLC=Updated / older-or-equal=Unchanged / different-owner=Conflict, FWW on FIRST-CLAIM HLC, deterministic node-id tiebreak, argument-order independent (FR20/NFR7); reserved names rejected.
AC: FR17–FR20 property-tested (argument-order symmetry, determinism across simulated nodes); postcard round-trip.
complexity: medium · test_tier: thorough

### S2.2 Gossip variants ServicePublishV2 + ServiceUnpublishV2 — FLAGGED
Two NEW appended variants (never mutate legacy ServiceUpdate — FR21, D-004); dispatch arms; tombstone semantics (retained, HLC-ordered, same-owner revive per FR31).
AC: FR21, FR30, FR31, FR36-old-node decode-skip verified with a pinned old-schema decoder test (FR35); mixed-version round-trip suite.
complexity: medium · test_tier: thorough

### S2.3 Persistence, boot restore, announce discipline
services.state (tmp+rename) written on tick + every publish/unpublish (FR22); boot restore with own-vs-learned split (D-006, FR23); re-announce own entries each anti-entropy tick (FR24); immediate gossip broadcast on publish/unpublish (FR25).
AC: restart → own services answerable immediately and re-announced; learned entries never re-announced.
complexity: medium · test_tier: smoke

### S2.4 Conflict application + loser state
On merge Conflict where self is loser: drop from answer set immediately (FR33), record lost-name {winner_node_id} for status/API (FR32), mark name as conflict-lost so publish re-attempts fail with the winner named (FR34).
AC: FR32–FR34 integration-tested on two in-memory nodes.
complexity: medium · test_tier: smoke

---

## E3 — Control API + publish CLI + provenance surfaces
FR26–FR28, FR32, FR34. Decision: D-002.

### S3.1 meshd control API (127.0.0.1:5380)
POST /v0/publish, POST /v0/unpublish, GET /v0/directory; structured errors (reserved, owned_by_other{winner}); localhost-only bind. Convergence target for bc7's file seams (D-002).
AC: FR28; publish of reserved/lost names returns the structured error synchronously; directory returns names+owners+conflicts.
complexity: medium · test_tier: smoke

### S3.2 CLI: meshd publish / unpublish
Thin HTTP clients of the control API (FR26/FR27); render errors actionably (reserved → "reserved and unclaimable"; conflict → winner node-id).
AC: FR16/FR34 error strings exact; exit codes nonzero on failure.
complexity: small · test_tier: yolo

### S3.3 status shows names, owners, conflicts
Extend status: every known name with owner node-id, active conflicts with winner/loser, lost names (FR32; dbv discipline: explicit none marker).
AC: FR32 output snapshot-tested.
complexity: small · test_tier: yolo

---

## E4 — Conformance, multi-node integration, fleet rollout + demo
FR1–FR35 end-to-end; success metrics section of the PRD.

### S4.1 Resolver conformance suite — FLAGGED
Automated dig-class matrix against the responder: well-known A (claimed + pre-claim), service A, non-A NODATA, unknown NXDOMAIN, SRV/TXT, SOA fields, oversized/EDNS0 queries.
AC: PRD "Resolver correctness" metric = 100% pass; runs in cargo test.
complexity: medium · test_tier: thorough

### S4.2 Multi-node integration: convergence + partition conflict — FLAGGED
Simulated ≥2-node harness (in-memory transport): publish→resolve across nodes within one tick; partition double-claim heals to identical winner everywhere, zero split-brain (NFR7); tombstone propagation ≤1 cycle.
AC: PRD "Conflict resolution correctness" + "Convergence bound" metrics automated.
complexity: large · test_tier: thorough

### S4.3 Fleet rollout + demo acceptance — FLAGGED
Cross-build; batch update all 4 nodes via mjolnir-apply (single batch — mixed-fleet window NXDOMAINs .mesh); live validation: phone joins → hello.mesh loads; wiki.mesh published on node A loads from node B ≤60s. Demo runbook incl. Android Private DNS + option-114 device caveats (PRD risks).
AC: PRD "Demo acceptance" metric passes on the physical fleet before 2026-07-06.
complexity: medium · test_tier: thorough

---

## Totals
4 epics · 14 stories · flagged: S1.4, S2.1, S2.2, S4.1, S4.2, S4.3 (6)
