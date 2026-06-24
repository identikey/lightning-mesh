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

# Derive every node's id + the full peer-id list up front.
declare -A NAME_ID
for row in "${NODES[@]}"; do
  read -r name _ip secret <<<"$row"
  id=$("$BIN" id --no-relay --secret-file "$secret" 2>/dev/null | awk '/node id:/{print $3}')
  [ -n "$id" ] || { echo "could not derive node id for $name from $secret"; exit 1; }
  NAME_ID[$name]="$id"
done

deploy_one() {
  read -r name ip secret <<<"$1"
  local self_id="${NAME_ID[$name]}"
  # --peer args = every OTHER node's id.
  local peers=""
  for row in "${NODES[@]}"; do
    read -r n _ _ <<<"$row"
    [ "$n" = "$name" ] && continue
    peers="$peers --peer ${NAME_ID[$n]}"
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
echo "================ deploy done — check 'endpoint addressable ... 10.254.x' + 'kind=DIRECT' in /log ================"
