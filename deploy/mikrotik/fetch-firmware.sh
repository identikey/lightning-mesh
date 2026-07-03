#!/usr/bin/env bash
# Re-download the official MikroTik RouterOS packages used to provision a router
# (the RouterOS upgrade package + the `container` and `wifi-qcom` extra packages
# that enable device-mode containers and WiFi). These are large MikroTik
# binaries — gitignored, NOT committed — so fetch them on a fresh clone.
#
# They are flashed onto the ROUTER (upload to Files + reboot); they are NOT
# inputs to the container image build. See docs/archive/mikrotik-container/mikrotik-routeros-container.md.
#
# Usage:
#   deploy/mikrotik/fetch-firmware.sh [VERSION] [ARCH]
# Defaults: VERSION=7.23.1, ARCH=arm (the L23UGSR / IPQ-5010 / ARMv7 board).
# Confirm the version against each router's `/system/resource/print`.
set -euo pipefail

VER="${1:-7.23.1}"
ARCH="${2:-arm}"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE="https://download.mikrotik.com/routeros/${VER}"
cd "$DIR"

echo ">> routeros-${VER}-${ARCH}.npk (main upgrade package)"
curl -fSL --proto '=https' -o "routeros-${VER}-${ARCH}.npk" \
  "${BASE}/routeros-${VER}-${ARCH}.npk"

echo ">> all_packages-${ARCH}-${VER}.zip (source of container + wifi-qcom)"
tmpzip="$(mktemp -t mtik-pkgs.XXXXXX).zip"
trap 'rm -f "$tmpzip"' EXIT
curl -fSL --proto '=https' -o "$tmpzip" "${BASE}/all_packages-${ARCH}-${VER}.zip"
# Extract only the two extra packages we need (the zip also holds wifi-qcom-ac
# and others we don't use).
unzip -o "$tmpzip" \
  "container-${VER}-${ARCH}.npk" \
  "wifi-qcom-${VER}-${ARCH}.npk" -d "$DIR" >/dev/null

echo ">> done. RouterOS ${VER}/${ARCH} packages in $DIR:"
ls -1 "routeros-${VER}-${ARCH}.npk" "container-${VER}-${ARCH}.npk" "wifi-qcom-${VER}-${ARCH}.npk"
