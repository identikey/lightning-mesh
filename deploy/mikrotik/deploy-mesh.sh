#!/usr/bin/env bash
# Deploy the mjolnir mesh to a set of MikroTik RouterOS containers in the
# CURRENT default mode: offline LAN backhaul (`mesh --lan`, mjolnir-mesh-4pk).
#
# Prerequisites on each router (do these once, by hand — see
# docs/deploy/mesh-bringup.md):
#   1. RouterOS 7.23.1 + container + wifi-qcom packages (deploy/mikrotik/fetch-firmware.sh)
#   2. device-mode=container enabled (physical reset-button hold at boot)
#   3. container-net-lan.rsc applied with $meshLink set to the shared-segment
#      port — this bridges the container onto the L2 the other nodes are on, so
#      meshd's self-assigned 10.254.0.0/16 backhaul address is mutually reachable
#      and peers form DIRECT iroh paths (no relay; sidesteps mjolnir-mesh-67h).
#
# This script does the per-node container lifecycle: scp the image tar, set the
# persistent IROH_SECRET, (re)create the container running `mesh --peer <others>`,
# and start it. The same cmd now runs in --lan mode by default; pass MESH_INTERNET=1
# to opt into the relay/internet path (needs container-net.rsc instead).
#
# STATUS: the --lan backhaul MECHANISM is validated on armv7 Linux containers
# (4pk). The MikroTik shared-L2 bring-up (container-net-lan.rsc) is not yet
# hardware-validated — treat this as a bench helper, not a turnkey tool. The
# durable home for this orchestration is meshctl (see the meshctl follow-up bead).
set -uo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

TAR="${MESH_TAR:-deploy/mikrotik/mjolnir-meshd-ros.tar}"
BIN="${MESHD:-target/debug/mjolnir-meshd}"   # used only to derive node ids from secrets
SSH="ssh -o BatchMode=yes -o ConnectTimeout=8"
EXTRA_ARGS=""
[ "${MESH_INTERNET:-0}" = "1" ] && EXTRA_ARGS="--internet"

# Node table: name  mgmt-ip  secret-file. Edit to match your swarm (or wire this
# to routers.toml). meshd derives each node's id from its persistent secret.
NODES=(
  "router-1  192.168.0.181  deploy/mikrotik/secrets/router-1.secret"
  "router-2  192.168.0.113  deploy/mikrotik/secrets/router-2.secret"
)

[ -f "$TAR" ] || { echo "missing image tar: $TAR (build with deploy/mikrotik/build.sh)"; exit 1; }
[ -x "$BIN" ] || { echo "missing meshd binary: $BIN (cargo build --bin mjolnir-meshd --features daemon)"; exit 1; }

# Identity of what we're about to ship to EVERY node (mjolnir-mesh-auu). The
# same tar goes to all routers, so its sha256 is the deploy fingerprint; the
# git stamp is what meshd prints in its startup banner. After deploy we read
# each node's banner back and assert they all match this — turning "they should
# be identical" into a checked fact instead of an assumption.
TAR_SHA="$(shasum -a 256 "$TAR" | awk '{print $1}')"
EXPECT_BUILD="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
[ -n "$(git status --porcelain 2>/dev/null)" ] && EXPECT_BUILD="${EXPECT_BUILD}-dirty"
echo ">> shipping $TAR"
echo "   sha256       : $TAR_SHA"
echo "   expect build : $EXPECT_BUILD  (must match build= in every node's banner)"
case "$EXPECT_BUILD" in
  *-dirty) echo "   WARNING: tree is DIRTY — rebuild the tar (build.sh) so it matches HEAD before trusting this deploy";;
esac

# Derive every node's id up front into parallel indexed arrays (bash 3.2 on
# macOS has no associative arrays).
NAMES=(); IDS=()
for row in "${NODES[@]}"; do
  read -r name _ip secret <<<"$row"
  id=$("$BIN" id --no-relay --secret-file "$secret" 2>/dev/null | awk '/node id:/{print $3}')
  [ -n "$id" ] || { echo "could not derive node id for $name from $secret"; exit 1; }
  NAMES+=("$name"); IDS+=("$id")
