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

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO_ROOT}"

FEATURES="${FEATURES:-daemon}"

echo ">> building ${IMAGE} (linux/arm/v7, bin=${BIN}, features=${FEATURES})"
# buildx is required to force the arm/v7 platform on a non-arm host.
docker buildx build \
  --platform linux/arm/v7 \
  --build-arg "BIN=${BIN}" \
  --build-arg "FEATURES=${FEATURES}" \
  -f deploy/mikrotik/Dockerfile \
  -t "${IMAGE}" \
  --load \
  .

echo ">> exporting image to ${OUT_TAR}"
# RouterOS /container imports a 'docker save' tar via file=. If your RouterOS
# version expects a flattened root-fs tar instead, swap to:
#   docker create --platform linux/arm/v7 "${IMAGE}" | xargs -I{} docker export {} -o "${OUT_TAR}"
# (verify against your RouterOS version's container docs — see the runbook).
docker save "${IMAGE}" -o "${OUT_TAR}"

echo ">> done: ${OUT_TAR}"
echo "   size: $(du -h "${OUT_TAR}" | cut -f1)"
