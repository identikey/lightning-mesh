#!/usr/bin/env bash
# Stage mjolnir-meshd + deps on an OpenWrt node, then apply DETACHED with a
# health-gated auto-rollback (mjolnir-mesh-6e5). Idempotent — safe to re-run,
# and re-runs no longer bounce wifi (the wpad swap is skipped once done).
#
# Safe to run IN-BAND — over the 802.11s mesh or the wifi being reconfigured.
# Everything is staged first (including prefetched packages, so no internet is
# needed mid-apply), then /root/mjolnir-stage/mjolnir-apply runs on the node
# under setsid: your SSH session dying during the wifi bounce doesn't matter.
# The applier health-checks the mesh afterwards and rolls back to the previous
# config/binary/wpad if it doesn't come back; this script polls for the result.
# Ethernet at 192.168.1.1 is the recovery of last resort, not a requirement.
#
# Usage:  deploy/openwrt/install-node.sh [options] root@<node-ip>
#   --wireless FILE      also run setup-wireless.sh under the applier, with FILE
#                        sourced as its env (MESH_ID/MESH_KEY/CLIENT_KEY/...)
#   --stage-only         stage everything but don't trigger the apply
#   --health-timeout N   seconds the applier waits for the mesh to come back
#                        before rolling back (default 120)
#   --pkg-cache DIR      local package cache (default deploy/openwrt/pkg-cache).
#                        Filled from any node that can reach the feeds; pushed
#                        to nodes that can't (e.g. a fresh box with no WAN).
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$DIR/mjolnir-meshd-aarch64"
STAGE=/root/mjolnir-stage
PKGS="babeld kmod-tun wpad-mesh-mbedtls wpad-basic-mbedtls"  # basic variant = rollback fuel for the wpad swap
# Drivers for every supported USB dongle ride along on every node (fleet
# plug-and-play: a dongle plugged in the FIELD needs no download). The table
# in files/usr/sbin/mjolnir-dongle is the single source of truth.
PKGS="$PKGS $(sh "$DIR/files/usr/sbin/mjolnir-dongle" packages)"

HOST=""
WIRELESS_ENV=""
STAGE_ONLY=0
HEALTH_TIMEOUT=120
PKG_CACHE="$DIR/pkg-cache"
while [ $# -gt 0 ]; do
	case "$1" in
		--wireless)       WIRELESS_ENV="${2:?--wireless needs a file}"; shift 2 ;;
		--stage-only)     STAGE_ONLY=1; shift ;;
		--health-timeout) HEALTH_TIMEOUT="${2:?--health-timeout needs seconds}"; shift 2 ;;
		--pkg-cache)      PKG_CACHE="${2:?--pkg-cache needs a dir}"; shift 2 ;;
		-*)               echo "unknown option: $1" >&2; exit 2 ;;
		*)                HOST="$1"; shift ;;
	esac
done
[ -n "$HOST" ] || { echo "usage: install-node.sh [options] root@<node-ip>"; exit 2; }
[ -f "$BIN" ] || { echo "binary missing — run deploy/openwrt/build.sh first"; exit 1; }
[ -z "$WIRELESS_ENV" ] || [ -f "$WIRELESS_ENV" ] || { echo "--wireless file not found: $WIRELESS_ENV"; exit 1; }

# ---- stage: push everything (scp -O: dropbear has no sftp-server) ------------
echo ">> staging payload -> $HOST:$STAGE"
ssh "$HOST" "mkdir -p $STAGE/pkgs"
scp -O "$BIN"                                  "$HOST:$STAGE/mjolnir-meshd"
scp -O "$DIR/files/etc/init.d/mjolnir-meshd"   "$HOST:$STAGE/init.d-mjolnir-meshd"
scp -O "$DIR/files/etc/init.d/mjolnir-babeld"  "$HOST:$STAGE/init.d-mjolnir-babeld"
scp -O "$DIR/files/etc/config/mjolnir"         "$HOST:$STAGE/config-mjolnir"
scp -O "$DIR/setup-wireless.sh"                "$HOST:$STAGE/setup-wireless.sh"
scp -O "$DIR/files/usr/sbin/mjolnir-apply"     "$HOST:$STAGE/mjolnir-apply"
scp -O "$DIR/files/usr/sbin/mjolnir-dongle"    "$HOST:$STAGE/mjolnir-dongle"
scp -O "$DIR/files/etc/hotplug.d/usb/70-mjolnir-dongle" "$HOST:$STAGE/hotplug-usb-mjolnir-dongle"
ssh "$HOST" "chmod +x $STAGE/mjolnir-apply"

RUN_WIRELESS=0
if [ -n "$WIRELESS_ENV" ]; then
	scp -O "$WIRELESS_ENV" "$HOST:$STAGE/wireless.env"
	RUN_WIRELESS=1
fi

# ---- prefetch packages on the node while connectivity is still good ----------
# The apply may bounce the radio that IS this node's egress (mesh-fed nodes),
# so packages must be local files before anything disruptive starts. apk names
# files name-version.apk, opkg names them name_version_arch.ipk.
echo ">> prefetching packages on the node ($PKGS)"
MISSING=$(ssh "$HOST" "
cd $STAGE/pkgs
if command -v apk >/dev/null 2>&1; then apk update >/dev/null 2>&1 || true; else opkg update >/dev/null 2>&1 || true; fi
missing=''
for p in $PKGS; do
	ls \"\$p\"-[0-9]*.apk \"\$p\"_*.ipk >/dev/null 2>&1 && continue
	if command -v apk >/dev/null 2>&1; then
		# -R pulls dependencies too (kmods need their -lib/firmware deps for a
		# no-network install later); fall back for apk builds without it
		apk fetch -R -o . \"\$p\" >/dev/null 2>&1 || apk fetch -o . \"\$p\" >/dev/null 2>&1 || missing=\"\$missing \$p\"
	else
		opkg download \"\$p\" >/dev/null 2>&1 || missing=\"\$missing \$p\"
	fi
done
echo \"\$missing\"")

