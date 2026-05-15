# mjolnir-mesh: Mesh VPN with Distributed Network Coordination

**Status:** Architecture overview | **Updated:** 2026-03-26

mjolnir-mesh is a Rust daemon that runs on OpenWrt routers and creates a
decentralized mesh network. Routers coordinate via Iroh QUIC connections
and a gossip-replicated CRDT store — no leader election, no central server.
Any router can go offline without disrupting the mesh.

## What It Provides

- **Unified DHCP** across co-located routers on the same L2 — no IP conflicts,
  seamless device roaming
- **Cross-site IP routing** via Iroh tunnels between geographically separate sites
- **Mesh-wide DNS** for hostname and service discovery across all sites
- **Service registration** — any device can publish services; all routers update DNS
- **Iroh as global address space** — routers and Iroh-aware devices have a NodeId
  as their global identity; IPv4 is for local LAN; DNS bridges the two

## Target Deployment

**Primary**: DWEB (decentralized web) events — 200+ people, 10+ GL.iNet routers
(AXT1800, BE3600), heavy device roaming between access points.

**Also**: Home meshes (2-3 routers), global roaming between sites.

---

## Two Modes of Router Interconnection

The mesh supports two modes simultaneously. A real deployment uses both.

### Mode 1: Local (same venue / same L2 broadcast domain)

Routers at the same venue bridge their LAN interfaces into one L2 domain.
All routers share a single subnet (e.g., `10.42.1.0/24`).

```
                    [ L2 Bridge ]
         ______________|______________
         |             |             |
    [Router-A]    [Router-B]    [Router-C]
    dnsmasq       dnsmasq       dnsmasq
    10.42.1.1     10.42.1.2     10.42.1.3
         |             |             |
     [devices]     [devices]     [devices]
```

- Every router runs dnsmasq on the same subnet — no leader election
- CRDT-synced hostsfile prevents IP conflicts (each dnsmasq knows all MAC→IP bindings)
- Devices roam between APs and keep the same IP
- mDNS works natively (same broadcast domain)

### Mode 2: Remote (different sites / via Iroh)

Each site has its own subnet. Routers connect via Iroh QUIC tunnels.

```
    [ Site A: 10.42.1.0/24 ]          [ Site B: 10.42.2.0/24 ]
         [Router-A] ──── Iroh QUIC tunnel ──── [Router-D]
         dnsmasq                                dnsmasq
         10.42.1.1                              10.42.2.1
              |                                      |
          [devices]                              [devices]
```

- Each site has its own subnet; cross-site routing via Iroh tunnels
- Route table synced via CRDT (`/routes/{subnet}`)
- DNS synced mesh-wide — any device resolves any hostname on any site
- mDNS forwarded via avahi reflector for `.local` names

Both modes coexist in the same mesh. A router at a venue can tunnel to a home router simultaneously.

---

## Fully P2P DHCP (No Leader)

Every router on local L2 runs dnsmasq with three integration points:

```
dhcp-hostsfile=/tmp/mjolnir/reservations   # CRDT-synced MAC→IP bindings
addn-hosts=/tmp/mjolnir/dns                # CRDT-synced hostname→IP
dhcp-script=/usr/bin/mjolnir-mesh dhcp-event  # reports new leases to CRDT
```

**Flow when a device connects:**

1. Device sends DHCP Discover on the L2 broadcast domain
2. One or more routers receive it; each checks `/tmp/mjolnir/reservations`
3. If the MAC has an existing binding, dnsmasq renews that IP (roaming case)
4. If the MAC is new, dnsmasq assigns a free IP and invokes `dhcp-event`
5. `mjolnir-mesh dhcp-event` writes the binding to the local CRDT store
6. iroh-gossip broadcasts the new binding to all mesh routers (~100ms)
7. All routers update their reservations file; dnsmasq on each router
   now knows that IP is taken

**Roaming** is a renewal, not a conflict — the same MAC reclaims its existing
IP from a new router. The CRDT treats this as an in-place update; FWW conflict
resolution only applies when two different MACs race for the same IP.

---

## Conflict Resolution

Two routers can assign the same IP to different devices within the ~100ms gossip
window. This is rare but handled cleanly:

