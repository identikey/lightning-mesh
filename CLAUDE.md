# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:ca08a54f -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
   (Beads sync via the git-committed `.beads/issues.jsonl`; dolt is embedded with no
   remote, so do NOT run `bd dolt push`.)
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->


## Build & Test

```bash
cargo build --workspace          # Cargo workspace (crates/*)
cargo test --workspace
cargo clippy
deploy/openwrt/build.sh          # cross-build static aarch64 mjolnir-meshd for routers
```

## Architecture Overview

A decentralized router mesh: symmetric, non-authoritative nodes; **the L3
overlay (iroh + babeld + CRDT) is the product, the radio is plumbing**.

- `crates/mjolnir-mesh` — `mjolnir-meshd`, the OpenWrt router daemon:
  802.11s backhaul (`br-mesh`), derived `10.254.<blake3(node_id)>/16`
  overlay addressing, babel routing, single overlay TUN `mjolnir0` for
  cross-site iroh traffic (bead `buw`).
- `crates/mjolnir-node` — desktop/VM mesh daemon (membership, gossip, rooms).
- `crates/mjolnir-meshctl` — `meshctl`, operator-side RouterOS reconciler.
- `crates/mjolnir-audio`, `mjolnir-media`, `mjolnir-moq` — voice/media over
  the mesh.
- `deploy/openwrt/` — fleet install/update: staged payload + detached apply
  with health-gated rollback (in-band safe; ethernet at `192.168.1.1` is
  recovery of last resort). See `docs/deploy/node-operations.md`.
- Design docs live in `docs/network-coordination/` and `docs/vision/`;
  decisions are tracked in beads (`bd show <id>`).

## Conventions & Patterns

- The management plane is the overlay: reach nodes at their derived
  `10.254.x` address over SSH; keep a maintainer inventory of node ids.
- No mDNS for mesh-wide/management discovery — gossip/CRDT address book
  (`buw.9`/`0yb`) is the direction; mDNS is link-local bootstrap only.
- Never bridge client L2 segments across nodes (breaks broadcast
  containment); each node owns its routed `/24`.
- Disruptive node changes go through `mjolnir-apply` (snapshot → apply →
  health gate → rollback), never through a live SSH session doing the
  mutation inline.