# Top up the node's stage from the local cache: push EVERY cached file the
# node doesn't have staged, not just the packages that failed to fetch —
# 'apk add --no-network' resolves dependencies only among files it can see,
# so kmod dependency CLOSURES must be present as staged files on feed-less
# nodes (mjolnir-mesh-9dj). Staging is cheap and harmless; only packages
# named by the applier ever get INSTALLED.
REMOTE_HAVE=$(ssh "$HOST" "ls $STAGE/pkgs 2>/dev/null" || true)
TO_PUSH=()
for f in "$PKG_CACHE"/*.apk "$PKG_CACHE"/*.ipk; do
	[ -f "$f" ] || continue
	grep -qxF "$(basename "$f")" <<<"$REMOTE_HAVE" || TO_PUSH+=("$f")
done
if [ "${#TO_PUSH[@]}" -gt 0 ]; then
	echo "   pushing ${#TO_PUSH[@]} cached package(s) the node lacks (incl. dependency closures)"
	scp -O "${TO_PUSH[@]}" "$HOST:$STAGE/pkgs/"
fi

# Anything REQUIRED but still nowhere (not fetched, not in cache, not installed)?
if [ -n "${MISSING// /}" ]; then
	for p in $MISSING; do
		ls "$PKG_CACHE/$p"-[0-9]*.apk "$PKG_CACHE/$p"_*.ipk >/dev/null 2>&1 && continue
		if ssh "$HOST" "command -v apk >/dev/null 2>&1 && apk info -e '$p' >/dev/null 2>&1 || opkg list-installed 2>/dev/null | grep -q '^$p '"; then
			echo "   $p: not prefetched, but already installed on the node — ok"
		else
			echo "   WARN: $p unavailable (no feeds on node, not in cache, not installed)."
			echo "         The applier will warn/skip or refuse depending on the package."
		fi
	done
fi

# Pull whatever the node has back into the cache, so the NEXT no-WAN box works.
mkdir -p "$PKG_CACHE"
scp -O "$HOST:$STAGE/pkgs/*" "$PKG_CACHE/" 2>/dev/null || true

# ---- applier parameters -------------------------------------------------------
ssh "$HOST" "cat > $STAGE/apply.env" <<EOF
HEALTH_TIMEOUT=$HEALTH_TIMEOUT
RUN_WIRELESS=$RUN_WIRELESS
EOF

if [ "$STAGE_ONLY" = 1 ]; then
	cat <<EOF
>> staged only. Trigger the apply yourself with:
     ssh $HOST 'setsid $STAGE/mjolnir-apply </dev/null >$STAGE/apply.log 2>&1 &'
   then watch for $STAGE/result (OK / ROLLED_BACK / FAILED).
EOF
	exit 0
fi

# ---- detach the applier: it must survive this SSH session dying ---------------
echo ">> launching detached apply (health timeout ${HEALTH_TIMEOUT}s)"
ssh "$HOST" "rm -f $STAGE/result
if command -v setsid >/dev/null 2>&1; then
	(setsid $STAGE/mjolnir-apply </dev/null >$STAGE/apply.log 2>&1 &)
else
	(nohup $STAGE/mjolnir-apply </dev/null >$STAGE/apply.log 2>&1 &)
fi"

# ---- poll for the result over reconnecting SSH --------------------------------
# The connection WILL drop if the apply bounces the radio we're riding — that's
# expected; keep reconnecting until the applier reports, or we give up.
echo ">> waiting for result (SSH may drop and reconnect — that's normal in-band)"
DEADLINE=$(( $(date +%s) + HEALTH_TIMEOUT + 240 ))
RES=""
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
	RES=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$HOST" "cat $STAGE/result 2>/dev/null" 2>/dev/null || true)
	[ -n "$RES" ] && break
	sleep 5
done

if [ -z "$RES" ]; then
	cat <<EOF
>> NO RESULT after $((HEALTH_TIMEOUT + 240))s. The node may still be converging,
   or it rolled back onto a config this machine can't reach from here.
   - retry:            ssh $HOST 'cat $STAGE/result; tail -40 $STAGE/apply.log'
   - recovery of last resort: ethernet LAN port, root@192.168.1.1
EOF
	exit 1
fi

echo ">> $RES"
echo ">> apply log tail:"
ssh -o BatchMode=yes -o ConnectTimeout=5 "$HOST" "tail -25 $STAGE/apply.log" 2>/dev/null || true

case "$RES" in
	OK*)
		cat <<EOF
>> done on $HOST. Next (fresh node only):
   1. ssh $HOST 'mjolnir-meshd id --secret-file /etc/mjolnir/secret'   # this node's id
   2. add the OTHER nodes' ids to /etc/config/mjolnir   (list peer '<id>')
   3. set backhaul_iface: 'br-lan' for the wired-switch bench, or pass
      --wireless <env-file> next run (or run /root/setup-wireless.sh) then 'br-mesh'
   4. ssh $HOST 'service mjolnir-meshd start && logread -e mjolnir_meshd'
EOF
		;;
	*)
		echo ">> apply did NOT stick — the node restored its previous config. Full log:"
		echo "     ssh $HOST 'cat $STAGE/apply.log'"
		exit 1
		;;
esac
