# Mjolnir + mjolnir-mesh: A Unified Compute + Networking Fabric

> **Status (2026-07-02):** This is a target-state integration vision. The mesh
> transport and CRDT subnet-claim data plane exist today (field-validated,
> `crates/mjolnir-mesh`), but VM registration, `.mesh` service DNS, and
> per-device CRDT registries are design-stage — they are the substance of the
> service-mesh phase (bead `e21`).

mjolnir-mesh (the router mesh network) and Mjolnir (the microVM orchestration platform) are separate systems built on the same transport layer — Iroh QUIC. Together, they form a platform where any device on the mesh can spawn and reach VMs anywhere, and any VM can be discovered by hostname. This document describes their integration and the architectural vision it enables.

## The Three Layers

```
┌─────────────────────────────────────────────────────────────┐
│  Applications & Agents                                       │
│  AI agents (MCP), developers (SSH), devices (HTTP/DNS)       │
├─────────────────────────────────────────────────────────────┤
│  Service Discovery                                           │
│  .mesh DNS — any service on any node, resolvable everywhere  │
├──────────────────────┬──────────────────────────────────────┤
│  NETWORKING           │  COMPUTE                             │
│  mjolnir-mesh         │  Mjolnir                             │
│  ┌────────────────┐   │  ┌────────────────────────────┐     │
│  │ Router nodes    │   │  │ VM nodes                    │     │
│  │ DHCP/DNS/CRDT   │   │  │ microVM spawn/snapshot     │     │
│  │ WiFi coverage   │   │  │ BTRFS CoW cloning          │     │
│  │ Device roaming  │   │  │ Guest agent (Iroh)          │     │
│  └────────────────┘   │  └────────────────────────────┘     │
├──────────────────────┴──────────────────────────────────────┤
│  Iroh (shared transport layer)                               │
│  NodeId identity, QUIC connections, gossip, NAT traversal    │
└─────────────────────────────────────────────────────────────┘
```

**Iroh** provides the foundation: encrypted P2P connections, global identity via NodeId (Ed25519 keypair), and NAT traversal. Both routers and VMs speak Iroh natively.

**mjolnir-mesh** provides the network: a CRDT-synced mesh that coordinates DHCP, DNS, routes, and service discovery across routers. Any device on the mesh can reach any other device by hostname.

**Mjolnir** provides the compute: a cluster of nodes running microVMs with 125ms boot time via BTRFS CoW cloning. Each VM has an Iroh endpoint and can join the mesh.

## VMs Join the Mesh Natively

A Mjolnir VM already has an Iroh NodeId and an IP on the local subnet. When it registers with the mesh CRDT, it becomes discoverable:

- The Mjolnir host (where the VM is running) writes the VM's details to the mesh CRDT:
  ```
  /devices/{vm_mac}    → { ip: "10.200.45.123", hostname: "dev-vm", router_id: host_node_id }
  /services/dev-vm-ssh → { hostname: "dev-vm.mesh", ip: "10.200.45.123", port: 22 }
  /dns/dev-vm          → { ip: "10.200.45.123" }
  ```

- Every dnsmasq on every router reads this CRDT and resolves `dev-vm.mesh` to the VM's IP
- A developer on Router-5 (coffee area) can `ssh dev-vm.mesh` and connect to the VM running on the host in the server room
- No ticket sharing, no special clients — just standard DNS + IP routing

## Spawn Anywhere, Reach Everywhere

Today's workflow:
```
Developer calls spawn_vm() via MCP
→ VM boots on Mjolnir host
→ Developer gets Iroh ticket
→ Shares ticket out of band
→ Someone else connects
```

With mesh integration:
```
Developer calls spawn_vm() via MCP
→ VM boots on Mjolnir host
→ Host writes VM to mesh CRDT
→ `agent-workspace.mesh` appears in DNS across all routers
→ Other agents and developers discover and connect via hostname
```

Standard tools work everywhere:
```bash
# From any device on any router:
ssh workspace.mesh                  # Point-to-point SSH
curl wiki.mesh:8080                 # Web service on a VM
ping compute-node.mesh              # Ping a VM by hostname
```

## Local Hardware + Cloud = One Mesh

