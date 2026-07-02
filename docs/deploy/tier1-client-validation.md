# Tier 1 — client data-plane validation (offline runbook)

> **Status 2026-07-01 (field session).** Steps 1.1–1.3 PASS from a live laptop,
> but only after two live bench workarounds, tracked as beads:
> **kf7** — the rendered babeld filter `redistribute ip <..>/24 ge 24 le 24
> allow` never matches, so claimed /24s weren't announced; a bare
> `redistribute ip <claimed /24> allow` line is hand-inserted in
> `/etc/mjolnir/babeld.conf` on tr3000/m3000-b/m3000 (reverts whenever meshd
> re-renders the conf — e.g. service restart). **659** — DHCP served stock
> 192.168.1.0/24 everywhere; `network.lan.ipaddr` is now the uci list
> `['<claimed .1>/24','192.168.1.1/24']` on those three nodes (claimed /24
> primary, stock kept as wired-recovery alias). **wr3000s-a is parked**: it
> lost the 10.42.242 claim collision to m3000 and holds no client subnet
> (bead eon) — don't associate test clients to it. Also filed: **eon** (claim
> lifecycle: restart self-collision, loser never re-claims) and **nrr**
> (meshd restart leaks a second babeld; babeld /32 local-address export spam).

Self-contained: everything needed is in this file. Run from the laptop while
associated to the mesh client AP. Nothing to build; this validates what is
already deployed.

## Fleet (hand-kept copy of `deploy/openwrt/fleet-nodes.conf`)

| name      | mgmt addr       | model         | role                                            |
|-----------|-----------------|---------------|-------------------------------------------------|
| wr3000s-a | `10.254.242.84` | Cudy WR3000S  | no WAN of its own                                |
| m3000-b   | `10.254.12.214` | Cudy M3000    | wired/jump node (`192.168.1.1` on ethernet)      |
| m3000     | `10.254.242.172`| Cudy M3000    | radio-only                                       |
| tr3000    | `10.254.61.115` | Cudy TR3000   | **gateway** (`gateway=1`, WAN via `192.168.0.1`) |

- Client AP: SSID **`Lightning Mesh`**, key **`lightning!`** (unless rotated in
  `fleet-secrets/wireless.env`). Same SSID on every node — which node you land
  on is the radio's choice; your gateway IP tells you which (see 1.0).
- Client subnets are hash-claimed per node (`10.42.<x>.0/24`, not listed here —
  discover live in step 1.0). Each node's `.1` lives on its `br-lan`.
- Ground truth on any node: `ssh root@<mgmt> 'service mjolnir-meshd diag'`
  (identity, backhaul addr, interface addrs, mesh routes; read-only).

## 1.0 Prep — build the subnet map (do this FIRST, while comfy)

From the laptop (any network that reaches the nodes — mesh AP works):

```sh
for n in 10.254.242.84 10.254.12.214 10.254.242.172 10.254.61.115; do
  echo "== $n"; ssh root@$n "ip -4 addr show br-lan | grep 10.42; ip route | grep -c 'proto babel'"
done
```

Record each node's `10.42.x.1`. You now have the name → mgmt → client-subnet
map the rest of the runbook uses. Also note the babel route count (should be
≥3 per node: the other nodes' /24s, plus a default on non-gateways if egress
is live).

Optional but recommended while still online: install iperf3 for step 1.4:

```sh
for n in 10.254.242.84 10.254.12.214 10.254.242.172 10.254.61.115; do
  ssh root@$n "opkg update && opkg install iperf3"
done
```

(If a non-gateway node's opkg fetch succeeds over the mesh, that is itself an
early Tier-2 egress pass.) `brew install iperf3` on the laptop if needed.

## 1.1 Client basics — DHCP + gateway

1. Join SSID `Lightning Mesh` from the laptop.
2. `ipconfig getifaddr en0` (or `ifconfig en0`) → expect `10.42.x.n`,
   netmask `/24`, router `10.42.x.1`.
3. `ping -c5 10.42.x.1` → sub-5 ms replies.

**Pass:** lease from a node's /24 and the `.1` answers.

## 1.2 Client↔client routed transit across the mesh

The point of the per-node-/24 architecture: traffic must be *routed*
node-to-node, not bridged.

1. From the laptop, ping every **other** node's client gateway:
   `ping -c5 10.42.y.1` for each y ≠ your x.
2. Associate a phone to `Lightning Mesh`; check its IP. If it landed on a
   *different* node (different `10.42.y`), ping the phone from the laptop.
   If it landed on the same node, walk the phone next to a far node until it
   reassociates (or toggle its wifi there), then ping.
3. `traceroute -n 10.42.y.1` → expect first hop your `.1`, then a
   `10.254.*` or direct step to the far node. Two hops of routing = proof.

