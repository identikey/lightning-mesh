> **ARCHIVED 2026-07-02.** The RouterOS-container MikroTik track is retired (beads ut9/ns1/ecd closed; AP/STA architecture retired). The live fleet is native OpenWrt — see deploy/openwrt/README.md. Kept as historical reference; the backing scripts remain in deploy/mikrotik/.

# Mesh client routing — what it does and why

This explains the route and firewall rules that let a WiFi or LAN client behind
one MikroTik router reach a host behind another router across the mjolnir mesh.
`client-routing.rsc` installs these rules. It depends on `container-net.rsc`
having already run — that script sets up `br-mesh` and the container veth.

## The core problem

The meshd container holds the TUN tunnels and runs babeld, which installs
per-`/24` routes inside the container's network namespace. RouterOS knows nothing
about those routes by default. Without a static route pointing `10.42.0.0/16`
at the container, RouterOS drops cross-mesh client traffic before it ever reaches
the daemon.

## Address spaces in play

| Subnet | What it is | Scope |
|---|---|---|
| `192.168.x.x` (or site LAN) | physical LAN | per-router, not mesh-routed |
| `172.20.0.0/24` | container ↔ router link | underlay plumbing, internal to each router |
| `10.42.0.0/16` | mesh client supernet | **the space every router's clients live in** |

Each router claims one `/24` from `10.42.0.0/16` for the clients behind it
(e.g. router A → `10.42.1.0/24`, router B → `10.42.7.0/24`). The supernet is
fixed; the per-router slice is a site configuration choice.

## End-to-end packet path

```
client A (e.g. 10.42.1.5)
  │
  │  on router A's local LAN (or WiFi)
  ▼
RouterOS A
  │  static route: 10.42.0.0/16 via 172.20.0.2
  ▼
container A (172.20.0.2)
  │  kernel forwards packet; babeld route: 10.42.7.0/24 via mj-peer-B TUN
  ▼
TUN (iroh QUIC session) ──────────────────────────────────────────────────▶
                                                                container B
  │  babeld knows 10.42.7.0/24 is local; packet exits br-mesh
  ▼
RouterOS B
  │  routes 10.42.7.0/24 to B's LAN (per-site LAN config — see caveat below)
  ▼
host B (10.42.7.42)
```

Reply packets trace the path in reverse; no NAT is needed for mesh-to-mesh
traffic (each /24 is globally meaningful within the mesh).

## What `client-routing.rsc` installs

### 1. Static route

```
/ip/route/add dst-address=10.42.0.0/16 gateway=172.20.0.2 \
    comment="mjolnir mesh clients"
```

Sends every packet destined for any mesh client address to the container.
babeld inside the container has a more-specific `/24` route for each peer and
will forward it out the correct TUN device.

### 2. Firewall forward-accept rules (both directions)

```
/ip/firewall/filter/add chain=forward action=accept src-address=10.42.0.0/16 \
    comment="mjolnir mesh transit src" place-before=0

/ip/firewall/filter/add chain=forward action=accept dst-address=10.42.0.0/16 \
    comment="mjolnir mesh transit dst" place-before=0
```

On a router with a default `forward-drop` policy (RouterOS out-of-box default),
transit packets are dropped without explicit accept rules. Two rules are required:

- **src rule** — accepts packets *arriving from* the mesh (container → local
  client), which appear on the `forward` chain when RouterOS routes them to the
  LAN.
- **dst rule** — accepts packets *heading into* the mesh (local client →
  container), same chain.

Both rules are placed at position 0 (top of the chain) so a later drop rule
cannot pre-empt them. On a blank firewall the `place-before` call is skipped
(no rules to place before); the accepts are still added.

## Dependencies and prerequisites

1. **`container-net.rsc` must run first.** It creates `br-mesh`, assigns
   `172.20.0.1/24` to the bridge, and sets up the container veth. Without it
   `172.20.0.2` is unreachable and the route installed here is a black hole.

2. **`net.ipv4.ip_forward` inside the container.** The container must forward
   packets between its TUN devices and its `eth0`. This is enabled by meshd at
   startup — it is NOT a RouterOS setting and does not need manual action.

3. **babeld running inside the container.** The static route sends all
   `10.42.0.0/16` traffic to the container, but the container needs babeld's
   routes to know which TUN to use for each `/24`. If babeld is not yet
   peered, traffic for remote `/24`s will be dropped inside the container.

## Caveat: each router's own /24 → local clients

`client-routing.rsc` installs the supernet route and mesh firewall rules only.
It does NOT configure how a router delivers packets to its own claimed `/24` on
the local LAN side (e.g., the DHCP pool, a LAN bridge, or a WiFi interface
handing out `10.42.1.x` addresses). That is per-site LAN configuration and is
left to the operator (or a future bead). Without it, remote mesh peers can send
to `10.42.1.x` addresses and the packets will arrive at RouterOS A, but A won't
know where to deliver them locally.
