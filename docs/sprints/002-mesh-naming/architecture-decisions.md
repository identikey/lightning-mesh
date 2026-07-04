# Sprint 002 — Architecture Decisions (.mesh naming, first stone)

PRD: `docs/prd-mesh-naming-first-stone.md` (validated). Design: `docs/network-coordination/mesh-naming.md`.
Steering: GUIDED. [USER] = decided by Duke in elicitation; [AUTO-DECIDED] = orchestrator, LOW/MEDIUM significance.

## D-001 [HIGH] [USER] DNS wire format: `simple-dns` crate

The embedded responder uses the `simple-dns` crate for packet parse/build (A, SRV, TXT, SOA,
NXDOMAIN/NODATA), keeping all projection logic in-house. Rationale: pure-Rust, small, no-std-capable —
fits the static aarch64 budget (NFR1) with far lower correctness risk than hand-rolling label
compression/EDNS0-OPT handling. hickory rejected as too heavy (PRD NFR1). Responses kept ≤512B; the
OPT record in queries is tolerated and ignored.

## D-002 [HIGH] [USER] Control plane: meshd embeds a localhost HTTP/JSON control API

`mjolnir-meshd` serves a minimal HTTP/JSON API bound to `127.0.0.1:5380`:
- `POST /v0/publish {name, port, txt?}` → 200 with the created entry, or a structured error
  (`reserved`, `owned_by_other {winner_node_id}`) — the synchronous error path FR16/FR34 need.
- `POST /v0/unpublish {name}` → tombstone + 200.
- `GET /v0/directory` → the live name/service/provenance projection (what `status` prints; what the
  bc7 front desk needs).

`meshd publish`/`unpublish`/(`status`, later) become thin HTTP clients of this API. CONVERGENCE
(the reason this shape won, per Duke): bc7's `mjolnir-hello` currently plans file seams
(directory.json out, pending/ spool in) — it can consume `GET /v0/directory` and later `POST` its
identity submissions to the same API, retiring the file seams. One control surface instead of three
mechanisms. Localhost-only bind; never exposed on br-mesh/LAN. Implementation may reuse the tiny
HTTP dependency already accepted for bc7 (`tiny_http`) or hand-roll over tokio — implementer's call
under the NFR1 size budget.

## D-003 [MEDIUM] [USER] `hello.mesh` pre-claim answer: `192.168.1.1`

Before the node claims its /24, well-known names answer with the stock alias `192.168.1.1` (kept
fleet-wide as the second lan address). Resolvable-and-reachable during warmup; flips to the claimed
gateway automatically (TTL 30s bounds staleness).

## D-004 [MEDIUM] [AUTO-DECIDED] Tombstone = separate `ServiceUnpublishV2` gossip variant

Matches the `SubnetClaimRelease` precedent: a new appended variant `{name, owner_node_id, hlc}`.
Tombstones are retained in `services.state` (HLC-ordered) so a late re-announce from a stale peer
loses to the tombstone; the same owner re-publishing with a newer HLC revives the name (FR31).
GC deferred to bead 99f.

## D-005 [LOW] [AUTO-DECIDED] SOA for negative answers

`hello.mesh. ops.hello.mesh. serial=1 refresh=3600 retry=600 expire=86400 minimum=30`. Serial is
static — the zone is a CRDT projection with no transfer semantics; `minimum=30` sets negative-cache
TTL equal to the positive TTL.

## D-006 [LOW] [AUTO-DECIDED] Own-vs-learned restore split

Single `services.state` file. On boot, entries whose `owner_node_id == self` are re-owned (eligible
for anti-entropy re-announce, FR23/FR24); all others are learned state, served but never re-announced.
No second file needed.

## Ceremony

Two HIGH decisions → ceremony finalized at **standard** (work items + this ADR-lite doc + the PRD as
the planning narrative; lean story template).
