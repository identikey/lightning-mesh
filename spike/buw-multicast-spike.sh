#!/usr/bin/env bash
#
# buw.1 spike harness — does babeld form an adjacency over ONE overlay TUN when
# the daemon emulates multicast? (mjolnir-mesh-buw.1)
#
# Topology (all on one host, root required):
#
#   netns buw-a                         netns buw-b
#   ┌────────────────────┐              ┌────────────────────┐
#   │ mjolnir0 (TUN)     │              │ mjolnir0 (TUN)     │
#   │  10.254.0.1/16     │   emulated   │  10.254.0.2/16     │
#   │  fe80::1           │   multicast  │  fe80::2           │
#   │   ▲                │   over UDP   │   ▲                │
#   │   │ spike bridge   │◄────────────►│   │ spike bridge   │
#   │  veth-a 10.0.0.1 ──┼── veth pair ─┼── veth-b 10.0.0.2  │
#   │  babeld on mjolnir0│              │  babeld on mjolnir0│
#   └────────────────────┘              └────────────────────┘
#
# The veth pair + UDP stand in for the iroh DatagramConn seam; the overlay TUN
# and the multicast fan-out are the real mjolnir_mesh::tun::overlay code.
#
# Run:  sudo spike/buw-multicast-spike.sh
set -euo pipefail

NS_A=buw-a
NS_B=buw-b
DUMP_PORT=33123
RUN_SECS="${RUN_SECS:-20}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/debug/buw-overlay-spike"
LOG=/tmp/buw-spike
mkdir -p "$LOG"

say() { printf '\n\033[1;36m== %s ==\033[0m\n' "$*"; }

cleanup() {
  say "cleanup"
  pkill -f buw-overlay-spike 2>/dev/null || true
  ip netns pids "$NS_A" 2>/dev/null | xargs -r kill 2>/dev/null || true
  ip netns pids "$NS_B" 2>/dev/null | xargs -r kill 2>/dev/null || true
  ip netns del "$NS_A" 2>/dev/null || true
  ip netns del "$NS_B" 2>/dev/null || true
}
trap cleanup EXIT

# ── 0. prerequisites ────────────────────────────────────────────────────────
if [[ $EUID -ne 0 ]]; then echo "run as root (sudo)"; exit 1; fi
if ! command -v babeld >/dev/null; then
  say "installing babeld"
  pacman -S --noconfirm babeld
fi
if [[ ! -x "$BIN" ]]; then
  echo "spike binary missing — build it first:"
  echo "  cargo build -p mjolnir-mesh --features spike --bin buw-overlay-spike"
  exit 1
fi

# fresh start
cleanup

# ── 1. namespaces + underlay veth ───────────────────────────────────────────
say "setting up namespaces + veth underlay"
ip netns add "$NS_A"
ip netns add "$NS_B"
ip link add veth-a netns "$NS_A" type veth peer name veth-b netns "$NS_B"
ip -n "$NS_A" addr add 10.0.0.1/24 dev veth-a
ip -n "$NS_B" addr add 10.0.0.2/24 dev veth-b
ip -n "$NS_A" link set veth-a up
ip -n "$NS_B" link set veth-b up
ip -n "$NS_A" link set lo up
ip -n "$NS_B" link set lo up

# Pin permanent neighbour entries so the underlay never fails ARP resolution.
# Without this the veth intermittently returns EHOSTUNREACH on send_to, dropping
# ~75% of the UDP datagrams that stand in for iroh's (reliable) QUIC transport.
MAC_A="$(ip -n "$NS_A" link show veth-a | awk '/link\/ether/{print $2}')"
MAC_B="$(ip -n "$NS_B" link show veth-b | awk '/link\/ether/{print $2}')"
ip -n "$NS_A" neigh replace 10.0.0.2 lladdr "$MAC_B" dev veth-a nud permanent
ip -n "$NS_B" neigh replace 10.0.0.1 lladdr "$MAC_A" dev veth-b nud permanent

# ── 2. overlay bridge (spike bin) in each namespace ─────────────────────────
say "starting overlay bridges (UDP standing in for iroh)"
SPIKE_LOG="${SPIKE_LOG:-overlay=debug}"
ip netns exec "$NS_A" env RUST_LOG="$SPIKE_LOG" "$BIN" \
  --tun mjolnir0 --addr 10.254.0.1/16 --ll fe80::1 \
  --listen 10.0.0.1:6000 --peer 10.0.0.2:6000 >"$LOG/spike-a.log" 2>&1 &
