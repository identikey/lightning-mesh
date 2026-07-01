#!/usr/bin/env bash
# Push mjolnir-meshd + its init/config to a freshly-flashed OpenWrt node and
# install deps. Idempotent — safe to re-run. See deploy/openwrt/README.md.
#
# RUN OVER ETHERNET / out-of-band. The wpad-basic->wpad-mesh swap and setup-wireless
# bounce wifi; SSHing in over the wifi you're reconfiguring will cut your own session.
#
# Usage:  deploy/openwrt/install-node.sh root@<node-ip>
set -euo pipefail

HOST="${1:?usage: install-node.sh root@<node-ip>}"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$DIR/mjolnir-meshd-aarch64"
[ -f "$BIN" ] || { echo "binary missing — run deploy/openwrt/build.sh first"; exit 1; }

echo ">> static binary -> /usr/bin/mjolnir-meshd"
# scp -O: OpenWrt dropbear has no sftp-server, so default scp (SFTP) fails. Land
# it as .new then mv, so replacing a RUNNING binary doesn't hit ETXTBSY.
scp -O "$BIN" "$HOST:/usr/bin/mjolnir-meshd.new"
ssh "$HOST" 'mv /usr/bin/mjolnir-meshd.new /usr/bin/mjolnir-meshd && chmod +x /usr/bin/mjolnir-meshd'

echo ">> init scripts (meshd + babeld) + wireless helper"
scp -O "$DIR/files/etc/init.d/mjolnir-meshd"  "$HOST:/etc/init.d/mjolnir-meshd"
scp -O "$DIR/files/etc/init.d/mjolnir-babeld" "$HOST:/etc/init.d/mjolnir-babeld"
scp -O "$DIR/setup-wireless.sh"               "$HOST:/root/setup-wireless.sh"
ssh "$HOST" 'chmod +x /etc/init.d/mjolnir-meshd /etc/init.d/mjolnir-babeld /root/setup-wireless.sh'

# UCI config carries node-specific state (peers, client_iface). Install the
# template ONLY if it's missing a meshd section — never clobber a config
# that's actually been customized (would wipe peers), but DO repair a missing,
# empty, or truncated-before-the-section file left by an interrupted prior
# run. `uci show` dumps every section as `pkg.name=type` (named) or
# `pkg.@type[N]=type` (anonymous), so grepping its output for a `=meshd` line
# catches a meshd section under either form — unlike `uci get mjolnir.meshd`,
# which only resolves the named path and would wrongly call an anonymous
# `config meshd` (no name token) "missing" and clobber it.
echo ">> uci config (template only if missing a meshd section — preserves existing peers/config)"
if ssh "$HOST" "uci -q show mjolnir 2>/dev/null | grep -q '=meshd\$'"; then
  echo "   /etc/config/mjolnir has a meshd section — left as-is"
else
  scp -O "$DIR/files/etc/config/mjolnir" "$HOST:/etc/config/mjolnir"
fi

# apk (OpenWrt 25.12+) vs opkg (older releases) package-manager branch, shared
# by the babeld/kmod-tun/wpad install steps below so a future tweak to the
# detection only needs to change one place.
PM_HELPERS='
pm_update() { if command -v apk >/dev/null 2>&1; then apk update; else opkg update; fi; }
pm_install() { if command -v apk >/dev/null 2>&1; then apk add "$1"; else opkg install "$1"; fi; }
pm_remove() { if command -v apk >/dev/null 2>&1; then apk del "$1" 2>/dev/null; else opkg remove "$1" 2>/dev/null; fi; }
'

echo ">> deps: babeld (required)"
# Needs the node to have internet. babeld is unconditionally required for the
# mesh to route at all, so its failure is fatal — but say so plainly rather
# than letting a bare `set -e` abort with only the remote package manager's
# own error text.
if ! ssh "$HOST" "$PM_HELPERS"'
set -e
pm_update
pm_install babeld'; then
  echo ">> FATAL: babeld install failed on $HOST — babeld is required for the mesh to route. Fix the node's package feeds/connectivity and re-run install-node.sh." >&2
  exit 1
fi

echo ">> deps: kmod-tun (REQUIRED for iroh tunnels: lan_tunnels=1 or --internet)"
if ssh "$HOST" "$PM_HELPERS"'pm_install kmod-tun'; then
  KMOD_TUN_OK=1
else
  KMOD_TUN_OK=0
fi

echo ">> wpad-mesh-mbedtls (802.11s SAE) — swaps stock wpad-basic-mbedtls, which lacks mesh"
# Removing wpad bounces wifi; fine — nodes are managed out-of-band over eth. Open mesh
# (no MESH_KEY) needs none of this; only SAE backhaul requires the mesh-capable wpad.
# Install the replacement BEFORE removing the stock package: if the index
# refresh or install fails, the node keeps wpad-basic-mbedtls rather than
# being left with neither installed.
ssh "$HOST" "$PM_HELPERS"'
pm_update
if pm_install wpad-mesh-mbedtls; then
  pm_remove wpad-basic-mbedtls
else
  echo "WARN: wpad-mesh-mbedtls install failed — left wpad-basic-mbedtls in place (SAE mesh wont auth; open mesh and existing client APs still work)"
fi'

echo ">> babeld lifecycle -> procd (m8t): disable the stock babeld service, use mjolnir-babeld"
ssh "$HOST" '
/etc/init.d/babeld disable 2>/dev/null; /etc/init.d/babeld stop 2>/dev/null
/etc/init.d/mjolnir-babeld enable 2>/dev/null
echo "  babeld now supervised by procd via mjolnir-babeld (started/reloaded by meshd)"'

echo ">> enable meshd service (won't start until you set peers in /etc/config/mjolnir)"
ssh "$HOST" '/etc/init.d/mjolnir-meshd enable'

if [ "$KMOD_TUN_OK" != "1" ]; then
  echo ">> WARNING: kmod-tun failed to install on $HOST — iroh tunnels (lan_tunnels=1, --internet) will NOT work (/dev/net/tun missing) until this is fixed and install-node.sh is re-run."
fi

cat <<EOF
>> done on $HOST. Next:
   1. ssh $HOST 'mjolnir-meshd id --secret-file /etc/mjolnir/secret'   # this node's id
   2. add the OTHER nodes' ids to /etc/config/mjolnir   (list peer '<id>')
   3. set backhaul_iface: 'br-lan' for the wired-switch bench, or run
      /root/setup-wireless.sh then set it to 'br-mesh'
   4. ssh $HOST 'service mjolnir-meshd start && logread -e mjolnir_meshd'
EOF
