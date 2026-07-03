> **ARCHIVED 2026-07-02.** The RouterOS-container MikroTik track is retired (beads ut9/ns1/ecd closed; AP/STA architecture retired). The live fleet is native OpenWrt — see deploy/openwrt/README.md. Kept as historical reference; the backing scripts remain in deploy/mikrotik/.

# MikroTik container networking — what it does and why

This explains the `veth` + `bridge` + `NAT` plumbing each router needs before a
`mjolnir-meshd` container can reach the mesh. It's the *underlay* — the scaffolding
that gives the container internet egress (so iroh can reach relays/peers). The
mesh overlay (the `10.255.0.0/16` TUN tunnels) runs on top of it. See the deploy
runbook (`mikrotik-routeros-container.md`) for the step-by-step commands.

## The core problem

A RouterOS container is an **isolated network namespace** — by default it has no
network at all: no LAN, no internet, no relays. Every command below exists to
(a) give the container a network interface, and (b) give it a route out.

## The pieces

### `veth` — a virtual ethernet cable
A veth is a virtual patch cable: a pair of linked interfaces, one end inside the
container (its `eth0`), the other in RouterOS (`veth-mesh`). Bytes in one end come
out the other. It's how the container is "plugged in."

```
/interface/veth/add name=veth-mesh address=172.20.0.2/24 gateway=172.20.0.1
```
- `address=172.20.0.2` → the **container's** IP (far end, inside the container)
- `gateway=172.20.0.1` → the container's default route
- `veth-mesh` → the **router's** end of the cable

### `bridge` + bridge port — a virtual switch
A bridge is a virtual ethernet switch inside RouterOS; a "bridge port" is an
interface plugged into it (like a physical switch port).

```
/interface/bridge/add name=br-mesh                              # the switch
/interface/bridge/port/add bridge=br-mesh interface=veth-mesh   # plug veth in
/ip/address/add address=172.20.0.1/24 interface=br-mesh         # router's IP on it
```

**Why a bridge instead of an IP directly on the veth?** Extensibility: the bridge
is a switch you can plug *more* containers/interfaces into, all sharing
`172.20.0.0/24` and the `172.20.0.1` gateway. The gateway IP lives on the switch.
For one container it's marginally more than needed, but it's how a multi-container
router grows.

### Choosing the IPs
1. **Private range** — `172.20.0.0/24` is RFC-1918, never routed publicly.
2. **Must not collide with the real LAN** — the physical LAN is `192.168.0.0/24`,
   so the container link must be a *different* subnet or routing breaks.
3. **Exact numbers are arbitrary** — `.1` router, `.2` container, by convention.
   Any unused private subnet works; `/24` leaves room for ~250 containers.

### `NAT` (masquerade) — why it's required
```
/ip/firewall/nat/add chain=srcnat action=masquerade src-address=172.20.0.0/24
```
`172.20.0.0/24` exists **only inside this one router** — the main router and the
internet have no route back to `172.20.0.2`, so replies could never return.
Masquerade rewrites the container's outbound packets to use the router's own LAN
IP (`192.168.0.x`); replies come back there and are translated back to the
container. Same trick a home router uses to share one public IP.

**Without NAT the container can reach `172.20.0.1` and nothing else — no DNS, no
relay, no mesh.** (A dropped masquerade rule presents as DNS timeouts /
"Failed to publish to pkarr" / no relay in the container — the address blob then
contains only `172.20.0.2`, which is unroutable from other routers.)

## Packet path (container → iroh relay)

```
container 172.20.0.2 ──veth──▶ br-mesh 172.20.0.1 ──[NAT: src→192.168.0.x]──▶
  ether1 ──▶ main router 192.168.0.1 ──▶ internet ──▶ iroh relay
(replies retrace the path; NAT translates 192.168.0.x back to 172.20.0.2)
```

## Three address spaces — don't conflate them

| Subnet | What it is | Scope |
|--------|-----------|-------|
| `192.168.0.0/24` | the real LAN | the physical network the routers sit on |
| `172.20.0.0/24` | container ↔ router link | **underlay plumbing**, internal to each router |
| `10.255.0.0/16` | the `/31` TUN tunnels (`mj-peer-*`) | **the mesh overlay** — the swarm fabric |

The first two just get the container online so iroh can phone home. The mesh you
care about — the `10.255.x.x` tunnels between peers — runs *on top*, inside the
TUN devices the daemon creates.

## This is identical on every router

The `veth + bridge + NAT` block is the same on all routers, which makes it
error-prone to type by hand (a dropped NAT rule already broke container egress
once). It's tracked for scripting — see beads `mjolnir-mesh-xh5` (container
network setup automation). `device-mode` enabling is the one step that *cannot*
be scripted (physical reset-button confirm).
