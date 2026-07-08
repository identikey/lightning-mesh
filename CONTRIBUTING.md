# Contributing to Lightning Mesh

Thanks for your interest in Lightning Mesh! This guide covers how to get set
up, our conventions, and the one legal step (the CLA) every contributor takes.

## Contributor License Agreement (required)

Lightning Mesh is **dual-licensed** — AGPL-3.0-or-later for everyone, plus a
commercial license from **Identikey Inc.** for closed products. That model only
works if the project can license every contribution under both. So **before
your first pull request is merged, you must agree to the
[Contributor License Agreement](CLA.md)**.

- **Individuals:** the CLA bot ([CLA Assistant](https://cla-assistant.io)) will
  comment on your first PR with a one-click sign link. Signing once covers all
  your future contributions. (If the bot is unavailable, you can instead add
  yourself to [`contributors/individual.md`](contributors/individual.md) using
  the statement in CLA.md → *How to sign* → Option A.)
- **Contributing on behalf of an employer:** have an authorized representative
  complete the Corporate CLA (CLA.md → Option B) by emailing
  **duke@identikey.io**.

You keep the copyright to your work — the CLA is a license, not an assignment.

## Development setup

This is a Cargo workspace (`crates/*`). You need a recent stable Rust toolchain.

```bash
cargo build --workspace     # build everything
cargo test --workspace      # run the test suite
cargo clippy --workspace    # lint (please keep it clean)
cargo fmt --all             # format before committing
```

Cross-building the router daemon (`mjolnir-meshd`) as a static aarch64 binary
for OpenWrt hardware:

```bash
deploy/openwrt/build.sh
```

See [`docs/deploy/node-operations.md`](docs/deploy/node-operations.md) for how
nodes are installed and updated (staged payload → detached apply → health-gated
rollback; never mutate a live node inline over SSH).

## Issue tracking: beads (`bd`)

This project tracks work with **beads**, not GitHub issues or TODO lists.

```bash
bd ready            # find available work
bd show <id>        # view an issue
bd update <id> --claim   # claim it before you start
bd close <id>       # when done
```

Reference an issue id in your PR description (e.g. `Refs: lightning-mesh-abc`).
The beads database syncs via the git-committed `.beads/issues.jsonl`.

## Pull request checklist

- [ ] Branch off `main`; keep the PR focused (one logical change).
- [ ] `cargo build --workspace`, `cargo test --workspace`, and
      `cargo clippy --workspace` all pass.
- [ ] `cargo fmt --all` applied.
- [ ] New source files carry the SPDX header (see below).
- [ ] PR description references the relevant beads id.
- [ ] You've agreed to the CLA (the bot will confirm).

## Conventions

- **License headers.** New source files start with:
  ```rust
  // SPDX-License-Identifier: AGPL-3.0-or-later
  // Copyright (C) 2026 Identikey Inc. and the Lightning Mesh contributors
  ```
- **Naming.** The public project name is *Lightning Mesh*; crates, binaries,
  and the overlay interface keep the `mjolnir-` prefix (`mjolnir-meshd`,
  `mjolnir0`). Don't rename these.
- **Architecture guardrails** (see [`CLAUDE.md`](CLAUDE.md) for the full list):
  never bridge client L2 across nodes (breaks broadcast containment); the L3
  overlay is the product and the radio is plumbing; management happens over the
  derived `10.254.x` overlay address, not link-local.

## Reporting security issues

Please do **not** open a public issue for security vulnerabilities. Email
**duke@identikey.io** with details and we'll coordinate a fix and disclosure.

## License of contributions

By contributing, you agree your contributions are licensed under
AGPL-3.0-or-later and, per the [CLA](CLA.md), may also be licensed by Identikey
Inc. under commercial terms. See [`LICENSE`](LICENSE) and
[`COMMERCIAL-LICENSE.md`](COMMERCIAL-LICENSE.md).
