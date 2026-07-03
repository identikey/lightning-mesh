#!/usr/bin/env bash
# Fleet-wide post-rollout validation: build stamp, effective backhaul address
# vs the inventory, backhaul /32 claim convergence (pt9), and collision events.
# Read-only; safe to run any time. Pairs with update-fleet.sh.
#
# Lessons baked in (learned the hard way during the pt9 rollout):
# - identity via `service mjolnir-meshd diag` (explicit --secret-file from UCI).
#   Bare `mjolnir-meshd status` is now also safe: it resolves the UCI
#   secret_file itself and prints `node id: UNKNOWN` when none is found, rather
#   than inventing an EPHEMERAL identity with a plausible-but-wrong backhaul
#   address (the pt9 friction that filed bead dbv, since fixed). diag stays the
#   convention for consistency with the service.
# - logread output carries ANSI escapes that break naive greps (bead 3xb);
#   strip them before matching.
# - the inventory is read on FD 3 — ssh inside the loop would otherwise eat
#   the remaining lines from stdin (same guard as update-fleet.sh).
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONF="$DIR/fleet-nodes.conf"
[ -f "$CONF" ] || { echo "inventory missing: $CONF"; exit 1; }

FLEET=0 FAIL=0
while IFS='|' read -r -u3 name addr node_id model notes; do
	case "$name" in ''|\#*) continue ;; esac
	FLEET=$((FLEET + 1))
	echo "===== $name (expect $addr)"
	out=$(ssh -o BatchMode=yes -o ConnectTimeout=6 "root@$addr" '
		service mjolnir-meshd diag 2>/dev/null | grep -E "build:|node id:|backhaul:" | head -3
		L=$(logread 2>/dev/null | sed "s/\x1b\[[0-9;]*m//g")
		echo "peer /32 claims seen: $(echo "$L" | grep "received peer subnet claim" | grep -o "cidr=10\.254\.[0-9.]*/32" | sort -u | wc -l | tr -d " ")"
		echo "backhaul collisions:  $(echo "$L" | grep -c "collision lost" | tr -d " ")"
		echo "babel routes:         $(ip -4 route | grep -c "via 10\.254" | tr -d " ")"
	' 2>/dev/null) || { echo "  UNREACHABLE"; FAIL=1; continue; }
	echo "$out" | sed 's/^/  /'
	echo "$out" | grep -q "backhaul: $addr/" || {
		echo "  !! effective backhaul differs from inventory ($addr) —"
		echo "  !! either a pt9 re-derivation (update fleet-nodes.conf) or a real problem"
		FAIL=1
	}
done 3< "$CONF"

echo
if [ "$FAIL" -eq 0 ]; then
	echo "VALIDATION OK across $FLEET node(s)"
else
	echo "VALIDATION FAILURES above"
	exit 1
fi