A DWEB event scenario:
- **Networking:** 10 GL.iNet routers (Router-1 through Router-10) plugged in around the venue, forming a local mesh via L2 bridge
- **Local compute:** 2 mini PCs under tables run Mjolnir, each hosting multiple VMs
- **Cloud compute:** 3 cloud Mjolnir nodes provide additional capacity, connected via Iroh tunnels
- **One mesh:** All routers and VMs coordinate through the same CRDT

A developer's phone on Router-5 WiFi can:
- `ssh workshop-vm.mesh` → VM on the mini PC (local, low latency)
- `ssh analysis-vm.mesh` → VM on cloud (via Iroh tunnel, higher latency but works)
- `ping projector.mesh` → Projector on Router-2 (local mesh DNS)
- All by hostname. Same UX regardless of where the resource lives.

## MCP as the Control Plane

AI agents orchestrate compute via MCP. The mesh provides discovery:

1. Agent calls `spawn_vm(name="workspace", enable_iroh: true)` via MCP
2. VM boots in ~125ms
3. Mjolnir host writes VM to mesh CRDT → `workspace.mesh` is now resolvable
4. Agent calls `get_connection_ticket()` for direct Iroh access, or relies on mesh DNS for standard tools
5. Agent uses the VM (via `exec()` or SSH)
6. Other agents discover `workspace.mesh` via mesh DNS and can coordinate work
7. Agent calls `create_snapshot()` → instant checkpoint for later reuse
8. Agent calls `stop_vm()` → VM goes dormant, wakes on message delivery

Multi-agent coordination becomes natural: no out-of-band ticket sharing, just mesh DNS discovery.

## Integration Points

### VM Registration

When Mjolnir spawns a VM with `enable_iroh: true`, the host registers it in the mesh CRDT. The registration includes:
- VM MAC address (unique key in `/devices/`)
- Assigned IP on the Mjolnir host's subnet
- Hostname (derived from VM name or provided explicitly)
- Services running (populated by the guest agent via heartbeat)
- Router NodeId (which Mjolnir host is running it)

### Route Advertisement

The Mjolnir host advertises its VM subnet range (10.200.0.0/10 or a narrower range) in the CRDT subnet claim ledger (Babel handles actual route propagation):

```
/subnets/10.200.0.0_10  → { owner_node_id: host_node_id, site_name: "vm-host", claimed_at: <hlc> }
```

Other routers read this and install routes via Iroh tunnels to reach VMs. Devices on any router can then reach any VM by IP (or hostname via DNS).

### Dormant VM Wake-up

A message to `dormant-vm.mesh` could trigger wake-up:
1. Router receives traffic destined for the dormant VM's IP
2. Router checks CRDT for the VM's home host (where it's checkpointed)
3. Router sends a wake-up message to that host via Iroh
4. Mjolnir host receives the message, restores the VM from BTRFS snapshot
5. Host sends the queued traffic to the VM
6. From the sender's perspective: brief delay, then connection succeeds

This enables serverless-like patterns: checkpoint a VM, it goes dormant consuming zero CPU/RAM, wakes on first traffic.

### Snapshot Distribution

BTRFS snapshots are immutable and could be shared across Mjolnir nodes via the mesh:
- Node-A creates a snapshot of a base image (e.g., `ubuntu-24.04-dev`)
- Snapshot metadata is written to CRDT: `/snapshots/ubuntu-24.04-dev`
- Node-B requests the snapshot via Iroh, receives it, stores it locally
- Any node can now spawn from that snapshot without reimplementing the base image

## Security Model

### Transport Layer

Iroh provides encryption at the transport level: QUIC with TLS 1.3 is mandatory and cannot be disabled. All connections between routers and between VMs and routers are encrypted.

### Identity and Membership

- **Iroh NodeId** = router or host identity (Ed25519 public key)
- **VM identity** = MAC address, scoped to its host (VM cannot migrate between hosts without re-registration)
- **Future mesh membership:** CRDT membership list with signed invitations; any existing member can invite a new node, revocation via CRDT tombstone
- **Current MVP:** Pre-shared key (PSK) derived into the gossip topic to prevent unauthorized joining

### Network Isolation

- VM isolation: KVM hypervisor boundary (real hardware isolation, not just containers)
- Router-to-router forwarding: iptables rules restrict IP forwarding to known mesh subnets; no arbitrary relay
- Route validation: CRDT entries are signed by their announcing node; routes are only installed if the source NodeId is trusted

## What This Enables

### For Developers

