#!/usr/bin/env bash
# Roll an update across the whole fleet, ONE NODE AT A TIME, in-band.
# Wraps install-node.sh (staged payload + detached apply + health-gated
# rollback — see deploy/openwrt/README.md) over the fleet-nodes.conf inventory.
#
# Usage:  deploy/openwrt/update-fleet.sh [install-node options] [node-name ...]
#   deploy/openwrt/update-fleet.sh                          # binary/deps to every node
#   deploy/openwrt/update-fleet.sh --wireless node.env      # + radio config everywhere
#   deploy/openwrt/update-fleet.sh m3000 tr3000             # only these nodes
#
# Design choices a future operator/agent should know:
# - SEQUENTIAL on purpose: never update two nodes at once — the mesh must stay
#   routable so the remaining nodes can still be reached (and so a health-gate
#   rollback has live neighbours to gate against).
# - HALTS on the first failed node instead of marching a broken update across
#   the fleet. The failed node has rolled itself back (or says FAILED with
#   nothing changed); fix, then re-run — applies are idempotent, already-updated
#   nodes are touched-nothing no-ops.
# - Walks fleet-nodes.conf top to bottom: keep it leaf-first, wired jump node
#   last (see the inventory header).
# - Reaching 10.254.x from the workstation needs the jump-host ssh config from
#   the README ("Fleet rollout" section); unreachable nodes are skipped and
#   reported, not fatal — power-cycled/absent nodes are normal.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONF="$DIR/fleet-nodes.conf"
[ -f "$CONF" ] || { echo "inventory missing: $CONF"; exit 1; }

INSTALL_ARGS=()
ONLY=()
while [ $# -gt 0 ]; do
	case "$1" in
		--wireless|--health-timeout|--pkg-cache) INSTALL_ARGS+=("$1" "${2:?$1 needs a value}"); shift 2 ;;
		--stage-only)                            INSTALL_ARGS+=("$1"); shift ;;
		-*)                                      echo "unknown option: $1" >&2; exit 2 ;;
		*)                                       ONLY+=("$1"); shift ;;
	esac
done

want() {
	[ "${#ONLY[@]}" -eq 0 ] && return 0
	local n; for n in "${ONLY[@]}"; do [ "$n" = "$1" ] && return 0; done
	return 1
}

UPDATED=() SKIPPED=()
# Read the inventory on FD 3 — the ssh/scp calls inside the loop would
# otherwise eat the remaining lines from stdin and end the rollout early.
while IFS='|' read -r -u3 name addr node_id model notes; do
	case "$name" in ''|\#*) continue ;; esac
	want "$name" || continue

	echo
	echo "===== $name ($model) — root@$addr ====="
	if ! ssh -o BatchMode=yes -o ConnectTimeout=6 "root@$addr" true 2>/dev/null; then
		echo ">> UNREACHABLE — skipping ($notes)"
		SKIPPED+=("$name")
		continue
	fi

	if ! "$DIR/install-node.sh" "${INSTALL_ARGS[@]+"${INSTALL_ARGS[@]}"}" "root@$addr"; then
		echo
		echo ">> ROLLOUT HALTED at $name — the node rolled back (or FAILED with nothing"
		echo ">> changed). Inspect:  ssh root@$addr 'cat /root/mjolnir-stage/apply.log'"
		echo ">> Already updated this run: ${UPDATED[*]:-none}. Re-running is safe (idempotent)."
		exit 1
	fi

	# Post-check beyond the node's own health gate: it still routes the mesh.
	routes=$(ssh -o BatchMode=yes -o ConnectTimeout=6 "root@$addr" \
		"ip -4 route | grep -c 'via 10\.254' || true" 2>/dev/null || echo 0)
	echo ">> $name: OK (babel routes to $routes neighbour(s))"
	UPDATED+=("$name")
done 3< "$CONF"

echo
echo "===== fleet rollout summary ====="
echo "updated:     ${UPDATED[*]:-none}"
echo "unreachable: ${SKIPPED[*]:-none}"
[ "${#SKIPPED[@]}" -eq 0 ] || echo "(skipped nodes keep their old version — re-run '$0 ${SKIPPED[*]}' when they're back)"
