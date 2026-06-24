#!/usr/bin/env bash
# Validate cross-mesh client routing: apply client-routing.rsc on every node,
# give each node a stand-in "client" IP inside its own claimed /24 on a dummy
# bridge, then ping across the mesh. 10.42.0.0/16 exists ONLY inside the mesh, so
# a reply proves the packet crossed it (babeld-installed route over the TUN).
#
# Backhaul-MODE-INDEPENDENT: babeld routes the client /24s over the per-peer TUN
# /31 overlay, which is the same whether the underlay is the --lan direct path
# (4pk) or the --internet relay path. So this test applies to either.
#
# Each node's /24 is deterministic from its node id (see its 'claimed client
# subnet' log line). The HOSTn values below are for the .181/.113 example swarm;
# update them to your nodes' claimed /24s.
set -uo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RSC=deploy/mikrotik/client-routing.rsc
SSH="ssh -o BatchMode=yes -o ConnectTimeout=8"
A=192.168.0.181 ; HOSTA=10.42.23.1      # router-1 claims 10.42.23.0/24
B=192.168.0.113 ; HOSTB=10.42.3.1       # router-2 claims 10.42.3.0/24

setup_router() {
  local ip="$1" host="$2"
  echo ">> [$ip] upload + import client-routing.rsc"
  scp -O -o BatchMode=yes -o ConnectTimeout=8 "$RSC" "admin@$ip:" || return 1
  $SSH "admin@$ip" '/import file-name=client-routing.rsc' 2>&1 | tail -2
  echo ">> [$ip] add br-client + stand-in client $host/24"
  $SSH "admin@$ip" ':if ([:len [/interface/bridge/find where name="br-client"]]=0) do={/interface/bridge/add name=br-client}' 2>&1 | head -1
  $SSH "admin@$ip" ":if ([:len [/ip/address/find where address~\"$host\"]]=0) do={/ip/address/add address=$host/24 interface=br-client}" 2>&1 | head -1
  $SSH "admin@$ip" '/ip/route/print where dst-address~"10.42"' 2>&1 | tail -4
}

case "${1:-all}" in
  setup) setup_router "$A" "$HOSTA"; setup_router "$B" "$HOSTB" ;;
  ping)  echo "=== A($A) -> B host $HOSTB (src $HOSTA), across the mesh ==="
         $SSH "admin@$A" "/ping $HOSTB src-address=$HOSTA count=5" 2>&1 | tail -8 ;;
  all)   setup_router "$A" "$HOSTA"; setup_router "$B" "$HOSTB"
         echo "=== A($A) -> B host $HOSTB (src $HOSTA), across the mesh ==="
         $SSH "admin@$A" "/ping $HOSTB src-address=$HOSTA count=5" 2>&1 | tail -8 ;;
esac
