#!/usr/bin/env bash
# Build mjolnir-hello (the hello.mesh front desk) as a static aarch64 musl
# binary for OpenWrt mt76 nodes, mirroring build.sh for mjolnir-meshd. OPTIONAL
# — a node runs the mesh fine without this binary (S7, bead mjolnir-mesh-eei).
#
# ORDERING DEPENDENCY (must run in this order): the SvelteKit frontend has to
# be built and synced into crates/mjolnir-hello/static/ BEFORE the Rust
# cross-build, because rust-embed (crates/mjolnir-hello/src/assets.rs)
# compiles the CONTENTS of that directory into the binary at build time — a
# stale or empty static/ bakes a stale or empty frontend into the binary.
#
# Usage:  deploy/openwrt/build-hello.sh   (no args). Run anywhere.
set -euo pipefail

TARGET="aarch64-unknown-linux-musl"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"
OUT="deploy/openwrt/mjolnir-hello-aarch64"
WEB_DIR="hello-mesh-web"

# 1. frontend build + embed sync (bun run build:embed = vite build + sync-embed.js
#    copying hello-mesh-web/build/ -> crates/mjolnir-hello/static/). Skippable
#    with SKIP_WEB=1 if static/ is already fresh (e.g. CI staged it separately).
if [ "${SKIP_WEB:-0}" != 1 ]; then
	command -v bun >/dev/null 2>&1 || { echo "bun not found — install it or set SKIP_WEB=1 with static/ pre-populated" >&2; exit 1; }
	echo ">> building hello-mesh-web frontend and syncing into crates/mjolnir-hello/static/"
	(cd "${WEB_DIR}" && bun install --frozen-lockfile && bun run build:embed)
else
	echo ">> SKIP_WEB=1 — assuming crates/mjolnir-hello/static/ is already fresh"
fi
[ -f "crates/mjolnir-hello/static/index.html" ] || { echo "crates/mjolnir-hello/static/index.html missing after frontend build — aborting cross-build" >&2; exit 1; }

# 2. cross-build the Rust binary, embedding whatever now sits in static/.
#    Same messense/rust-musl-cross image + CROSS_TARGET isolation as build.sh
#    (mjolnir-mesh-0xu) — see that script's comments for why.
CROSS_TARGET="target/openwrt-cross"

echo ">> building mjolnir-hello for ${TARGET}"
docker run --rm \
  -v "${REPO_ROOT}:/work" -w /work \
  -e CARGO_TARGET_DIR="/work/${CROSS_TARGET}" \
  messense/rust-musl-cross:aarch64-musl \
  cargo build --release --locked --target "${TARGET}" \
    -p mjolnir-hello --bin mjolnir-hello

cp "${CROSS_TARGET}/${TARGET}/release/mjolnir-hello" "${OUT}"
echo ">> done -> ${OUT}  ($(du -h "${OUT}" | cut -f1))"
file "${OUT}" 2>/dev/null || true
echo ">> install-node.sh / update-fleet.sh will pick this up automatically and"
echo "   stage it (front desk stays off until 'option enabled 1' in the 'hello'"
echo "   section of /etc/config/mjolnir)."