**Pass:** replies from another node's `.1` and from a client behind it.

## 1.3 Management from anywhere

From the laptop on the client AP:

```sh
ssh root@10.254.61.115 'service mjolnir-meshd diag'   # tr3000, or any node
```

**Pass:** SSH works to all four mgmt addrs from the client network — the
management plane is reachable from the client plane via routing.

## 1.4 Throughput baselines (record these — baseline for the RTT-metric work)

Reality check: with all four nodes in one room, 802.11s gives a full mesh —
**every path is 1 hop**. For a true 2-hop transit number, physically separate
a node (far room / downstairs) until `traceroute` between edge nodes shows a
middle hop, or check babeld's view via
`ssh root@<mgmt> 'ip route | grep 10.42'` (next-hop changes when it stops
being direct).

On the target node: `iperf3 -s` (via SSH, leave running).
From the laptop:

```sh
iperf3 -c 10.254.x   # to the node you're associated to (AP-local)
iperf3 -c 10.254.y   # to a far node (crosses the mesh)
```

Node↔node (cleaner than laptop-involved numbers):

```sh
ssh root@10.254.242.84 "iperf3 -c 10.254.61.115"    # 1 hop
# after separating a node, rerun the pair that now transits it: 2 hops
```

Record: pair, hop count, Mbps, retransmits. Expect roughly **half per extra
hop** — same-channel (ch 6) airtime is shared between receive and re-transmit.

## 1.5 Resilience — kill a transit node

Needs a genuine 2-hop path (see 1.4 separation note). Say laptop→A…B…C where
B is transit:

1. `ping 10.42.c.1` (or the far node's mgmt addr) — leave running.
2. Pull B's power. Note the timestamp / count the lost pings.
3. **Blackout** = seconds of lost replies while babeld reroutes (if A—C have
   any direct radio path) or until B returns (if not).
4. Plug B back in. Watch it rejoin: pings via B resume,
   `ssh root@<B> 'service mjolnir-meshd diag'` shows routes repopulated.

Record: seconds to reroute, seconds to full rejoin, any stuck state
(a node that needs a manual `service mjolnir-meshd restart` is a bug — bead it).

## 1.6 Roaming reality check

1. `ping -i 1 10.254.61.115` from the laptop, leave running.
2. Walk between nodes until the laptop reassociates (watch the BSSID:
   `sudo wdutil info | grep -i bssid` on macOS).
3. **Expected**: reassociation succeeds, but the laptop gets a **new IP** from
   the new node's /24 → the running ping/sessions break, then resume with the
   new source.

That IP change is by design (no L2 bridging across nodes; broadcast
containment). Document it honestly: FT/`bnd` would speed up *auth* only — it
cannot preserve the IP. Session survival across roams needs a client-side
overlay (future work), not L2 tricks.

## Tier-2 spoiler — egress over the mesh may already work

a8o is implemented and was validated live once: tr3000 runs with
`gateway=1`, meshd renders babeld config that announces the kernel default
route into the mesh (nearest-exit metric semantics), the wan-zone masquerade
covers mesh sources, and **every** node's dnsmasq already forwards client DNS
to `9.9.9.9` / `1.1.1.1` (fleet-wide via `setup-wireless.sh`). So from the
laptop on a **non-gateway** node:

```sh
ping -c5 8.8.8.8                 # raw egress through tr3000
dig example.com @10.42.x.1       # DNS via the local node's dnsmasq
traceroute -n 8.8.8.8            # expect: 10.42.x.1 → mesh hop(s) → tr3000 → 192.168.0.1 → internet
curl -sI https://example.com     # full stack
```

If `ping 8.8.8.8` fails, check in order:

1. Non-gateway node has the route? `ssh root@<mgmt> 'ip route show default'`
   → expect `default via <fe80/10.254 next-hop> dev mj-peer-… proto babel`-ish.
   Missing → tr3000 isn't announcing (check its `uci get mjolnir.meshd.gateway`
   and that its own `ip route show default` points out the WAN).
2. Route present but no replies → NAT: on tr3000,
   `iptables -t nat -L -v | grep -i masq` (or `nft list ruleset | grep masq`) —
   mesh-sourced traffic must hit the wan-zone masquerade.
3. Ping works but names don't → DNS: `dig example.com @9.9.9.9` from the
   laptop; if that works, the node's dnsmasq forwarders are missing
   (`uci get dhcp.@dnsmasq[0].server`).

## Recording results

When back online: `bd create --title="Tier 1 client data-plane baseline <date>" --type=task --priority=2`
with the numbers from 1.4/1.5 in the description — it's the baseline the
RTT-metric work gets compared against. Bead anything that failed.
