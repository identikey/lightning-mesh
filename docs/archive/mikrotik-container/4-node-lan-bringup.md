> **ARCHIVED 2026-07-02.** The RouterOS-container MikroTik track is retired (beads ut9/ns1/ecd closed; AP/STA architecture retired). The live fleet is native OpenWrt — see deploy/openwrt/README.md. Kept as historical reference; the backing scripts remain in deploy/mikrotik/.

# 4-Node LAN Mesh Bring-Up (wired switch) — mjolnir-mesh-2j6

Prove the full mesh on the **known-good wired switch** before swapping in the
wireless backhaul (mjolnir-mesh-b1d). One variable at a time: if 4 nodes route
client traffic over the wire, any later failure is in the radio layer, not here.

## Confirm before starting

- [ ] All 4 nodes have **device-mode=container** enabled (physical reset-button
      hold at boot — can't be done over SSH). *Memory note: only `.181` / `.113`
      were validated; the other two likely still need this + the image.*
- [ ] Fresh image uploaded to each: `deploy/mikrotik/mjolnir-meshd-ros.tar`
      (rebuilt this session — has `--lan` default + IPv4 backhaul).
- [ ] Each node has a persistent `IROH_SECRET` set (stable identity / node id).
- [ ] You know each node's **switch-facing interface** (the port into the shared
      switch). Find it on the router: `/interface/ethernet print`.

## Per-node setup

1. **Bridge the container onto the switch L2.** Edit `container-net-lan.rsc`, set
   `:local meshLink "etherN"` to the switch-facing port, then
   `/import file-name=container-net-lan.rsc`.
2. **Container cmd** (the daemon defaults to `--lan` now — no flag needed):
   ```
   mesh --roster /roster --babeld babeld --backhaul-iface eth0
   ```
   where `/roster` lists the **four node ids** (one per line; `#` comments ok).
   Get each id from its meshd log line `node id: <hex>` (or the deploy's id read).
   Use **real** `babeld` — not `/bin/true` — so client /24s actually route.
3. Start the container.

## Validation checklist (watch each node's meshd log)

- [ ] `assigned IPv4 backhaul address 10.254.x.y` — 4 distinct addresses.
- [ ] `endpoint addressable` line **includes** the `10.254.x.y` addr.
- [ ] For each of the **6 pairs**: `tunnel up mj-peer-<id>` then
      `tunnel path … kind="DIRECT" remote=Ip(10.254.x.y:…)`. No `kind=RELAY`
      (there's no relay in `--lan` anyway — a RELAY line means something's wrong).
- [ ] CRDT: `claimed client subnet` (4 distinct /24s in `10.42.0.0/16`) and
      `gossip: received peer subnet claim` from the other nodes (convergence).
- [ ] babeld on each node learns the other three /24s
      (`10.42.x.0/24 … installed`).
- [ ] **Data path:** from a client on node A's LAN, reach a host (or the router
      gateway) on node B's claimed /24. Needs `client-routing.rsc` applied on each
      node for the RouterOS-side glue.

## Troubleshooting

| Symptom | Check |
|---|---|
| A pair never forms a tunnel | both nodes have each other's id in `/roster`; the container is actually bridged onto the switch via `$meshLink` (mDNS needs the shared L2) |
| `kind=RELAY` appears | shouldn't in `--lan`; confirm no `--internet`/`--relay`, and the node has internet-less LAN mode |
| no `assigned … backhaul` line | `/proc/sys` writable in the container; `eth0` is the veth (`--backhaul-iface`) |
| babeld installs no routes | babeld is real (not `/bin/true`); config `redistribute`; IPv6 link-local present on the TUNs (op4) |

## Once this passes

→ mjolnir-mesh-b1d (wireless backhaul). The only change is pointing
`container-net-lan.rsc`'s `$meshLink` at the **WiFi backhaul interface** instead
of the ether port — the container/software layer is unchanged.