ip netns exec "$NS_B" env RUST_LOG="$SPIKE_LOG" "$BIN" \
  --tun mjolnir0 --addr 10.254.0.2/16 --ll fe80::2 \
  --listen 10.0.0.2:6000 --peer 10.0.0.1:6000 >"$LOG/spike-b.log" 2>&1 &

# wait for both mjolnir0 TUNs to appear, then enable the MULTICAST flag the TUN
# lacks by default (without it babeld will not send Hellos to ff02::1:6).
for ns in "$NS_A" "$NS_B"; do
  for _ in $(seq 1 50); do
    ip -n "$ns" link show mjolnir0 >/dev/null 2>&1 && break
    sleep 0.1
  done
  ip -n "$ns" link set dev mjolnir0 multicast on
done
say "overlay TUNs up"
ip -n "$NS_A" addr show mjolnir0 | sed 's/^/  A: /'
ip -n "$NS_B" addr show mjolnir0 | sed 's/^/  B: /'

# ── 3. packet captures (evidence of where hellos flow / stop) ───────────────
say "starting packet captures (mjolnir0 = babel; veth = UDP-tunneled datagrams)"
ip netns exec "$NS_A" tcpdump -n -l -i mjolnir0 >"$LOG/tcpdump-a-mjolnir0.txt" 2>/dev/null &
ip netns exec "$NS_B" tcpdump -n -l -i mjolnir0 >"$LOG/tcpdump-b-mjolnir0.txt" 2>/dev/null &
ip netns exec "$NS_A" tcpdump -n -l -i veth-a udp >"$LOG/tcpdump-a-veth.txt" 2>/dev/null &
sleep 1  # let captures attach

# ── 4. real babeld on each mjolnir0 ─────────────────────────────────────────
say "adding a client /24 per node + babeld (-d 2 for hello-level logging)"
declare -A SUBNET=( [a]=10.42.1 [b]=10.42.2 )
for pair in "$NS_A:a" "$NS_B:b"; do
  ns="${pair%%:*}"; tag="${pair##*:}"; net="${SUBNET[$tag]}"

  # A dummy iface carrying this node's client /24, so babeld has a real kernel
  # route to redistribute over the overlay (the thing we prove propagates).
  ip -n "$ns" link add client0 type dummy 2>/dev/null || true
  ip -n "$ns" addr add "${net}.1/24" dev client0
  ip -n "$ns" link set client0 up

  # Per-node babeld config mirroring the production renderer (babel/config.rs):
  # redistribute ONLY this client /24; deny local + the underlay (10.0.0.0/24)
  # and overlay-link (10.254.0.0/16) blocks. Without these denies babeld installs
  # an 'unreachable' /32 for the peer's transport address and poisons the very
  # UDP/veth path the overlay rides on (a self-inflicted feedback loop).
  cat >"$LOG/babeld-$tag.conf" <<CONF
interface mjolnir0 type wired hello-interval 1
redistribute ip ${net}.0/24 ge 24 le 24 allow
redistribute local deny
redistribute deny
in ip 10.0.0.0/24 deny
out ip 10.0.0.0/24 deny
in ip 10.254.0.0/16 deny
out ip 10.254.0.0/16 deny
CONF

  ip netns exec "$ns" babeld \
    -c "$LOG/babeld-$tag.conf" \
    -I "$LOG/babeld-$tag.pid" \
    -S "$LOG/babeld-$tag.state" \
    -L "$LOG/babeld-$tag.log" \
    -g "$DUMP_PORT" \
    -d 2 \
    mjolnir0 >"$LOG/babeld-$tag.out" 2>&1 &
done

say "running for ${RUN_SECS}s to let adjacency form"
sleep "$RUN_SECS"

