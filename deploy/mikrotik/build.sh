#!/usr/bin/env bash
# Build the linux/arm/v7 OCI image for the MikroTik RouterOS container target
# and export it as a tar for upload to the router.
#
# Board: MikroTik L23UGSR-5HaxD2HaxD (IPQ-5010, ARMv7 32-bit). See
# docs/deploy/mikrotik-routeros-container.md for the on-router steps.
#
# Usage:
#   deploy/mikrotik/build.sh [BIN]
# where BIN is the headless daemon binary name (default: mjolnir-meshd,
# tracked by beads mjolnir-mesh-tr6). Run from the repo root.
set -euo pipefail

BIN="${1:-mjolnir-meshd}"
IMAGE="mjolnir-meshd:armv7"
OUT_TAR="deploy/mikrotik/${BIN}-armv7.tar"
# RouterOS-ready tar (classic docker-save layout, uncompressed layers).
ROS_TAR="deploy/mikrotik/${BIN}-ros.tar"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

FEATURES="${FEATURES:-daemon}"

# Source fingerprint stamped into the binary (mjolnir-mesh-auu). Derived on the
# HOST because `.git` is dockerignored — passed into the build via --build-arg
# and surfaced in meshd's startup banner so two routers can be proven identical.
GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then GIT_SHA="${GIT_SHA}-dirty"; fi

echo ">> building ${IMAGE} -> ${OUT_TAR} (linux/arm/v7, bin=${BIN}, features=${FEATURES}, build=${GIT_SHA})"
# buildx is required to force the arm/v7 platform on a non-arm host.
#
# Output via the buildkit `docker` exporter (type=docker) — this writes a
# legacy docker-archive tarball (root manifest.json referencing Config+Layers),
# which RouterOS /container can import. Do NOT use `--load` + `docker save`:
# on Docker with the containerd image store that emits an OCI-layout tar, which
# RouterOS rejects with "no config found in manifest".
docker buildx build \
  --platform linux/arm/v7 \
  --build-arg "BIN=${BIN}" \
  --build-arg "FEATURES=${FEATURES}" \
  --build-arg "GIT_SHA=${GIT_SHA}" \
  -f deploy/mikrotik/Dockerfile \
  -t "${IMAGE}" \
  --output "type=docker,dest=${OUT_TAR}" \
  .

echo ">> repacking to RouterOS-ready format -> ${ROS_TAR}"
# buildkit emits gzip layers in blobs/sha256/ which RouterOS rejects
# ("could not load next layer"). Repack to classic uncompressed docker-save.
bash "$(dirname "${BASH_SOURCE[0]}")/repack-docker-archive.sh" "${OUT_TAR}" "${ROS_TAR}" "${IMAGE}"

echo ">> done."
echo "   build tar : ${OUT_TAR}  ($(du -h "${OUT_TAR}" | cut -f1))"
echo "   UPLOAD THIS -> ${ROS_TAR}  ($(du -h "${ROS_TAR}" | cut -f1))"
