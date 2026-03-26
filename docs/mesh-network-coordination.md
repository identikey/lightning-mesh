# Mesh Network Coordination: Vision and Architecture

**Status:** Vision document | **Date:** 2026-03-25

mjolnir-mesh is expanding from a WebRTC audio signaling server into a distributed mesh coordination daemon for ad-hoc router networks. The system will replicate DHCP lease tables and DNS records across OpenWrt routers (GL.iNet devices like AXT1800, BE3600) using Iroh's QUIC mesh and CRDTs, enabling seamless roaming, hostname-based service discovery, and partition-tolerant network coordination.

## Problem Statement

When multiple OpenWrt routers form an ad-hoc mesh, each runs its own dnsmasq DHCP/DNS daemon. Today, these servers operate independently, causing critical failures:

- **IP collision**: Two routers can assign the same IP address to different devices because they lack shared lease state.
- **Hostname fragmentation**: A device on Router-A cannot resolve the hostname of a device on Router-B.
- **No seamless roaming**: When a device roams from Router-A to Router-B, it loses its IP lease and must request a new one, breaking ongoing connections.
- **No partition recovery**: When a router rejoins the mesh after a network split, its stale lease table has no way to sync with others.

**Solution**: Use iroh-docs (a replicated CRDT key-value store) to sync DHCP leases and DNS records across all mesh nodes, backed by iroh-gossip for low-latency broadcast. dnsmasq on each router reads the merged state, providing a unified address space across the mesh.

## Architecture Layers

```
┌─────────────────────────────────────────┐
│  dnsmasq (DHCP/DNS per-router)          │
│  reads /tmp/leases/mesh + /tmp/hosts/mesh
├─────────────────────────────────────────┤
│  Lease/DNS Sync Engine (CRDT)           │
│  iroh-docs: replicated KV store         │
│  iroh-gossip: broadcast topics          │
├─────────────────────────────────────────┤
│  Iroh Node (QUIC mesh)                  │
│  Encrypted connections, NAT traversal   │
└─────────────────────────────────────────┘
```

### Layer 1: Iroh Node Foundation

Each router runs a single Iroh endpoint, providing:

- **QUIC-based mesh**: Encrypted, hole-punched connections between routers
- **NAT traversal**: Via built-in relay infrastructure
- **Content-addressed data**: All state is reproducible from a DAG of operations
- **Node discovery**: Via gossip and n0 (decentralized discovery)

The Rust crates already exist:

- `crates/mjolnir-node`: CLI binary with `host`/`join`/`id` subcommands. Owns an Iroh `Endpoint`, a `MoqBridge`, and a `Room`.
- `crates/mjolnir-moq`: MoQ protocol bridge (ALPN: `b"moq-lite/0"`) with actor-based session management.
- `crates/mjolnir-audio`: Opus codec scaffolding with cpal capture/playback.

### Layer 2: Distributed DHCP Lease Sync

**Problem solved**: Ensuring two routers never assign the same IP to different devices.

**Solution**: Use iroh-docs to replicate lease state with last-write-wins conflict resolution.

#### Lease Table Schema

The core data structure in the iroh-doc:

```
Key: /leases/{ip}
Value: {
  mac: "00:1a:2b:3c:4d:5e",
  hostname: "laptop-alice",
  expiry: 1711363200,  // Unix timestamp
  router_id: "node_xyz",  // Which router issued this lease
  claimed_at: 1711276800  // When the lease was claimed (for tie-breaking)
}

Key: /dns/{hostname}
Value: {
  ip: "192.168.1.42",
  type: "A",  // or AAAA
  ttl: 3600,
  router_id: "node_xyz"
}
```

#### Dual-Path Synchronization

**Hot path (gossip):** When a router's dnsmasq issues or renews a lease, the lease daemon broadcasts to all peers via iroh-gossip on topic `"dhcp-leases"`:

```json
{
  "action": "claim_lease",
  "ip": "192.168.1.42",
  "mac": "00:1a:2b:3c:4d:5e",
  "hostname": "device-name",
  "expiry": 1711363200,
  "router_id": "node_xyz"
}
```