- **FWW (first-writer-wins)** with HLC ordering — lower timestamp wins
- **Losing router**: removes the lease from dnsmasq, deauths the losing device
- **Losing device**: auto-reconnects (~2 sec), gets a new non-conflicting IP
- **Winning device**: completely undisturbed

Lease times can be long (1 hour+). Conflicts are resolved immediately on
detection, not deferred to renewal time.

---

## CRDT Store

A custom eventually-consistent KV store built on iroh-gossip (not iroh-docs,
which is not available as a shipped crate):

| Concern | Mechanism |
|---------|-----------|
| Local state | In-memory HashMap + optional disk persistence (SD card) |
| Hot path | iroh-gossip broadcast to all peers |
| Catch-up | Anti-entropy full state exchange on reconnect |
| Conflict rule | FWW + HLC for leases; last-write-wins for DNS/services/routes |

**Schema:**

```
/devices/{mac}       →  { ip, hostname, router_id, expiry }
/dns/{hostname}      →  { ip, mac }
/services/{name}     →  { hostname, ip, port, protocol, txt }
/routes/{subnet}     →  { node_id, via_node_id, site, expires }
```

**Anti-entropy**: When a router rejoins after a partition, it performs a full
state exchange with its peers. Both sides apply FWW merge and converge without
re-broadcasting. Subsequent queries reflect the merged state.

---

## Service Discovery

Any device can publish a service into the mesh CRDT:

```
/services/wiki  →  { hostname: "wiki.mesh", ip: 10.42.1.50, port: 8080 }
```

All routers update their DNS. Anyone on any site reaches `wiki.mesh:8080`.
This works across both local and remote modes.

### DNS Naming

The mesh uses the `.mesh` domain for all daemon-managed names, avoiding `.local` to prevent avahi/mDNS naming collisions. Hostnames come from DHCP Option 12 with a MAC-derived fallback for devices that don't set one.

---

## Iroh as Global Address Space

```
    Global identity:  Iroh NodeId  (routers, VMs, Iroh-aware devices)
    Local identity:   IPv4 address (consumer devices, standard apps)
    Bridge:           DNS + CRDT   (hostname → IP  or  hostname → NodeId)
```

Iroh NodeId is a public key derived from a secret key stored on each router.
It serves as a stable, globally routable identity that persists across IP changes,
NAT boundaries, and site moves. The mesh DNS namespace spans both:

- `device.mesh` → `10.42.1.42` (IPv4 for local LAN apps)
- `router-b.mesh` → `<NodeId>` (Iroh for direct peer connections)

---

## Relationship to Mjolnir VMs

Mjolnir VMs have Iroh built into their network stack. VMs join the same Iroh
mesh as the routers and appear in the shared DNS namespace:

```
Mesh namespace (routers + VMs + devices):
  Router-A  10.42.1.1   node_abc...
  Router-B  10.42.1.2   node_def...
  VM-nginx  10.42.1.100 node_ghi...   ← Mjolnir VM
  laptop    10.42.1.42  (IPv4 only)
  phone     10.42.1.43  (IPv4 only)
```

Any device on any router can resolve `nginx.mesh` and connect to the VM.
The mesh becomes a unified namespace spanning infrastructure and clients.

---

## Architecture Layers

```
┌──────────────────────────────────────────────────────┐
│  Applications / Devices                              │
│  standard DHCP + DNS clients, mDNS, Iroh-aware apps  │
├──────────────────────────────────────────────────────┤
│  dnsmasq  (per router)                               │
│  DHCP server, DNS resolver                           │
│  reads /tmp/mjolnir/reservations + /tmp/mjolnir/dns  │
│  invokes mjolnir-mesh dhcp-event on lease events     │
├──────────────────────────────────────────────────────┤
│  mjolnir-mesh daemon  (Rust binary)                  │
│  CRDT store (FWW + HLC)                              │
│  iroh-gossip replication                             │
│  dnsmasq file writer                                 │
│  cross-site route management                         │
├──────────────────────────────────────────────────────┤
│  Iroh  (QUIC mesh)                                   │
│  encrypted connections, NAT traversal, node identity │
└──────────────────────────────────────────────────────┘
```

---