done
id_for() { local i; for i in "${!NAMES[@]}"; do [ "${NAMES[$i]}" = "$1" ] && { printf '%s' "${IDS[$i]}"; return; }; done; }

deploy_one() {
  read -r name ip secret <<<"$1"
  local self_id; self_id=$(id_for "$name")
  # --peer args = every OTHER node's id.
  local peers="" i
  for i in "${!NAMES[@]}"; do
    [ "${NAMES[$i]}" = "$name" ] && continue
    peers="$peers --peer ${IDS[$i]}"
  done
  local secret_hex; secret_hex=$(tr -d '[:space:]' < "$secret")
  echo "================ $name ($ip) — self=$self_id ================"
  scp -O -o BatchMode=yes -o ConnectTimeout=8 "$TAR" "admin@$ip:" || { echo "SCP FAILED"; return 1; }
  $SSH "admin@$ip" '/container/stop  [find where comment="mjolnir-meshd"]' 2>&1 | head -1; sleep 2
  $SSH "admin@$ip" '/container/remove [find where comment="mjolnir-meshd"]' 2>&1 | head -1; sleep 2
  $SSH "admin@$ip" '/container/envs/remove [find where list="mjolnir-env"]' 2>&1 | head -1
  $SSH "admin@$ip" "/container/envs/add list=\"mjolnir-env\" key=\"IROH_SECRET\" value=\"$secret_hex\"" 2>&1 | head -1
  $SSH "admin@$ip" "/container/add file=\"$(basename "$TAR")\" interface=veth-mesh root-dir=\"mjolnir\" cmd=\"mesh${peers}${EXTRA_ARGS:+ $EXTRA_ARGS}\" envlist=\"mjolnir-env\" comment=\"mjolnir-meshd\" logging=yes start-on-boot=yes" 2>&1 | head -2
  sleep 10
  $SSH "admin@$ip" '/container/start [find where comment="mjolnir-meshd"]' 2>&1 | head -1; sleep 3
  $SSH "admin@$ip" '/container/print where comment="mjolnir-meshd"' 2>&1 | grep -iE 'running|stopped|mjolnir' | head -2
}

for row in "${NODES[@]}"; do deploy_one "$row"; done

# Verify every node booted the SAME build (mjolnir-mesh-auu). meshd logs a
# `mjolnir-meshd starting ... build=<sha>` banner; with logging=yes it lands in
# RouterOS /log. Read it back from each node and assert all stamps == EXPECT_BUILD.
echo "================ verifying build stamps (give meshd a few seconds to boot) ================"
sleep 6
mismatch=0
for row in "${NODES[@]}"; do
  read -r name ip _ <<<"$row"
  line=$($SSH "admin@$ip" '/log/print where message~"mjolnir-meshd starting"' 2>/dev/null | tail -1)
  stamp=$(printf '%s' "$line" | sed -n 's/.*build=\([^ ,]*\).*/\1/p')
  if [ "$stamp" = "$EXPECT_BUILD" ] && [ -n "$stamp" ]; then
    echo "  OK   $name ($ip): build=$stamp"
  else
    echo "  FAIL $name ($ip): build=${stamp:-<no banner in /log yet>} (expected $EXPECT_BUILD)"
    mismatch=1
  fi
done
if [ "$mismatch" = 0 ]; then
  echo ">> ALL NODES IDENTICAL: build=$EXPECT_BUILD  (sha256 $TAR_SHA)"
else
  echo ">> SKEW OR UNVERIFIED — nodes are NOT provably identical. Re-run deploy, or check /log on the failing node."
fi
echo "================ deploy done — check 'endpoint addressable ... 10.254.x' + 'kind=DIRECT' in /log ================"