All peers receive the message in ~10-100ms and add the entry to their iroh-doc immediately. The message is best-effort (UDP-like) but sufficient because:

- The iroh-doc persists the claim durably
- Retransmission happens on reconnection
- Gossip reaches all peers quickly enough to prevent collisions in normal operation

**Cold path (docs):** iroh-docs automatically syncs the full CRDT state when nodes reconnect:

1. Node A goes offline with leases `{IP1, IP2, IP3}`
2. Node B continues issuing new leases while A is away: `{IP4, IP5}`
3. When A rejoins, iroh-docs performs a merge: both A and B converge to `{IP1, IP2, IP3, IP4, IP5}` without re-broadcasting
4. Subsequent queries include merged state

### Layer 3: Shared DNS Resolution

Once leases are replicated across the mesh, DNS becomes unified.

**Per-router DNS update**: The lease sync engine writes the merged lease table to `/tmp/hosts/mesh`:

```
192.168.1.42  laptop-alice  laptop-alice.local
192.168.1.43  printer-office  printer-office.local
192.168.1.50  camera-kitchen  camera-kitchen.local
```

**dnsmasq configuration**: Each router's dnsmasq includes this file:

```
addn-hosts=/tmp/hosts/mesh
```

Now every router's DNS resolver answers queries for any hostname in the mesh, regardless of which router issued the lease.

**Result**: A device on Router-A connecting to Router-B can resolve `printer-office.local` and connect to the printer, which is registered with Router-B.

## Seamless Roaming

When a device moves from Router-A to Router-B:

1. **Device sends DHCP discover** to Router-B's broadcast domain
2. **Router-B's dnsmasq** checks the iroh-doc: `IP1` is already leased to `laptop-alice` with `mac_A`
3. **Router-B** sees the MAC is roaming and updates the lease entry: same IP, same hostname, but `router_id` now points to Router-B
4. **No disruption**: The device keeps its IP, and any peer can resolve `laptop-alice` to the same IP regardless of which router it's connected to