## Implementation Roadmap

**Phase 1 — Core daemon**: CRDT store, gossip replication, dnsmasq integration
(hostsfile + dhcp-script + dns), single-router proof of concept.

**Phase 2 — Multi-router local mesh**: L2 bridging, multi-dnsmasq coordination,
conflict resolution via deauth, roaming.

**Phase 3 — Remote sites**: Iroh tunnel routing, cross-site DNS, route table CRDT.

**Phase 4 — Services and discovery**: Service registration, mDNS forwarding,
avahi integration.

**Phase 5 — OpenWrt packaging**: Cross-compilation for ARM/MIPS, init scripts,
UCI integration.

---

## Dependencies

| Crate | Version | Role |
|-------|---------|------|
| iroh | 0.96 | QUIC mesh, NAT traversal, node identity |
| iroh-gossip | 0.96 | Gossip broadcast topics |
| postcard + serde | — | Serialization |
| tokio | — | Async runtime |
| clap | — | CLI |
| tracing | — | Structured logging |

iroh is pinned to 0.96 due to `web-transport-iroh 0.2.2` compatibility —
`Session::raw()` requires matching `Connection` types. Upgrade planned when
dependencies align.

---

## Architecture Details

- **CRDT design**: `docs/architecture/dhcp-crdt.md`
- **Network topology**: `docs/architecture/network-architecture.md`
- **dnsmasq integration**: `docs/architecture/dnsmasq-integration.md`
- **Vision**: `docs/vision/why-decentralized-mesh.md`

## Crate Architecture

Two transports live side by side, picked by workload topology:

* **Direct iroh bidi streams** (`crates/mjolnir-node/src/audio_proto.rs`) for the
  full-mesh real-time case — N peers each sending a small Opus frame every 20 ms
  to every other peer. ~150 LOC, no protocol overhead.
* **MoQ over WebTransport** (`crates/mjolnir-moq`) for the one-publisher /
  many-subscriber broadcast case — video, screen-share, recorded streams. MoQ's
  named broadcasts, group sequencing, and cache-aware stream layouts pay rent
  in that topology; they don't in audio mesh.

### Current
- `mjolnir-node` — binary crate. CLI entry, room logic, gossip-driven peer discovery, and the audio bidi-stream protocol (`audio_proto.rs`).
- `mjolnir-audio` — Opus capture/encode/decode/playback + jitter buffer + per-peer mixer with pluggable PLC. No mesh awareness.
- `mjolnir-media` — codec-agnostic media primitives (sequence-keyed jitter ring, `Recover` trait, `SelfHealingBuffer`).
- `mjolnir-moq` — MoQ-over-WebTransport bridge. Adapts iroh connections to moq-lite sessions. Reserved for broadcast-topology workloads (video, screen-share); currently not used by the audio path.

### Planned (Mesh Library Extraction)
- `mjolnir-mesh` — new lib crate. Owns: iroh endpoint, gossip, CRDT store, DHCP/DNS coordination, routing. Exposes a generic `publish_stream(name)` / `on_peer_stream(peer_id)` surface plus the MoQ pub/sub stack for broadcast workloads.
- `mjolnir-node` — becomes a thin binary depending on mjolnir-mesh + mjolnir-audio.
- `mjolnir-moq` — stays available for broadcast-topology consumers (e.g. video).
- `mjolnir-audio` — stays in application layer; consumers wire the audio bidi protocol against `mjolnir-mesh`'s endpoint.

The DHCP/DNS/CRDT coordination described in this doc suite lives in the mjolnir-mesh lib crate — it is networking infrastructure, not application logic.

## Reading Order

For newcomers to the project:
1. [Why Decentralized Mesh](vision/why-decentralized-mesh.md) — motivation and big picture
2. [This document](mesh-network-coordination.md) — architecture overview
3. [Network Architecture](architecture/network-architecture.md) — local vs remote modes, routing
4. [DHCP CRDT](architecture/dhcp-crdt.md) — distributed state design and Rust types
5. [dnsmasq Integration](architecture/dnsmasq-integration.md) — practical integration reference
6. [Mjolnir Integration](vision/mjolnir-integration.md) — how the mesh ties into Mjolnir's microVM platform