- Spin up a VM → instant hostname → instant discovery across the mesh
- SSH into VMs by hostname, not by ticket
- Collaborate: other developers see the VM in mesh DNS, no sharing needed
- Snapshot and share workspaces instantly

### For Events

A DWEB conference with 200+ attendees and 10 routers:
- Network self-organizes: plug in routers, they join the mesh, WiFi covers the venue
- Compute on-demand: organizers bring mini PCs running Mjolnir, spawn VMs for workshops
- Services discoverable: workshop VMs appear as `workshop-1.mesh`, `workshop-2.mesh`, etc.
- Resilient: any router can go down, the mesh adapts; any VM can be checkpointed and moved

### For AI Agents

- Multi-agent orchestration: agents spawn VMs on different hardware, discover each other by hostname
- Checkpointing: agents create snapshots of progress, other agents restore and continue
- Decoupled operation: agents don't coordinate tickets; the mesh provides the discovery layer
- Cost-effective: scale compute with dormant VMs; wake only when needed

### For Community Networks

- Local services (wikis, file servers, game servers) discoverable by `.mesh` hostname
- Global reach: nodes can connect via Iroh from anywhere, still reachable as `service.mesh`
- No central server: all routers and VMs coordinate peer-to-peer
- Resilient: any node can fail, the rest adapt

## Mesh Access vs Gateway Access

Mjolnir provides two complementary paths to reach VMs: the **mesh** (local/event) and the **gateway** (`vm.worldtree.network`, a central HTTP-to-Iroh bridge). They serve different audiences.

### How They Work

```
Mesh path:    Device → WiFi → Router → Iroh tunnel or L2 → VM
Gateway path: Browser → Cloudflare → Gateway server → Iroh QUIC → VM
```

The gateway translates HTTP requests to Iroh connections using a z32-encoded NodeId subdomain (e.g., `b4qf3tl0yd.vm.worldtree.network`). It supports HTTP, WebSocket, and SSE — any TCP-based protocol that starts with an HTTP Host header.

### Comparison

| | Mesh | Gateway |
|---|---|---|
| **Latency** | 1-5ms (same L2), 10-50ms (Iroh tunnel) | 100-300ms (Cloudflare → gateway → relay) |
| **Protocols** | Any IP protocol (SSH, HTTP, DB, gRPC, UDP) | HTTP-shaped only (needs Host header to route) |
| **Internet required** | No — works entirely on LAN | Yes — both ends need internet |
| **Discovery** | `.mesh` DNS, per-service (`vm.mesh:22`, `vm.mesh:3000`) | One URL per VM |
| **Setup** | Connect to mesh WiFi | None — any browser, any network |
| **Infrastructure** | Routers only, no external dependencies | Requires gateway server, DNS, Cloudflare, TLS |
| **Single point of failure** | None — any router can route | Gateway server down = no web access |
| **Audience** | People at the event / on the mesh | Anyone on the internet |

### When to Use Which

**Mesh** — You're at the event, on the local network, or connected to a mesh router remotely via Iroh. You want low latency, full protocol support, and no cloud dependency. SSH, development, databases, real-time apps.

**Gateway** — You want to share a URL with someone not on the mesh. Public demos, remote participants, browser-first workflows. Click a link, it works.

**Both** — A VM is reachable via both paths simultaneously. `dev-vm.mesh` for local users, `b4qf3tl0yd.vm.worldtree.network` for remote users. Same VM, same services, different access paths.

### The Sovereignty Angle

At a DWEB event, internet may be flaky or absent. The gateway requires internet on both ends — no internet, no VM access. The mesh works entirely on local infrastructure: routers, Mjolnir hosts, and devices. Your hardware, your network, no cloud required.

The gateway is for reach. The mesh is for performance and sovereignty.

## References

- [Why Decentralized Mesh Networking](why-decentralized-mesh.md) — Motivation and vision for mjolnir-mesh
- [Network Architecture](../network-coordination/network-architecture.md) — CRDT, routing, subnet allocation
- [CRDT Design](../archive/network-coordination/dhcp-crdt.md) — Conflict-free replicated data for mesh state (archived design reference; live CRDT doc: [gossip-and-crdt](../network-coordination/gossip-and-crdt.md))
- [Mjolnir Documentation](https://mjolnir.local) — VM orchestration, snapshots, MCP tools