**Key insight**: The CRDT with per-author-key semantics (Iroh's native model) handles this automatically. The latest timestamp from any router wins, so a router can claim an existing lease as long as its timestamp is newer.

## Partition Tolerance

**Scenario**: A travel router gets unplugged (network partition).

**What happens**:

- **Remaining mesh**: Other routers retain the full lease table in their iroh-docs. They continue issuing new leases and syncing via gossip.
- **Travel router**: Offline, has a stale snapshot of state at time T.
- **Rejoin**: Router reconnects to the mesh. iroh-docs syncs automatically:
  - Leases issued while offline are merged in
  - Travel router's old leases are retained (they may have expired, but that's OK—dnsmasq handles expiry)
  - Conflicts: Latest-timestamp-wins (Iroh's native semantics)

**No manual intervention needed.** The CRDT merge is automatic.

## Iroh Primitives Used

### iroh-gossip: Broadcast Channel

Topic: `"dhcp-leases"`

- **Purpose**: Low-latency notification of new/updated leases
- **Semantics**: Best-effort broadcast to all peers
- **Payload**: Serialized lease claim (JSON or postcard)
- **Latency**: ~10-100ms to most peers (depends on network topology)
- **Why gossip and not docs?** Gossip is faster for hot notifications. Docs handles durability and catch-up.

### iroh-docs: CRDT Store

- **Namespace**: Hierarchical keys like `/leases/{ip}` and `/dns/{hostname}`
- **Authors**: Each router is an author; it can modify entries under its own `router_id` prefix
- **Conflict resolution**: Per-key, per-author (Iroh's native model). Tie-breaker: timestamp or content hash.
- **Automatic sync**: When nodes connect, iroh-docs performs a 3-way merge
- **Durability**: All state is persisted to disk and recoverable on restart

### iroh-net: Encrypted QUIC Transport

- **Connections**: Point-to-point QUIC between routers
- **NAT traversal**: Automatic hole-punching via relay infrastructure
- **Encryption**: QUIC's native TLS 1.3, per-connection
- **Peer discovery**: Via gossip, n0, or static addresses

## Deployment Target

A single Rust static binary per router:

```bash
mjolnir-mesh-daemon --router-id node_xyz --iroh-secret <key>
```

**Runs on**: OpenWrt (ARM/MIPS architectures). Cross-compiled via Cargo with OpenWrt toolchain.

**Resource footprint**:
- **Memory**: ~20-30 MB (Rust binary, Iroh endpoint, doc store)
- **CPU**: Minimal when idle; spikes during lease/gossip broadcasts
- **Storage**: Lease table grows by ~100 bytes per device per year; docs store is append-only but garbage-collectible

**Integration points**:

1. **Watch dnsmasq leases** (`/tmp/dhcp.leases`) via inotify
2. **Broadcast** new/renewed leases via iroh-gossip (topic: `"dhcp-leases"`)
3. **Merge state** from iroh-docs into local lease table
4. **Write output** to `/tmp/hosts/mesh` for dnsmasq to read
5. **Listen** for updates from other routers via iroh-gossip + iroh-docs

## Relationship to Mjolnir VMs

Mjolnir microVMs already have Iroh built into their network stack. This mesh coordination layer creates a unified address space spanning routers + VMs:

- A VM can join the same Iroh mesh as the routers (sharing transport)
- A device connected to any router can resolve and connect to services running in any VM
- VMs appear in the shared DNS namespace alongside router-issued leases

**Example workflow**:

```
OpenWrt Mesh (routers + VMs):
  Router-A (192.168.1.1)
    └─ Device: laptop (192.168.1.42)
  Router-B (192.168.1.2)
    └─ Mjolnir VM running nginx (192.168.1.100)

Laptop can:
  1. SSH to another device via hostname (dnsmasq resolves it)
  2. Roam from Router-A to Router-B without IP change
  3. Access services in the Mjolnir VM by hostname (nginx.local)
```

## Future Extensions

### 1. Firewall Rules Propagation

Extend the doc schema to include per-device firewall rules:

```
Key: /firewall/{device_id}
Value: {
  rules: [ { port: 22, protocol: tcp, action: accept } ],
  router_id: "node_xyz"
}
```

Each router's firewall daemon reads the merged rules and applies them locally.

### 2. Network Topology Awareness

Use Iroh's connection metrics to build a dynamic topology map:

```
Key: /topology/{link}
Value: {
  from: "node_xyz",
  to: "node_abc",
  latency_ms: 25,
  bandwidth_mbps: 100,
  last_update: 1711276800
}
```

Routing daemons can use this to optimize path selection.

### 3. Service Discovery

A generalized mDNS-like protocol over the mesh:

```
Key: /services/{service_type}/{instance}
Value: {
  hostname: "printer.local",
  port: 9100,
  txt_records: { color: true, model: "HP-4050" },
  router_id: "node_xyz"
}
```

Clients query `/services/_http._tcp/*` to discover web services across the entire mesh.

## Implementation Roadmap

### Phase 1: Core DHCP Lease Sync (MVP)

**Deliverable**: Single router can replicate its lease table to another via iroh-docs, and both serve the same DNS.

**Work items**:

1. `LeaseWatcher`: Watch `/tmp/dhcp.leases` (dnsmasq output) via inotify
2. `LeaseSync`: Module to publish/subscribe lease claims via iroh-gossip and iroh-docs
3. `MeshDns`: Merge iroh-doc state into `/tmp/hosts/mesh`
4. `DaemonConfig`: CLI flags for `--router-id`, `--iroh-secret`, `--dnsmasq-leases-path`
5. Tests: Simulate two routers, verify lease collision prevention

**Effort**: ~2-3 weeks (Rust/Tokio)

### Phase 2: Multi-Router Mesh

**Deliverable**: 3+ routers form a mesh, all syncing leases automatically.

**Work items**:

1. Iroh endpoint bootstrapping: n0 discovery or static seed nodes
2. Gossip join/leave handling: Update mesh topology on router connect/disconnect
3. Expiry handling: Lease TTL enforcement, garbage collection
4. Integration test: Spin up 3 VM routers, verify full mesh sync

**Effort**: ~2 weeks

### Phase 3: Roaming and Partition Recovery

**Deliverable**: A device can roam between routers and keep the same IP. Partition-rejoins merge automatically.

**Work items**:

1. MAC-based lease claim logic: Detect roaming, update `router_id` in-place
2. Timestamp tie-breaker: Implement Iroh's per-author-key conflict resolution
3. Partition test: Simulate network split, verify merge on rejoin
4. Stability test: Run 3 routers + 10 devices for 24 hours, measure DNS resolve latency

**Effort**: ~2-3 weeks

### Phase 4: OpenWrt Deployment

**Deliverable**: Static binary runs on actual GL.iNet routers.

**Work items**:

1. Cross-compile for ARM/MIPS (OpenWrt targets)
2. Init script: Systemd or UCI integration
3. dnsmasq config: Patch `/etc/dnsmasq.conf` to include `/tmp/hosts/mesh`
4. Monitoring: Prometheus metrics export (optional)

**Effort**: ~1-2 weeks

## Security Considerations

### Node Identity and Authentication

Each router is identified by its Iroh `NodeId` (public key derived from `SecretKey`). No additional identity layer is needed; Iroh's encryption is sufficient.

**Trust model**: All routers in a mesh trust each other. This is appropriate for a single organization's router mesh. For federated meshes (multiple organizations), add OAuth2/JWT layer on top.

### DHCP Lease Integrity

Leases are mutable by any router (any author can write to iroh-docs). For single-organization deployments, this is acceptable because:

1. All routers are under one operator's control
2. Iroh provides tamper-evidence via content-addressed data
3. Leases are cached locally and self-healing (expired leases are ignored)

For untrusted peers, add a signature layer:

```
Value: {
  claim: { ip, mac, hostname, expiry },
  signature: sign(claim, router_private_key),
  router_id: node_xyz
}
```

Each router validates the signature before accepting a lease claim.

### DNS Spoofing

Writing `/tmp/hosts/mesh` allows any peer to inject hostnames. Mitigation:

1. Use DNSSEC on the dnsmasq resolver for upstream queries
2. For mesh hostnames, validate that the entry came from a trusted router (via signature layer above)
3. TTL-based expiry: Stale DNS entries eventually disappear

## Testing Strategy

### Unit Tests

- Lease parsing from dnsmasq format
- CRDT merge logic (especially conflicts and expiry)
- Gossip message serialization

### Integration Tests

- Two-router sync: Host leases on Router-A, verify Router-B reads them
- Multi-router gossip: Verify all routers receive broadcasts
- Partition recovery: Split mesh, rejoin, verify state consistency
- Roaming: Device moves from Router-A to Router-B, keeps same IP

### Stability Tests

- 3+ routers, 10+ devices, 24-hour run
- Periodic partitions and heals
- High churn: Devices joining/leaving frequently
- Measure: DNS resolve latency, lease replication time, memory growth

### Performance Benchmarks

- Lease claim broadcast latency (p50, p95, p99)
- Doc sync time for 1000 leases
- DNS query latency (/tmp/hosts/mesh)
- Binary size and memory footprint on OpenWrt

## References

**Iroh**:
- `iroh` 0.97: QUIC mesh, NAT traversal, node discovery
- `iroh-gossip` 0.97: Broadcast topics, best-effort messaging
- `iroh-docs` (planned 0.97+): CRDT key-value store with automatic merge

**Router Platforms**:
- OpenWrt: Linux-based firmware for routers (openwrt.org)
- GL.iNet devices: AXT1800, BE3600 (GL.iNet.biz)

**Relevant RFCs**:
- RFC 3315: DHCPv6
- RFC 2131: DHCP
- RFC 1035: DNS

**Project Files**:
- `crates/mjolnir-node/src/mesh.rs`: Iroh endpoint setup
- `crates/mjolnir-node/src/ticket.rs`: Join ticket generation
- `crates/mjolnir-moq/src/lib.rs`: MoQ bridge (can be extended for lease topics)
- Cargo workspace: `Cargo.toml` (iroh, iroh-gossip, tokio, tracing)