# ── 5. evidence + verdict ───────────────────────────────────────────────────
set +e  # diagnostics below must all run even if individual greps find nothing
say "overlay bridge packet accounting (from RUST_LOG=overlay=debug)"
for tag in a b; do
  f="$LOG/spike-$tag.log"
  printf '  node %s: rd-tun=%s tx-ok=%s tx-DROP=%s rx-peer=%s wr-tun=%s\n' "$tag" \
    "$(grep -c 'rd-tun' "$f" 2>/dev/null)" \
    "$(grep -cE 'tx-peer [0-9]' "$f" 2>/dev/null)" \
    "$(grep -c 'tx-peer dropped' "$f" 2>/dev/null)" \
    "$(grep -c 'rx-peer' "$f" 2>/dev/null)" \
    "$(grep -c 'wr-tun' "$f" 2>/dev/null)"
done

say "packet-flow evidence"
count() { grep -c "$2" "$1" 2>/dev/null || echo 0; }
A_TX=$(count "$LOG/tcpdump-a-mjolnir0.txt" 'ff02::1:6')   # A's babel hellos leaving its TUN
B_RX=$(count "$LOG/tcpdump-b-mjolnir0.txt" 'ff02::1:6')   # hellos arriving on B's TUN
UDP=$(grep -c '' "$LOG/tcpdump-a-veth.txt" 2>/dev/null || echo 0)  # UDP datagrams over the veth
echo "  babel hellos out of A's mjolnir0 : $A_TX"
echo "  babel hellos into  B's mjolnir0 : $B_RX"
echo "  UDP datagrams on A's veth        : $UDP"
echo "  babeld-a 'hello' log lines       : $(grep -ic hello "$LOG/babeld-a.log" 2>/dev/null || echo 0)"

# Decisive signal: each babeld installs a kernel route to the PEER's exported
# /32 (e.g. B learns 10.0.0.1/32 and 10.254.0.1/32 from A) — only possible if a
# neighbour adjacency formed over the overlay TUN.
say "kernel routes learned via babel (proto)"
A_ROUTES="$(ip -n "$NS_A" route show 2>/dev/null)"
B_ROUTES="$(ip -n "$NS_B" route show 2>/dev/null)"
echo "--- node A routes ---"; echo "$A_ROUTES" | sed 's/^/  /'
echo "--- node B routes ---"; echo "$B_ROUTES" | sed 's/^/  /'

# also grab babeld's own neighbour dump via the -g port (try ::1 and 127.0.0.1)
dump_babel() {
  ip netns exec "$1" python3 - "$DUMP_PORT" <<'PY' 2>/dev/null || true
import socket, sys, time
port = int(sys.argv[1])
for host in ("::1", "127.0.0.1"):
    try:
        s = socket.create_connection((host, port), timeout=2)
    except OSError:
        continue
    s.settimeout(1.5); s.sendall(b"dump\n"); buf=b""; end=time.time()+2.5
    while time.time() < end:
        try: c = s.recv(65536)
        except socket.timeout: break
        if not c: break
        buf += c
    print(buf.decode(errors="replace")); break
PY
}
DUMP_A="$(dump_babel "$NS_A")"
[ -n "$DUMP_A" ] && { echo "--- node A babel neighbours ---"; echo "$DUMP_A" | grep -Ei 'neighbour' | sed 's/^/  /'; }

# PASS if each node learned the PEER's client /24 as a REACHABLE babel route.
# `^10.42.X.0/24 ... proto babel` matches only a reachable route; an unreachable
# one is printed as `unreachable 10.42.X.0/24 ...` and won't match the anchor.
say "VERDICT"
if echo "$A_ROUTES" | grep -qE '^10\.42\.2\.0/24 .*proto babel' \
   && echo "$B_ROUTES" | grep -qE '^10\.42\.1\.0/24 .*proto babel'; then
  echo -e "\033[1;32mPASS\033[0m: each node installed a REACHABLE babel route to the peer's client /24"
  echo "      over the single overlay TUN — adjacency formed via emulated multicast."
  echo "      The buw single-TUN, multi-neighbour data plane is viable."
  RC=0
else
  echo -e "\033[1;31mFAIL\033[0m: peer client /24 not reachable on one/both sides. Captures + logs in $LOG/."
  echo "  Where did hellos stop? out-of-A=$A_TX into-B=$B_RX udp=$UDP"
  echo "  babeld-a.log tail:"; tail -n 8 "$LOG/babeld-a.log" 2>/dev/null | sed 's/^/    /' || true
  RC=1
fi

exit $RC
