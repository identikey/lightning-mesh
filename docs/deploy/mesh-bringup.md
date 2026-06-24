# Mesh bring-up (offline LAN backhaul) — runbook + status

How to bring up the mjolnir mesh across a set of same-site nodes, in the
**current default mode**: an offline LAN, no relay, no internet
(`mjolnir-meshd mesh --lan`). This is the deployment direction settled by
`mjolnir-mesh-4pk`. For the older relay/internet path, see the bottom section.

> **Hardware note.** The bring-up below is written for the **MikroTik RouterOS
> container** target (where the deploy scripts live). The project is separately
> evaluating **OpenWrt mt76 nodes** as the radio/hardware target (`L3 as the
> invariant` — the meshd/babeld stack is the same regardless); see
> `docs/research/openwrt-mesh-hardware/` and `deploy/openwrt/`.

## The idea (why --lan + a derived backhaul)

The relay path double-NATs each container (`container-net.rsc`), which stops
same-LAN nodes from forming direct iroh paths — the asymmetric-loss symptom of
`mjolnir-mesh-67h`. The `--lan` approach removes the relay entirely:

1. Every node's container is bridged onto **one shared L2 segment**
   (`container-net-lan.rsc`).
2. meshd **self-assigns a derived IPv4 backhaul address** — `10.254.<h>.<l>`,
   where `h,l = blake3(node_id)[..2]` — to its shared-segment interface
   (`--backhaul-iface`, default `eth0`) **before iroh binds**. (Order matters:
   iroh only advertises addresses present at bind time. IPv4, not an IPv6 ULA,
   because iroh surfaces private IPv4 over mDNS but not ULAs — see the
   `iroh-lan-backhaul-findings` memory.)
3. Nodes discover each other by **bare node id over mDNS** on the shared L2 and
   the per-peer tunnel **upgrades to a DIRECT path** (`kind=DIRECT remote=10.254.x`,
   ~3 ms, no relay).

The L3 overlay is unchanged: per-peer `/31` TUN tunnels, babeld routing, and the
client `/24` claims all ride the (now direct) tunnel.

## Bring-up steps

Per node, once:

1. **Firmware** — RouterOS 7.23.1 + `container` + `wifi-qcom` packages:
   `deploy/mikrotik/fetch-firmware.sh`, upload all three, reboot. (See
   `mikrotik-routeros-container.md`.)
2. **device-mode=container** — enable via the physical reset-button hold at boot
   (cannot be done over SSH).
3. **Shared L2** — edit `deploy/mikrotik/container-net-lan.rsc`, set `$meshLink`
   to the port facing the other nodes (the common switch port on the bench, or
   the WiFi backhaul interface in the field) — **the one value you must get
   right** — then `/import file-name=container-net-lan.rsc`.

Then, from your workstation:

4. **Deploy the daemon** — `deploy/mikrotik/deploy-mesh.sh` scp's the image tar,
   sets each node's persistent `IROH_SECRET`, and (re)creates the container
   running `mesh --peer <other-ids…>` (defaults to `--lan`). Edit the `NODES`
   table in the script for your swarm. (Build the image first with
   `deploy/mikrotik/build.sh`.)
5. **Verify the direct backhaul** — in each node's container log
   (`/log/print where topics~"container"`):
   - `assigned IPv4 backhaul address … 10.254.x`
   - `endpoint addressable … 10.254.x`
   - per peer: `tunnel up … mj-peer-…` and a path of `kind=DIRECT remote=10.254.x`
   - `gossip overlay joined`, distinct `claimed client subnet 10.42.x.0/24`
   - `babeld started …` and (per peer) a `Neighbour fe80::… ` line
6. **Validate client routing** — `deploy/mikrotik/apply-routing-and-test.sh`
   applies `client-routing.rsc`, gives each node a stand-in client in its `/24`,
   and pings across the mesh. A reply proves cross-mesh routing.

## Status (what's proven vs provisional)

**Validated** (armv7 Linux containers, the deployment arch — `4pk`):
- the `--lan` backhaul mechanism end-to-end: derived address → mDNS discovery by
  bare node id → tunnel upgrades to a DIRECT path (no relay).

Validated on the **MikroTik hardware** in an earlier round (relay path, but the
overlay is mode-independent): per-peer tunnels, gossip + CRDT claim convergence,
`ip_forward`, babeld spawn/config/SIGHUP, and — with the IPv6-link-local on the
TUN (`op4`) — babeld forming an adjacency and installing the cross-mesh route.

**Not yet hardware-validated** (tracked on `mjolnir-mesh-4pk`):
1. `container-net-lan.rsc` shared-L2 bridging on real MikroTik (interface-specific).
2. The full babeld client data path under the new backhaul (babeld was stubbed
   with `/bin/true` in the container test; re-run real babeld on the bench).
3. 4 nodes (not 2) on a real switch — confirm all pairs go DIRECT.

The literal client-to-client ping (`mjolnir-mesh-apo`) depends on these landing.

## The relay / internet path (cross-site)

When the mesh must span the internet across separate sites, opt into the relay
path instead: use `container-net.rsc` (NAT egress) and run with `--internet`
(`MESH_INTERNET=1 deploy/mikrotik/deploy-mesh.sh`). This is the original path;
its tradeoff is the `67h` relay-loss/asymmetry the `--lan` path avoids.

## Known gap: meshctl

`meshctl deploy` still drives the **old** `tun-listen`/`tun-connect` single-tunnel
flow — not `mesh` mode, `--lan`, or the shared-L2 setup. The bring-up above is
therefore driven by the shell helpers in `deploy/mikrotik/`. Folding the `mesh`
+ `--lan` + `container-net-lan.rsc` workflow into meshctl is the durable home for
this orchestration (tracked as a follow-up bead).
