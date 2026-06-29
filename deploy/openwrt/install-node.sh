#!/usr/bin/env bash
# Push mjolnir-meshd + its init/config to a freshly-flashed OpenWrt node and
# install deps. Idempotent — safe to re-run. See deploy/openwrt/README.md.
#
# Usage:  deploy/openwrt/install-node.sh root@<node-ip>
set -euo pipefail

HOST="${1:?usage: install-node.sh root@<node-ip>}"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$DIR/mjolnir-meshd-aarch64"
[ -f "$BIN" ] || { echo "binary missing — run deploy/openwrt/build.sh first"; exit 1; }

echo ">> static binary -> /usr/bin/mjolnir-meshd"
scp "$BIN" "$HOST:/usr/bin/mjolnir-meshd"
ssh "$HOST" 'chmod +x /usr/bin/mjolnir-meshd'

echo ">> init script + uci config + wireless helper"
scp "$DIR/files/etc/init.d/mjolnir-meshd" "$HOST:/etc/init.d/mjolnir-meshd"
scp "$DIR/files/etc/config/mjolnir"       "$HOST:/etc/config/mjolnir"
scp "$DIR/setup-wireless.sh"              "$HOST:/root/setup-wireless.sh"
ssh "$HOST" 'chmod +x /etc/init.d/mjolnir-meshd /root/setup-wireless.sh'

echo ">> deps: babeld (required) + kmod-tun (only for cross-site iroh tunnels — best-effort)"
# OpenWrt 25.12+ uses apk; older releases use opkg. Needs the node to have internet.
ssh "$HOST" '
if command -v apk >/dev/null 2>&1; then
  apk update && apk add babeld && apk add kmod-tun || echo "WARN: babeld installed; kmod-tun skipped (not needed for the LAN bench)"
else
  opkg update && opkg install babeld && opkg install kmod-tun || echo "WARN: babeld installed; kmod-tun skipped (not needed for the LAN bench)"
fi'

echo ">> enable service (won't start until you set peers in /etc/config/mjolnir)"
ssh "$HOST" '/etc/init.d/mjolnir-meshd enable'

cat <<EOF
>> done on $HOST. Next:
   1. ssh $HOST 'mjolnir-meshd id --secret-file /etc/mjolnir/secret'   # this node's id
   2. add the OTHER nodes' ids to /etc/config/mjolnir   (list peer '<id>')
   3. set backhaul_iface: 'br-lan' for the wired-switch bench, or run
      /root/setup-wireless.sh then set it to 'br-mesh'
   4. ssh $HOST 'service mjolnir-meshd start && logread -e mjolnir_meshd'
EOF
