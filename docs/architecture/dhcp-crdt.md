# DHCP CRDT Architecture: Distributed Lease Synchronization

**Status:** Architecture specification | **Date:** 2026-03-25 | **Author:** mjolnir-mesh team

This document specifies the design and implementation of the DHCP CRDT (Conflict-free Replicated Data Type) layer. It is the core data structure that enables multiple routers to maintain a consistent, shared DHCP lease table without a central authority. See [../mesh-network-coordination.md](../mesh-network-coordination.md) for the high-level vision.

---

## 1. Data Model

### 1.1 Lease Entry Schema

The fundamental unit of replication is a **lease entry**, which represents a single DHCP IP assignment.

```rust
/// A single DHCP lease in the shared mesh state.
/// Key: `/leases/{ip}` where {ip} is dotted-quad IPv4 (e.g., "192.168.1.42")
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseEntry {
    /// MAC address of the client device, colon-separated hex (e.g., "00:1a:2b:3c:4d:5e")
    pub mac: String,

    /// Hostname of the device, DNS-safe (e.g., "laptop-alice")
    /// May be empty if DHCP request did not include hostname
    pub hostname: String,

    /// Unix timestamp when lease expires (seconds since epoch)
    /// Routers use this to clean up stale entries locally
    pub expiry: u64,

    /// Iroh NodeId of the router that issued this lease
    /// Used to distinguish which router owns the claim for conflict resolution
    pub router_id: String,

    /// Unix timestamp when this lease was first claimed (seconds since epoch)
    /// Used as tie-breaker when multiple routers claim same IP simultaneously
    /// Highest timestamp wins under last-writer-wins policy
    pub claimed_at: u64,

    /// Lease duration in seconds (e.g., 3600 for 1 hour)
    /// Informational; expiry takes precedence for lifecycle
    pub duration_secs: u32,
}
```

**Field Semantics:**

- **mac**: Uniquely identifies the client. When a device roams from Router-A to Router-B, the MAC remains constant but `router_id` changes.
- **hostname**: May be DNS-unsafe (contains spaces, special chars). Must be sanitized before writing to `/tmp/hosts/mesh`.
- **expiry**: A lease is considered valid if `current_time < expiry`. Expired leases remain in the CRDT (they become stale) but should not be written to dnsmasq or DNS output.
- **router_id**: The Iroh `NodeId` of the issuing router, as a base32-encoded string or hex representation. Uniquely identifies the author in conflict scenarios.
- **claimed_at**: Critical for tie-breaking. If two routers claim the same IP within the gossip latency window (~100ms), this field determines which claim wins.

**Serialization:** Postcard binary format, packed efficiently for minimal wire size.

### 1.2 DNS Entry Schema

DNS entries map hostnames to IP addresses for shared resolution.

```rust
/// A DNS record in the mesh state.
/// Key: `/dns/{hostname}` where {hostname} is lowercased, DNS-safe
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsEntry {
    /// IPv4 address associated with this hostname
    pub ip: String,

    /// Record type: "A" (IPv4) or "AAAA" (IPv6), stored as string for extensibility
    pub record_type: String,

    /// Time-to-live in seconds; informational (dnsmasq enforces via file reload)
    pub ttl: u32,

    /// Iroh NodeId of the router that registered this record
    pub router_id: String,

    /// Timestamp when this record was created; used for LWW conflict resolution
    pub created_at: u64,
}
```

**Design note:** DNS entries are derived from lease entries (computed from `/leases/*` via a merge function). They are stored separately in the CRDT for fast hostname-to-IP lookups without scanning the lease table.

### 1.3 Key Space

The iroh-docs key namespace is hierarchical and carefully structured to avoid collision and enable efficient merging:

```
/leases/{ip}
  Example: "/leases/192.168.1.42"
  Multiple authors can write to the same key.
  Iroh stores all author versions; merge function selects canonical version.

/dns/{hostname}
  Example: "/dns/laptop-alice"
  Similar multi-author semantics.

/meta/routers
  Metadata: list of active router node IDs.
  Optional; used for topology awareness.

/meta/schema_version
  Tracks schema evolution. Current: "1"
```

**Why flat namespace with structured keys?**

- Simplifies iroh-docs API: keys are byte strings, no hierarchical index required.
- Enables prefix scans (e.g., list all leases with range query on `/leases/`).
- Avoids false namespace conflicts (e.g., "/leases/" as a key vs. "/leases/X").

### 1.4 Serialization Format

All values are serialized using **postcard** (binary, schema-less, deterministic).

```rust
// For a LeaseEntry:
let entry = LeaseEntry { /* ... */ };
let encoded: Vec<u8> = postcard::to_allocvec(&entry)?;
// Store encoded as iroh-doc value

// On read:
let value: Vec<u8> = doc.get(key).await?;
let entry: LeaseEntry = postcard::from_bytes(&value)?;
```

**Advantages:**

- **Compact**: ~80-120 bytes per lease (vs. ~200-300 for JSON)
- **Deterministic**: Identical Rust structs produce identical byte sequences (essential for content-addressed systems)
- **Versioning**: Postcard supports schema evolution via serde's `#[serde(default)]` fields
- **No custom encoding**: Leverages serde ecosystem; no hand-rolled parsing

---

## 2. CRDT Semantics

### 2.1 Iroh-Docs Model

Iroh-docs is a replicated key-value store with **per-key, per-author** conflict resolution:

- **Author**: Each router has a unique author ID (its Iroh `NodeId`).
- **Entry**: A (key, author, value, timestamp) tuple.
- **Per-key**: Multiple entries with the same key from different authors coexist.
- **Conflict resolution**: Readers see all versions and apply a deterministic merge function.

**Example scenario:**

```
Router-A claims 192.168.1.42 at timestamp=1000 with mac=AA:AA:...
Router-B claims 192.168.1.42 at timestamp=1001 with mac=BB:BB:...
(both happen within gossip latency, iroh-docs receives both)

When merged, both versions are stored. Merge function checks:
  - Router-B's timestamp (1001) > Router-A's timestamp (1000)
  - Result: Router-B's entry is canonical; Router-A's is archived
```

### 2.2 Merge Function: Last-Writer-Wins with Lease Semantics

The canonical merge function for `/leases/{ip}`:

```rust
/// Given all author versions for a lease key, return the canonical lease entry.
/// Implements last-writer-wins with tie-breaking on router_id for determinism.
pub fn merge_lease_versions(versions: Vec<(AuthorId, LeaseEntry)>) -> LeaseEntry {
    versions
        .into_iter()
        .max_by(|(author_a, entry_a), (author_b, entry_b)| {
            // Primary: higher claimed_at timestamp wins
            match entry_a.claimed_at.cmp(&entry_b.claimed_at) {
                std::cmp::Ordering::Equal => {
                    // Tie-breaker: lexicographically lower router_id (deterministic)
                    author_a.cmp(author_b)
                }
                other => other,
            }
        })
        .map(|(_, entry)| entry)
        .expect("versions must be non-empty")
}
```

**Why last-writer-wins (LWW) is correct for DHCP:**

1. **Roaming**: Device moves from Router-A to Router-B. Router-B updates the lease with a newer `claimed_at`, so its version wins. This allows Router-B to reclaim the same IP for the roaming device without collision.

2. **Partition recovery**: Router-A goes offline with a stale IP assignment. When it rejoins, other routers' newer timestamps dominate. The system converges to the correct state automatically.

3. **Expiry semantics**: Expired leases are stale but harmless (dnsmasq ignores them). LWW ensures that the most recent claim (whether issued by any router) is the source of truth.

4. **No revocation needed**: A router that claimed an IP doesn't need to explicitly release it. It simply stops issuing it and other routers' claims naturally supersede via newer timestamps.

### 2.3 Why Not Other Merge Strategies?

- **Content-hash-based (CRDTs)**: Would require consensus on hash algorithm and serialization order. Postcard makes this feasible but adds complexity; LWW is simpler and sufficient.
- **Vector clocks**: Would require all-to-all clock synchronization. In a mesh, this is expensive and introduces latency. Timestamps with NTP are practical for routers.
- **Application-level consensus**: E.g., "assign the IP to the first router to request it." Would require central coordination (Raft, Paxos) which defeats the purpose of CRDT.

### 2.4 Conflict Scenario: Simultaneous Claim

Two routers claim the same IP for different devices within the gossip window:

```
Time T=1000:
  Router-A: DHCP request from device-A (mac=AA:...)
            Claims 192.168.1.42, set claimed_at=1000, router_id=router_a
            Broadcasts via gossip immediately

  Router-B: DHCP request from device-B (mac=BB:...)
            Claims 192.168.1.42, set claimed_at=1000, router_id=router_b
            Broadcasts via gossip immediately

Time T=1050 (both gossip messages propagate):
  Router-A receives Router-B's claim. Compares:
    - claimed_at_a = 1000, claimed_at_b = 1000 (tie)
    - router_id_a vs router_id_b (lexicographic comparison)
    - Winner: whichever router_id sorts lower (deterministic)

  Router-B receives Router-A's claim. Same comparison leads to same result.

Time T=1100:
  All routers agree: one device owns 192.168.1.42. The other device is denied (dnsmasq rejects it).
  Problem: Device-B loses the IP. Router-B must issue a new lease from its pool.
```

**Mitigation:** Each router maintains a local **lease pool** (e.g., 192.168.1.100-192.168.1.254). Before claiming an IP, a router checks its local state AND the CRDT merge result. If a race occurs, dnsmasq's own DHCP logic handles the retry (client requests again). With proper pool partitioning (see § 5.2), collisions are rare.

---

## 3. Consistency Guarantees

### 3.1 What This System Guarantees

1. **Eventual consistency**: All routers converge to the same lease table within ~1-2 seconds in normal operation.
   - Gossip propagation: ~100ms
   - CRDT merge and update: ~50ms
   - Local dnsmasq reload: ~500-1000ms

2. **Durability**: Once a lease is written to iroh-docs and acknowledged by the author, it persists across router restarts and network outages.

3. **Partition tolerance**: If the mesh splits (e.g., travel router goes offline), each partition continues issuing leases independently. When they rejoin, the CRDT merge automatically incorporates all leases from both partitions.

4. **Causality**: If Router-A reads a lease from Router-B and then updates it, Router-A's update reflects the read state (via the `claimed_at` timestamp).

### 3.2 What This System Does NOT Guarantee

1. **Strong consistency**: In the first ~100-200ms after a lease is claimed, some routers may not yet know about it. A second device could be assigned the same IP if it contacts a different router in this window.
   - **Practical impact**: Minimal. DHCP lease requests are processed sequentially per router, and leases are long-lived (typically 1-24 hours). The collision window is <200ms, and the probability of two simultaneous requests is low.
   - **Mitigation**: See § 5.2 (pool partitioning).

2. **Atomic cross-key updates**: If a lease and its corresponding DNS entry must be updated together, there's no atomic transaction. Routers may see intermediate states (lease exists but DNS entry is stale).
   - **Practical impact**: DNS eventually reflects leases; temporary inconsistency is acceptable for a mesh LAN.

3. **Causal delivery of gossip**: Gossip messages may arrive out of order. If Router-A revokes a lease and then re-issues it, a peer may see the re-issue before the revocation.
   - **Practical impact**: Mitigated by lease expiry and LWW semantics. The most recent claim always wins.

### 3.3 Window of Vulnerability

In the worst case, two routers might assign the same IP to two different devices if:

1. Both routers' dnsmasq caches are missing the same IP (cache invalidated)
2. Both receive DHCP requests from different devices simultaneously
3. Neither has yet seen the other's gossip message
4. Both write to iroh-docs before either reads the update

**Probability**: Negligible in practice because:
- Lease TTL is long (hours), so dnsmasq cache is usually hot
- Gossip latency is <100ms, much faster than DHCP request processing interval
- Once one routers sees the other's claim, it marks the IP as in-use

**Recovery**: If a collision occurs, the device that loses the race gets DHCP NACK and must request again (DHCP standard). The winner keeps the IP.

---

## 4. Lease Lifecycle

### 4.1 Full Lifecycle Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│ LEASE LIFECYCLE: Device requests IP from Router-A                   │
├─────────────────────────────────────────────────────────────────────┤

T=0 (Local)
  Device sends DHCP DISCOVER to Router-A's broadcast domain

T=+10ms (Router-A processes)
  Router-A's dnsmasq receives request
  Dnsmasq checks its pool for free IP (e.g., 192.168.1.42)
  Dnsmasq sends DHCP OFFER with 192.168.1.42, TTL=3600s
  Dnsmasq writes entry to /tmp/dhcp.leases

T=+20ms (Hot path: gossip broadcast)
  mjolnir-mesh daemon wakes (inotify on /tmp/dhcp.leases)
  Creates LeaseEntry {
    ip: "192.168.1.42",
    mac: "aa:bb:cc:dd:ee:ff",
    hostname: "device-name",
    expiry: now + 3600,
    router_id: router_a_node_id,
    claimed_at: now_micros,
    duration_secs: 3600
  }
  Serializes with postcard
  Broadcasts message on iroh-gossip topic "dhcp-leases"
  All peers receive in ~10-100ms

T=+30ms (Cold path: docs write)
  mjolnir-mesh writes entry to iroh-docs key="/leases/192.168.1.42"
  Iroh persists entry to disk (author=router_a_node_id)
  Entry is immediately synced to all connected peers via iroh's background replication

T=+30-150ms (Peer side)
  Router-B, Router-C receive gossip message
  They verify the entry is not expired and MAC is not already known
  They update their local lease cache
  Router-B reads iroh-docs (either from gossip metadata or separate sync)

  (If Router-B had the same IP in its pool, it marks it as claimed now)

T=+100ms onward
  Device receives DHCP ACK from Router-A
  Device configures IP address 192.168.1.42
  Any peer can now resolve "device-name" to 192.168.1.42 via /tmp/hosts/mesh

  Example: Device on Router-B queries router_b.dnsmasq for "device-name"
           Router-B's dnsmasq reads /tmp/hosts/mesh (maintained by mjolnir-mesh)
           Returns 192.168.1.42 (from Router-A's claim)

T=+3600s (Expiry)
  Lease expires. Device should request renewal (DHCP RENEW).
  If device is still connected, dnsmasq renews it with same IP.
  If device is gone, lease entry remains in CRDT (stale) but dnsmasq ignores it.

T=+7200s (Garbage collection - optional)
  mjolnir-mesh periodically scans /leases/* for expired entries.
  Can delete them from iroh-docs (but not necessary; stale entries are ignored).
  Compact strategy: if >1000 stale entries, run GC to save memory.
```

### 4.2 Timing Breakdown

| Phase | Duration | Owner | Action |
|-------|----------|-------|--------|
| DHCP request → dnsmasq processes | 5-20ms | router-A dnsmasq | Issue OFFER, write /tmp/dhcp.leases |
| inotify wake → gossip broadcast | 5-15ms | mjolnir-mesh (hot path) | Parse lease, serialize, broadcast |
| Gossip propagation | 10-100ms | iroh-gossip | Fanout to all peers |
| iroh-docs write (persistent) | 5-20ms | mjolnir-mesh (cold path) | Serialize, append to doc store |
| Peer ingests gossip + docs | 20-100ms | router-b, router-c (peers) | Verify entry, update cache, write /tmp/hosts/mesh |
| dnsmasq reloads /tmp/hosts/mesh | 50-500ms | dnsmasq (on SIGHUP or inotify) | Parse file, update DNS resolver cache |
| Device receives DHCP ACK | 30-100ms | network | DHCP standard |
| **Total end-to-end propagation** | **~100-300ms** | **system** | Device can resolve hostname on any peer |

### 4.3 Lease Renewal

When a device renews its lease (DHCP RENEW):

1. Device sends DHCP REQUEST to the same router (Router-A)
2. Router-A's dnsmasq extends the lease in /tmp/dhcp.leases (updates expiry)
3. mjolnir-mesh detects the change (inotify) and broadcasts updated LeaseEntry
   - Same IP, same MAC, same hostname
   - **New claimed_at timestamp** (important: must update to maintain freshness)
   - Updated expiry
4. Peers receive the update and refresh their copies
5. No collision risk (same router, same device)

### 4.4 Lease Release

When a device leaves (sends DHCP RELEASE) or when a router voluntarily releases an IP:

**Option A: Passive expiry**
- Device goes offline without sending RELEASE
- Lease entry remains in CRDT with expiry timestamp
- Dnsmasq on all routers ignores it after expiry (checks `expiry < now()`)
- No explicit delete needed; system is self-healing

**Option B: Explicit delete** (optional, for cleanup)
- mjolnir-mesh detects dnsmasq removing entry from /tmp/dhcp.leases (inotify)
- mjolnir-mesh sends `LeaseAction::Release` via gossip
- Peers delete the entry from their local cache (but CRDT retains it for history)

For MVP, use Option A (passive expiry). It's simpler and eventual consistency naturally cleans up.

---

## 5. Conflict Scenarios

### 5.1 Two Routers Assign Same IP Simultaneously

**Scenario**: Device-A and Device-B send DHCP DISCOVER to Router-A and Router-B respectively, both targeting the same IP pool. Both routers have the IP 192.168.1.100 available (neither has seen the other's lease yet).

```
T=0:
  Device-A → DHCP DISCOVER to Router-A
  Device-B → DHCP DISCOVER to Router-B

T=+10ms:
  Router-A's dnsmasq sees Device-A, assigns 192.168.1.100 (mac AA:AA:...)
  Router-A writes to /tmp/dhcp.leases
  Router-B's dnsmasq sees Device-B, assigns 192.168.1.100 (mac BB:BB:...)
  Router-B writes to /tmp/dhcp.leases
  (Neither has yet heard of the other's assignment)

T=+20ms:
  Router-A broadcasts LeaseEntry(ip=192.168.1.100, mac=AA:AA:..., claimed_at=T0)
  Router-B broadcasts LeaseEntry(ip=192.168.1.100, mac=BB:BB:..., claimed_at=T0+5ms)

T=+100ms (all peers have both messages):
  Router-A, Router-B, and any other peer run merge_lease_versions():
    - claimed_at(A) < claimed_at(B)
    - Result: Router-B's entry wins
    - Device-A loses the IP

  Router-A's dnsmasq:
    - Learns from iroh-docs that Router-B claimed the IP
    - Marks 192.168.1.100 as in-use
    - Cannot renew Device-A's lease for that IP

  Router-A responds to Device-A with DHCP NACK (lease expired/unavailable)
  Device-A must send DHCP DISCOVER again → Router-A assigns a different IP (e.g., 192.168.1.101)
```

**Mitigation strategies:**

1. **Pool partitioning**: Assign each router a partition of the address space
   - Router-A: 192.168.1.100-150
   - Router-B: 192.168.1.151-200
   - Router-C: 192.168.1.201-250
   - Collisions are impossible if partitions don't overlap
   - Trade-off: Reduces flexibility if a partition exhausts its pool

2. **Reservation window**: Each router holds a reservation on each IP before handing it out
   - Broadcast claim → wait 50ms for responses → if no competing claim, issue lease
   - Trade-off: Increases latency (50ms for every DHCP request)

3. **Accept eventual consistency**: Collisions are rare (<200ms window + low request rate). DHCP retry handles it.
   - Trade-off: Requires robust DHCP client behavior (which is standard)

For MVP, use **strategy 3 + pool partitioning for safety**.

### 5.2 Device Roams Between Routers

**Scenario**: Device-A is connected to Router-A with IP 192.168.1.42. It moves to Router-B's range (different broadcast domain). It needs a new DHCP lease.

```
T=0:
  Device-A has IP 192.168.1.42 (issued by Router-A)
  Lease in CRDT: { ip, mac=AA:AA:..., router_id=router_a, claimed_at=T-1000 }

T=+10s (device moves):
  Device-A moves to Router-B, loses connection to Router-A
  Sends DHCP DISCOVER to Router-B

T=+20s:
  Router-B's dnsmasq receives DISCOVER from Device-A (mac=AA:AA:...)
  Checks CRDT state: IP 192.168.1.42 is leased to mac AA:AA:...
  BUT router_id=router_a, claimed_at=T-1000

  Router-B has two options:
    Option 1: Issue the same IP (if Router-A has relinquished it or device is roaming)
    Option 2: Issue a new IP

  **Roaming-aware design**: Device sends DHCP RENEW to Router-B
    Device includes DHCP client ID (based on MAC)
    Router-B checks if MAC matches existing lease in CRDT
    Router-B updates the lease in-place: same IP, same MAC, new router_id=router_b, new claimed_at=T+20s
    Router-B broadcasts updated entry via gossip
    Device gets DHCP ACK with 192.168.1.42

T=+100s:
  All routers see the updated entry (claimed_at=T+20s, router_id=router_b)
  This wins over the old entry (claimed_at=T-1000) via LWW
  Lease is now attributed to Router-B
  If Router-A had the old entry, it merges and accepts Router-B's newer version

Result:
  Device keeps IP 192.168.1.42
  Hostname (device-name) still resolves to 192.168.1.42 on all routers
  No service disruption
```

**Implementation note**: DHCP renewal (RENEW state) is more graceful than DISCOVER. If the device can RENEW with the same router, use that path. Falling back to DISCOVER (upon timeout) is the standard behavior.

### 5.3 Router Goes Offline and Rejoins with Stale State

**Scenario**: Router-A is part of the mesh, issues leases 192.168.1.100-200. It gets disconnected (network cable unplugged or hardware reboot). Meanwhile, Router-B and Router-C continue issuing leases.

```
T=0:
  All routers synced. CRDT has:
    /leases/192.168.1.100 → {mac=AA:AA:..., router_a, claimed_at=T0}
    /leases/192.168.1.101 → {mac=BB:BB:..., router_a, claimed_at=T0}
    ...

T=+100s:
  Router-A disconnects (unplugged from mesh, lost iroh connection)
  Router-B and Router-C continue operating independently
  Router-B issues lease: /leases/192.168.1.200 → {mac=XX:XX:..., router_b, claimed_at=T+100}
  Router-C issues lease: /leases/192.168.1.201 → {mac=YY:YY:..., router_c, claimed_at=T+100}

  Router-A is offline. Its CRDT snapshot is stale (missing 192.168.1.200, 192.168.1.201).

T=+200s:
  Router-A is plugged back in, reestablishes iroh connections
  mjolnir-mesh daemon starts
  Iroh endpoint bootstraps and finds peers (Router-B, Router-C)

  **CRDT merge**: iroh-docs automatically syncs
    - Router-A's local snapshot: {100, 101, ...}
    - Router-B and Router-C's state: {100, 101, ..., 200, 201}
    - Result: All three converge to {100, 101, ..., 200, 201}

T=+205s:
  Router-A's merge is complete
  All three routers have identical CRDT state
  No manual intervention needed

  Later, if Router-A tries to issue 192.168.1.200 again:
    - It checks CRDT, sees 192.168.1.200 is already leased (router_b, claimed_at=T+100)
    - Router-A marks it as in-use, skips it
    - Assigns a different IP to new requests

Result:
  No conflicts, no data loss
  System heals automatically via CRDT merge
```

**Key design principle**: CRDT merge is **idempotent and deterministic**. Merging the same data multiple times produces the same result. This is what makes the system resilient.

### 5.4 Lease Expires While Router Is Offline

**Scenario**: Device-A is leased to 192.168.1.42 with expiry=T+3600. Router-A claims it. Then Router-A goes offline. The lease expires at T+3600, but Router-A doesn't know.

```
T=0:
  Router-A issues lease: /leases/192.168.1.42 → {mac=AA:AA:..., expiry=T+3600, router_a}

T=+1000s:
  Router-A goes offline
  Router-A still has the lease in its CRDT snapshot

T=+3600s (lease expires):
  Router-A is still offline
  Router-B and Router-C notice the lease is expired (current_time > expiry)
  They stop serving DNS records for it
  They may also mark the IP as free in their own dnsmasq (optional)

T=+5000s:
  Router-A comes back online
  CRDT merges: Router-A sees the same lease { ..., expiry=T+3600 }
  Router-A checks: current_time (T+5000) > expiry (T+3600) → EXPIRED
  Router-A marks it as stale, doesn't serve it

Result:
  Expired lease is self-healing
  No conflicts
  IP is now free for reuse
```

**Design choice**: Don't delete expired leases from the CRDT. They eventually become stale and are ignored. Optional garbage collection for storage efficiency.

### 5.5 Mesh Split-Brain (Network Partition)

**Scenario**: A mesh of 5 routers is partitioned into two groups. Group A (3 routers) and Group B (2 routers) operate independently for some time, then merge.

```
T=0:
  5 routers in mesh, all synced
  CRDT: 100 leases (distributed)

T=+100s (network partition):
  Link between Group A and Group B goes down
  Router-A, Router-B, Router-C (Group A) can communicate with each other
  Router-D, Router-E (Group B) can communicate with each other
  No cross-group communication

T=+100-500s (partition active):
  Group A: New device joins Router-A, gets lease 192.168.1.150
    Router-A writes to its CRDT: { ip, router_a, claimed_at=T+200 }
    Group A routers sync: B, C also see the lease

  Group B: New device joins Router-D, gets lease 192.168.1.150
    Router-D writes to its CRDT: { ip, router_d, claimed_at=T+250 }
    Group B routers sync: E also sees the lease

  Both groups now have conflicting entries for 192.168.1.150
  (Different MAC addresses, different routers)

T=+500s (network heals):
  Link between Group A and Group B is restored
  Router-B (Group A) reconnects to Router-D (Group B)
  Iroh nodes establish connections

  **CRDT merge initiates**:
  Router-B receives Router-D's state (100 leases + new 192.168.1.150 from router_d)
  Router-B merges:
    - Entry A: { ip=192.168.1.150, router_a, claimed_at=T+200, mac=AA:AA:... }
    - Entry D: { ip=192.168.1.150, router_d, claimed_at=T+250, mac=DD:DD:... }
    - Merge: claimed_at(D) > claimed_at(A) → Router-D's entry wins
    - Router-D keeps IP, Router-A loses it

T=+510s:
  Group A and Group B have merged CRDT state
  Router-A learns it lost 192.168.1.150 to Router-D
  Router-A marks it as taken, stops serving it
  If Device-A is still connected to Router-A, Router-A issues a DHCP NACK (lease expired)
  Device-A must request a new IP

Result:
  Split-brain is resolved via LWW
  One device loses its IP (conflict resolved deterministically)
  Both devices can coexist on the same network (after conflict resolution)
  System reaches convergence automatically
```

**Trade-off**: LWW is simple but not always "fair." The newer timestamp wins, regardless of which partition is larger or more authoritative. For a mesh of trusted peers (all routers under one operator), this is acceptable. For untrusted peers, add Byzantine fault tolerance (signatures, voting, etc.).

---

## 6. dnsmasq Integration

### 6.1 Input Path: Watch and Ingest

The daemon watches dnsmasq's lease file and injects updates into the CRDT.

```
┌─────────────────────────────────────┐
│ dnsmasq                             │
│ DHCP server, running on each router │
└──────────────┬──────────────────────┘
               │ writes
               v
        /tmp/dhcp.leases
        (text file, one lease per line)
               │
               │ inotify:
               │ IN_MODIFY, IN_ATTRIB
               v
┌─────────────────────────────────────┐
│ mjolnir-mesh DnsmasqWatcher         │
│ Reads/parses /tmp/dhcp.leases       │
│ Detects new/updated/removed leases  │
└──────────┬──────────────────────────┘
           │
           │ creates LeaseEntry
           │ serializes to postcard
           │
           ├─→ iroh-gossip topic "dhcp-leases" [hot path, ~10-100ms latency]
           │
           └─→ iroh-docs key "/leases/{ip}" [cold path, durable]
```

**File format** (/tmp/dhcp.leases):

```
dnsmasq writes one lease per line:
{timestamp} {mac-address} {ip-address} {hostname} {client-id}

Example:
1711276800 aa:bb:cc:dd:ee:ff 192.168.1.100 laptop-alice *
1711276805 11:22:33:44:55:66 192.168.1.101 printer-office *
```

**Parsing logic**:

```rust
/// Parse a single line from /tmp/dhcp.leases
pub fn parse_dnsmasq_lease_line(line: &str) -> Result<(String, LeaseEntry)> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return Err(anyhow::anyhow!("invalid lease line format"));
    }

    let timestamp = parts[0].parse::<u64>()?;
    let mac = parts[1].to_string();
    let ip = parts[2].to_string();
    let hostname = parts[3].to_string();

    // Compute expiry: timestamp is the expiration, but dnsmasq stores as unix epoch
    // (Some versions store as relative duration; see dnsmasq docs)
    let expiry = timestamp; // Assuming epoch format

    // Extract duration from lease: typically set by dnsmasq config (e.g., 3600s)
    // For now, estimate from relative time: expiry - now
    let duration_secs = if expiry > now() {
        (expiry - now()) as u32
    } else {
        3600 // default fallback
    };

    let router_id = get_local_router_id(); // Iroh NodeId of this router
    let claimed_at = now_micros() as u64 / 1_000_000; // current unix timestamp

    Ok((
        ip.clone(),
        LeaseEntry {
            mac,
            hostname,
            expiry,
            router_id,
            claimed_at,
            duration_secs,
        },
    ))
}
```

**Watcher implementation**:

```rust
use notify::{Watcher, RecursiveMode, watcher};
use std::sync::mpsc::channel;
use std::time::Duration;

pub struct DnsmasqWatcher {
    path: PathBuf,
    watcher: RecommendedWatcher,
}

impl DnsmasqWatcher {
    pub fn new(path: &str) -> Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = watcher(tx, Duration::from_millis(100))?;
        watcher.watch(path, RecursiveMode::NonRecursive)?;

        Ok(Self {
            path: path.into(),
            watcher,
        })
    }

    pub async fn watch_and_sync<S: LeaseStore>(
        &self,
        store: &S,
    ) -> Result<()> {
        loop {
            // Wait for file modification event
            // Read entire file, parse all leases
            let current_leases = self.read_leases()?;

            // Compare with previous state (delta detection)
            let delta = self.compute_delta(&current_leases)?;

            // Sync new/updated leases to CRDT
            for (ip, entry) in delta.added_or_updated {
                store.claim_lease(&ip, &entry).await?;
            }

            // Optional: release removed leases
            for ip in delta.removed {
                store.release_lease(&ip).await?;
            }
        }
    }

    fn read_leases(&self) -> Result<HashMap<String, LeaseEntry>> {
        let contents = std::fs::read_to_string(&self.path)?;
        let mut leases = HashMap::new();

        for line in contents.lines() {
            let (ip, entry) = parse_dnsmasq_lease_line(line)?;
            leases.insert(ip, entry);
        }

        Ok(leases)
    }
}
```

### 6.2 Output Path: Write Merged State

The daemon maintains `/tmp/hosts/mesh` with the merged DNS records from all routers.

```
┌──────────────────────────────────────────┐
│ iroh-docs CRDT                           │
│ All routers' leases and DNS records      │
└──────────────┬───────────────────────────┘
               │ merge + filter
               │ (valid, non-expired entries)
               v
       LeaseIndex
       {ip → LeaseEntry, ...}
               │
               │ convert to hosts format
               v
        /tmp/hosts/mesh
        (hosts file format)
               │
               │ inotify:
               │ IN_CLOSE_WRITE
               │ (or SIGHUP signal)
               v
┌──────────────────────────────────────────┐
│ dnsmasq                                  │
│ addn-hosts=/tmp/hosts/mesh               │
│ Reloads on file change or signal         │
└──────────────────────────────────────────┘
               │
               │ serves DNS queries
               v
        192.168.1.42 → laptop-alice
        192.168.1.43 → printer-office
```

**Hosts file format** (/tmp/hosts/mesh):

```
# Generated by mjolnir-mesh lease sync daemon
# Generated: 2026-03-25 10:15:30 UTC
# Do not edit manually; changes will be overwritten on next sync

192.168.1.42  laptop-alice laptop-alice.local
192.168.1.43  printer-office printer-office.local
192.168.1.50  camera-kitchen camera-kitchen.local
192.168.1.100 device-100 device-100.local
```

**Writing logic**:

```rust
pub async fn write_hosts_file<S: LeaseStore>(
    store: &S,
    output_path: &str,
    domain_suffix: &str, // default: "local"
) -> Result<()> {
    // Read all leases from store
    let leases = store.get_all_leases().await?;

    // Filter: only include non-expired, non-empty hostnames
    let mut entries = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();

    for (ip, lease) in leases {
        if lease.expiry > now && !lease.hostname.is_empty() {
            // Sanitize hostname: remove spaces, special chars, lowercase
            let safe_hostname = sanitize_hostname(&lease.hostname);
            entries.push((ip.clone(), safe_hostname.clone()));

            // Also add .local CNAME
            entries.push((ip.clone(), format!("{}.{}", safe_hostname, domain_suffix)));
        }
    }

    // Sort for deterministic output (helps with diffs)
    entries.sort();

    // Generate file content
    let mut content = String::new();
    content.push_str("# Generated by mjolnir-mesh\n");
    content.push_str(&format!("# Generated: {}\n", chrono::Utc::now()));
    content.push_str("# Do not edit manually\n\n");

    for (ip, hostname) in entries {
        content.push_str(&format!("{}\t{}\n", ip, hostname));
    }

    // Write atomically (write to temp file, then move)
    let temp_path = format!("{}.tmp", output_path);
    tokio::fs::write(&temp_path, &content).await?;
    tokio::fs::rename(&temp_path, output_path).await?;

    // Signal dnsmasq to reload (optional, or rely on inotify)
    // std::process::Command::new("killall").arg("-HUP").arg("dnsmasq").output()?;

    Ok(())
}

fn sanitize_hostname(hostname: &str) -> String {
    hostname
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '-', // replace invalid chars
        })
        .collect::<String>()
        .to_lowercase()
}
```

### 6.3 Race Conditions and Mitigation

**Race 1: dnsmasq writes lease while daemon is reading**

```
Daemon reads /tmp/dhcp.leases:
  Sees leases: [A, B, C]

Meanwhile, dnsmasq appends lease D:
  [A, B, C]
  [A, B, C, D]  ← appended

Daemon's read captured only [A, B, C]
Lease D is missed in this cycle

Mitigation:
  - Set inotify to IN_CLOSE_WRITE, not IN_MODIFY
  - Wait for file to close before reading (ensures atomic write from dnsmasq)
  - Re-read file after each inotify event
  - Use flock() if dnsmasq supports it
```

**Race 2: Daemon writes /tmp/hosts/mesh while dnsmasq is reading**

```
Daemon writes /tmp/hosts/mesh:
  Creates /tmp/hosts/mesh.tmp
  Writes content (partial write)

Meanwhile, dnsmasq reads /tmp/hosts/mesh:
  Old file is being read

Daemon finishes:
  Moves /tmp/hosts/mesh.tmp → /tmp/hosts/mesh

dnsmasq is unaware (inotify tells it to reload)

Mitigation:
  - Atomic rename (on POSIX, rename is atomic)
  - Write to temp file, rename in one syscall
  - dnsmasq's inotify triggers reload (IN_CLOSE_WRITE on /tmp/hosts/mesh)
```

**Race 3: Multiple daemons writing /tmp/hosts/mesh**

In a redundant setup, multiple instances of mjolnir-mesh might run. Both write to /tmp/hosts/mesh.

```
Mitigation:
  - Use file locking (flock)
  - Or, use a single daemon instance with a leader-election mechanism (Iroh gossip)
  - Or, designate one router as the DNS authority
```

For MVP, assume a single mjolnir-mesh instance per router (reasonable for embedded routers).

### 6.4 Reload Behavior

After updating /tmp/hosts/mesh, dnsmasq must reload:

**Option A: Signal-based**
```bash
killall -HUP dnsmasq
```
Dnsmasq reloads /etc/dnsmasq.conf and addn-hosts files.

**Option B: inotify-based**
Dnsmasq watches /tmp/hosts/mesh for changes and reloads automatically.

**Option C: Polling**
Dnsmasq polls /tmp/hosts/mesh periodically (slow, not recommended).

For modern dnsmasq (>2.80), Option A is reliable. Option B requires dnsmasq support for addn-hosts watching.

---

## 7. Gossip Protocol Details

### 7.1 Message Format

Gossip messages broadcast lease updates with minimal latency. The message is a serialized **LeaseAction** enum.

```rust
/// Action to broadcast via iroh-gossip topic "dhcp-leases"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LeaseAction {
    /// Claim a new lease or update an existing one
    Claim {
        ip: String,
        mac: String,
        hostname: String,
        expiry: u64,
        router_id: String,
        claimed_at: u64,
        duration_secs: u32,
    },

    /// Release a lease (optional, for explicit cleanup)
    Release {
        ip: String,
        router_id: String,
        released_at: u64,
    },

    /// Heartbeat/keepalive (optional, for liveness detection)
    Heartbeat {
        router_id: String,
        timestamp: u64,
    },
}

/// Serialized gossip message payload (postcard binary format)
type GossipMessage = Vec<u8>; // postcard::to_allocvec(&action)?
```

**Serialized size**:
- LeaseAction::Claim: ~120-150 bytes
- LeaseAction::Release: ~80 bytes
- Topic broadcast: sub-1KB total per message

### 7.2 Topic Naming

Gossip is organized by topic (broadcast channel). For DHCP:

```
Topic: "dhcp-leases"
  Purpose: Broadcast DHCP lease claims, updates, releases
  Subscribers: All routers in the mesh
  Delivery: Best-effort, best-effort ordering
  Latency: ~10-100ms fanout

Topic: "dhcp-sync-request" (optional)
  Purpose: Explicitly request a sync (e.g., after partition rejoin)
  Subscribers: All routers
  Payload: { router_id, timestamp }

Topic: "mesh-control" (optional, for future extensions)
  Purpose: Network control messages (topology, metrics, etc.)
  Subscribers: All routers
```

### 7.3 Serialization and Codec

**Encoding**: Postcard binary format (compact, deterministic).

```rust
pub struct GossipCodec;

impl GossipCodec {
    pub fn encode_action(action: &LeaseAction) -> Result<Vec<u8>> {
        postcard::to_allocvec(action)
            .map_err(|e| anyhow::anyhow!("postcard encode error: {}", e))
    }

    pub fn decode_action(bytes: &[u8]) -> Result<LeaseAction> {
        postcard::from_bytes(bytes)
            .map_err(|e| anyhow::anyhow!("postcard decode error: {}", e))
    }
}

// Usage:
let action = LeaseAction::Claim { ... };
let encoded = GossipCodec::encode_action(&action)?;
gossip_topic.broadcast(&encoded).await?;

// On receive:
let bytes = gossip_message.content;
let action = GossipCodec::decode_action(&bytes)?;
```

### 7.4 Message Loss and Idempotency

Gossip is best-effort; messages may be lost. The CRDT (iroh-docs) provides durability for missed messages.

```
Scenario: Router-C misses Router-A's gossip message about lease L1.

T=0:
  Router-A broadcasts: LeaseAction::Claim { ip=192.168.1.42, ... }

T=+50ms:
  Router-B receives the message ✓
  Router-C is in a black hole, misses it ✗

T=+100ms:
  Router-A writes L1 to iroh-docs
  Iroh syncs L1 to Router-B and Router-C

T=+200ms:
  Router-C's iroh-docs sync catches up
  Router-C gets L1 from Router-A's author entry in the doc
  Result: L1 is known to all routers, despite gossip loss

Recovery: Automatic via CRDT.
```

**Idempotency**: Receiving the same LeaseAction twice has the same effect as receiving it once.

```rust
// When Router-B receives a gossip message:
pub async fn process_lease_action(
    action: &LeaseAction,
    store: &LeaseStore,
) -> Result<()> {
    match action {
        LeaseAction::Claim { ip, mac, hostname, expiry, router_id, claimed_at, duration_secs } => {
            // Read current state
            let current = store.get_lease(ip).await?;

            // Check if this action is newer
            if let Some(current_lease) = current {
                if claimed_at <= current_lease.claimed_at {
                    // Stale message, ignore
                    return Ok(());
                }
            }

            // Write new lease (or update if exists)
            let entry = LeaseEntry { ip, mac, hostname, expiry, router_id, claimed_at, duration_secs };
            store.set_lease(ip, &entry).await?;
            Ok(())
        }
        // ... other variants
    }
}
```

This logic is **idempotent**: processing the same LeaseAction multiple times produces the same result.

### 7.5 Gossip Topology

Gossip uses epidemic broadcast (flood-fill) to reach all peers. The topology is implied by Iroh's peer connections.

```
Router-A <--QUIC--> Router-B
            |
            |
            <--QUIC--> Router-C

Router-A broadcasts LeaseAction:
  Router-A → Router-B, Router-C (direct)
  Router-B → Router-A (echo suppression), Router-C (if not direct)
  Router-C → Router-A, Router-B (if not direct)

Latency: ~10-100ms depending on topology (direct vs. relay).
```

Iroh's gossip module handles topology automatically; the daemon just publishes to the topic.

---

## 8. Proposed Rust Types

### 8.1 Core Data Structures

```rust
use serde::{Deserialize, Serialize};

/// A single DHCP lease in the mesh state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseEntry {
    pub mac: String,
    pub hostname: String,
    pub expiry: u64,
    pub router_id: String,
    pub claimed_at: u64,
    pub duration_secs: u32,
}

impl LeaseEntry {
    /// Check if this lease has expired.
    pub fn is_expired(&self, now: u64) -> bool {
        now > self.expiry
    }

    /// Check if this lease matches a given MAC address.
    pub fn matches_mac(&self, mac: &str) -> bool {
        self.mac.eq_ignore_ascii_case(mac)
    }
}

/// A DNS record in the mesh state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsEntry {
    pub ip: String,
    pub record_type: String,
    pub ttl: u32,
    pub router_id: String,
    pub created_at: u64,
}

/// Action to broadcast via iroh-gossip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LeaseAction {
    Claim {
        ip: String,
        mac: String,
        hostname: String,
        expiry: u64,
        router_id: String,
        claimed_at: u64,
        duration_secs: u32,
    },

    Release {
        ip: String,
        router_id: String,
        released_at: u64,
    },

    Heartbeat {
        router_id: String,
        timestamp: u64,
    },
}
```

### 8.2 Store Traits

```rust
use async_trait::async_trait;
use anyhow::Result;
use std::collections::HashMap;

/// Abstraction for the lease storage backend (iroh-docs or in-memory for tests).
#[async_trait]
pub trait LeaseStore: Send + Sync {
    /// Get a single lease by IP.
    async fn get_lease(&self, ip: &str) -> Result<Option<LeaseEntry>>;

    /// Set or update a lease.
    async fn set_lease(&self, ip: &str, entry: &LeaseEntry) -> Result<()>;

    /// Remove a lease (optional; expiry is preferred).
    async fn delete_lease(&self, ip: &str) -> Result<()>;

    /// Get all non-expired leases.
    async fn get_all_leases(&self) -> Result<HashMap<String, LeaseEntry>>;

    /// Get all leases by MAC address (for roaming detection).
    async fn get_leases_by_mac(&self, mac: &str) -> Result<Vec<LeaseEntry>>;

    /// Perform CRDT merge and return merged leases.
    async fn merge_and_get_all(&self) -> Result<HashMap<String, LeaseEntry>>;
}

/// Abstraction for DNS store.
#[async_trait]
pub trait DnsStore: Send + Sync {
    async fn get_dns_entry(&self, hostname: &str) -> Result<Option<DnsEntry>>;
    async fn set_dns_entry(&self, hostname: &str, entry: &DnsEntry) -> Result<()>;
    async fn delete_dns_entry(&self, hostname: &str) -> Result<()>;
    async fn get_all_dns_entries(&self) -> Result<HashMap<String, DnsEntry>>;
}
```

### 8.3 Synchronization Components

```rust
/// Watches dnsmasq's lease file and syncs changes to the CRDT.
pub struct DnsmasqWatcher {
    path: std::path::PathBuf,
    store: Arc<dyn LeaseStore>,
}

impl DnsmasqWatcher {
    pub async fn run(&self) -> Result<()> {
        // inotify loop
        // Read, parse, delta, sync
    }
}

/// Writes merged CRDT state to dnsmasq's hosts file.
pub struct HostsFileWriter {
    output_path: std::path::PathBuf,
    store: Arc<dyn LeaseStore>,
}

impl HostsFileWriter {
    pub async fn run(&self) -> Result<()> {
        // Periodically read all leases, filter, write /tmp/hosts/mesh
    }
}

/// Broadcasts lease actions via iroh-gossip.
pub struct GossipBroadcaster {
    topic: String, // "dhcp-leases"
    router_id: String,
}

impl GossipBroadcaster {
    pub async fn broadcast_claim(&self, entry: &LeaseEntry) -> Result<()> {
        let action = LeaseAction::Claim { /* ... */ };
        let encoded = postcard::to_allocvec(&action)?;
        // self.gossip_topic.broadcast(&encoded).await?
    }
}

/// Receives and processes gossip messages.
pub struct GossipSubscriber {
    topic: String,
    store: Arc<dyn LeaseStore>,
}

impl GossipSubscriber {
    pub async fn run(&self) -> Result<()> {
        // loop { receive message, decode, process }
    }
}
```

### 8.4 Merge Function

```rust
/// Merge multiple author versions of a lease to get the canonical entry.
pub fn merge_lease_versions(
    versions: Vec<(String, LeaseEntry)>, // (router_id, entry)
) -> Option<LeaseEntry> {
    versions
        .into_iter()
        .max_by(|(id_a, entry_a), (id_b, entry_b)| {
            match entry_a.claimed_at.cmp(&entry_b.claimed_at) {
                std::cmp::Ordering::Equal => id_a.cmp(id_b), // tie-breaker
                other => other,
            }
        })
        .map(|(_, entry)| entry)
}

/// Merge multiple author versions of a DNS entry.
pub fn merge_dns_versions(
    versions: Vec<(String, DnsEntry)>,
) -> Option<DnsEntry> {
    versions
        .into_iter()
        .max_by(|(id_a, entry_a), (id_b, entry_b)| {
            match entry_a.created_at.cmp(&entry_b.created_at) {
                std::cmp::Ordering::Equal => id_a.cmp(id_b),
                other => other,
            }
        })
        .map(|(_, entry)| entry)
}
```

---

## 9. Performance Characteristics

### 9.1 Latency

| Operation | Latency | Bottleneck |
|-----------|---------|-----------|
| DHCP request → lease claim | 10-20ms | dnsmasq DHCP processing |
| Lease write to dnsmasq file | 5-15ms | file I/O |
| inotify wake | <1ms | kernel event |
| Postcard serialization | <1ms | CPU |
| Gossip broadcast | 10-100ms | network fanout, Iroh gossip |
| iroh-docs write (persistent) | 5-20ms | disk I/O + replication |
| Peer ingests message | 20-100ms | network + local processing |
| dnsmasq reload (/tmp/hosts/mesh) | 50-500ms | dnsmasq config reload |
| **Total E2E propagation** | **~100-300ms** | **dnsmasq reload** |

**Optimization**: Signal dnsmasq to reload after writing /tmp/hosts/mesh:
```bash
killall -HUP dnsmasq
```
Reduces reload latency from 500ms (polling) to ~50-100ms.

### 9.2 Memory Usage

Per router:

- **Iroh endpoint**: ~5-10 MB (QUIC state, connections)
- **iroh-docs store**: ~1 MB per 100 leases (postcard is compact)
  - Assume 100-500 devices in mesh → 1-5 MB
- **iroh-gossip subscriptions**: <1 MB (topics, buffers)
- **Local caches**: ~1 MB (HashMap of active leases)
- **Total baseline**: ~10-20 MB

For a mesh with 500 devices across 5 routers:
- 100 leases per router
- ~100 bytes postcard per entry
- ~10 KB per router's lease table
- Total: ~50 KB for entire mesh CRDT state
- Safe margin for metadata and overhead: ~5-10 MB per router

**Scalability**: Linear in number of leases (O(n)). 1000 leases = ~20 MB, acceptable for a router with >256 MB RAM.

### 9.3 Disk I/O

iroh-docs persists entries to a local append-only log. Each write:

- **Postcard serialization**: <1 ms
- **Append to log**: ~5-20 ms (SSD)
- **Fsync (durability)**: ~10-50 ms (HDD)

**Optimization**: Batch writes or use async writes (eventual durability).

### 9.4 Gossip Bandwidth

Per lease claim:

- **Message size**: ~120 bytes (postcard + metadata)
- **Fanout**: Reaches all N peers in O(log N) hops
- **Bandwidth per claim**: 120 bytes * log(N) * peers
- **Example**: 10 routers, 100 leases total, ~2 claims/minute = ~40 bytes/sec per router (negligible)

**Peak**: High churn (devices joining/leaving frequently) → more messages. Design for 1 claim/device/hour for steady-state estimate.

### 9.5 Doc Store Growth

iroh-docs is append-only. Each version of an entry takes storage.

- **Worst case**: 10 routers, each updates the same IP 100 times
- **Versions stored**: 10 * 100 = 1000 versions for one IP
- **Storage**: 1000 * 100 bytes = ~100 KB per IP (stale)
- **Mitigation**: Garbage collection (compact old versions) every few days

For MVP, accept append-only growth; add GC in Phase 2.

---

## 10. Edge Cases and Mitigations

### 10.1 Clock Skew Between Routers

Routers may have out-of-sync system clocks (no NTP). The `claimed_at` timestamp might not reflect true causality.

```
Router-A claims IP at local time 1000 (clock is 10s behind)
Router-B claims same IP at local time 1050 (clock is correct)

Merge logic uses claimed_at: 1050 > 1000 → Router-B wins
But in reality, Router-A claimed it first (globally).

Result: Wrong router wins (not a correctness bug, just suboptimal).
```

**Mitigation: Hybrid Logical Clock (HLC)**

Use HLC instead of system timestamp:
```rust
pub struct HLC {
    physical_time: u64, // system clock (seconds)
    logical_counter: u32, // monotonic counter
}

// When writing: increment logical_counter if physical_time hasn't advanced
// Ensures causal ordering even with clock skew
```

**For MVP**: Accept clock skew; add HLC in Phase 2. Single-router ownership (no race) is common.

### 10.2 Rapid Lease Churn

Devices join/leave frequently. Each change creates gossip messages and CRDT entries.

```
Scenario: 100 devices powering on/off every minute
  100 devices * 2 events/minute = 200 messages/minute = ~3 per second

Per router bandwidth: 3 * 120 bytes = 360 bytes/sec (acceptable)
```

**Scaling**: 1000 devices → 3600 bytes/sec per router (still acceptable, <0.5% of typical router uplink).

### 10.3 Gossip Storm

If a router malfunctions and broadcasts the same message repeatedly:

```
Malfunction: Router-X keeps broadcasting
  "Claim IP 192.168.1.42 as router_x"

Result: All other routers see duplicate messages
         They process idempotently (no side effect)
         No harm, but wastes bandwidth
```

**Mitigation**: Message deduplication (track message IDs via bloom filter or set), or rely on idempotency (no harm if processed twice).

### 10.4 Doc Store Compaction

The append-only log grows unbounded. Eventually, storage is exhausted.

```
Strategy 1: Periodic GC
  Every day: scan all entries, keep only latest per key
  Remove stale versions
  Rewrite log (downtime: ~100-500ms)

Strategy 2: Snapshot + delta
  Iroh may support this natively in later versions
```

**For MVP**: Monitor doc store size; implement GC if >100 MB.

### 10.5 Stale DNS Entries After Expiry

A lease expires and is no longer valid, but its DNS entry persists in /tmp/hosts/mesh.

```
Scenario: Device leaves without DHCP RELEASE
  Lease expires (expiry < now)
  DNS entry: 192.168.1.42 → device-name still in hosts file
  New device gets 192.168.1.42
  DNS returns old device name for new device

Result: DNS pollution (incorrect resolution)
```

**Mitigation**: Write only non-expired entries to /tmp/hosts/mesh (already in write_hosts_file code above).

### 10.6 Network Partition + Collision + Healing

Most complex scenario: mesh splits, both partitions assign the same IP to different devices, then heal.

(See § 5.5 for detailed walkthrough. Result: LWW resolves deterministically, one device loses IP.)

**Mitigation**: Acceptance + DHCP retry handling. This is inherent to distributed systems without strong consensus.

---

## 11. Implementation Phases

### Phase 1: Core CRDT (Weeks 1-3)

**Deliverable**: Single router pair syncs leases via iroh-docs + gossip.

**Scope**:
- LeaseEntry, DnsEntry types with postcard serialization
- LeaseStore trait + in-memory implementation (for unit tests)
- LeaseStore trait + iroh-docs-backed implementation
- DnsmasqWatcher (inotify on /tmp/dhcp.leases)
- GossipBroadcaster (publish Claim/Release actions)
- GossipSubscriber (receive and process)
- HostsFileWriter (merge + write /tmp/hosts/mesh)
- Unit tests for merge logic, parsing, serialization
- Integration test: two routers exchange leases

**Deliverables**:
- `crates/mjolnir-dhcp/` crate with core types and traits
- `crates/mjolnir-daemon/` binary (orchestrates components)
- README with single-router setup

### Phase 2: Multi-Router Mesh (Weeks 4-5)

**Deliverable**: 3+ routers, automatic discovery + sync.

**Scope**:
- Iroh endpoint bootstrapping (n0 discovery or static seeds)
- Mesh join/leave (subscription updates)
- Expiry handling (stale lease cleanup)
- Integration test: 3 VMs, DHCP churn, verify consistency
- Basic metrics (messages sent, leases replicated, latency)

### Phase 3: Roaming + Partition Tolerance (Weeks 6-8)

**Deliverable**: Device roams, partitions heal automatically.

**Scope**:
- Roaming detection (MAC reuse)
- Partition test (split mesh, rejoin, verify merge)
- Clock skew resilience (HLC if needed)
- Stability test: 3 routers + 10 devices + 24h runtime

### Phase 4: OpenWrt Deployment (Weeks 9-10)

**Deliverable**: Static binary runs on actual routers.

**Scope**:
- Cross-compile for ARM/MIPS
- UCI/systemd integration
- dnsmasq config patching
- Field testing on GL.iNet hardware

---

## 12. Testing Strategy

### 12.1 Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lease_entry_serialization() {
        let entry = LeaseEntry {
            mac: "aa:bb:cc:dd:ee:ff".into(),
            hostname: "device".into(),
            expiry: 1711276800,
            router_id: "router_a".into(),
            claimed_at: 1711270000,
            duration_secs: 3600,
        };

        let encoded = postcard::to_allocvec(&entry).unwrap();
        let decoded: LeaseEntry = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(entry, decoded);
    }

    #[test]
    fn test_merge_lease_versions_lww() {
        let entry_a = LeaseEntry {
            claimed_at: 1000,
            ..Default::default()
        };
        let entry_b = LeaseEntry {
            claimed_at: 2000,
            ..Default::default()
        };

        let result = merge_lease_versions(vec![
            ("router_a".into(), entry_a),
            ("router_b".into(), entry_b),
        ]);

        assert_eq!(result.unwrap().claimed_at, 2000);
    }

    #[test]
    fn test_merge_lease_tie_breaker() {
        let entry_a = LeaseEntry {
            claimed_at: 1000,
            ..Default::default()
        };
        let entry_b = LeaseEntry {
            claimed_at: 1000, // same timestamp
            ..Default::default()
        };

        let result = merge_lease_versions(vec![
            ("router_z".into(), entry_a),
            ("router_a".into(), entry_b),
        ]);

        // router_a < router_z (lexicographic)
        assert_eq!(result.unwrap().router_id, "router_a");
    }

    #[test]
    fn test_dnsmasq_lease_parsing() {
        let line = "1711276800 aa:bb:cc:dd:ee:ff 192.168.1.42 device-name *";
        let (ip, entry) = parse_dnsmasq_lease_line(line).unwrap();

        assert_eq!(ip, "192.168.1.42");
        assert_eq!(entry.mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(entry.hostname, "device-name");
    }

    #[test]
    fn test_hosts_file_write_filters_expired() {
        let now = 1711276800u64;
        let mut leases = HashMap::new();

        // Add valid lease
        leases.insert(
            "192.168.1.100".into(),
            LeaseEntry {
                hostname: "valid".into(),
                expiry: now + 1000,
                ..Default::default()
            },
        );

        // Add expired lease
        leases.insert(
            "192.168.1.101".into(),
            LeaseEntry {
                hostname: "expired".into(),
                expiry: now - 1000,
                ..Default::default()
            },
        );

        // Only "valid" should appear in hosts file
    }
}
```

### 12.2 Integration Tests

- **Two-router lease sync**: Router-A issues lease, Router-B resolves hostname
- **Multi-router gossip**: Broadcast is received by all peers
- **Partition recovery**: Split mesh, merge, verify no data loss
- **Roaming**: Device moves between routers, keeps same IP

### 12.3 Stability and Performance Tests

- **3+ routers, 24h runtime**: Monitor memory, CPU, lease consistency
- **Churn test**: 100 devices joining/leaving randomly
- **Gossip latency**: Measure claim broadcast → reception on all peers
- **Doc store growth**: Track disk usage over time

---

## 13. References

**Iroh**:
- Iroh 0.97: [github.com/n0-computer/iroh](https://github.com/n0-computer/iroh)
- iroh-gossip: Topic-based broadcast
- iroh-docs: CRDT key-value store (in development)

**CRDT Theory**:
- "A comprehensive study of CRDT" (Shapiro et al., 2011)
- Last-writer-wins register: Simple, deterministic, suitable for unstable networks

**DHCP**:
- RFC 2131: Dynamic Host Configuration Protocol
- dnsmasq: [thekelleys.org.uk/dnsmasq/doc.html](http://thekelleys.org.uk/dnsmasq/doc.html)

**Serialization**:
- Postcard: [docs.rs/postcard](https://docs.rs/postcard)
- Serde: [serde.rs](https://serde.rs)

**Project Files**:
- `crates/mjolnir-node/src/mesh.rs`: Iroh endpoint setup
- `Cargo.toml`: Workspace dependencies (iroh, postcard, tokio)

---

## 14. Appendix: Example Trace

A detailed example of a device joining the mesh:

```
Time T=1711276800 (2026-03-25 10:00:00 UTC)

1. Device "laptop-alice" (MAC aa:bb:cc:dd:ee:ff) boots on Router-A's network

2. Device broadcasts DHCP DISCOVER

3. Router-A's dnsmasq (dnsmasq -C /etc/dnsmasq.conf)
   - Receives DISCOVER
   - Checks its pool: 192.168.1.100-254 (free: 192.168.1.100)
   - Sends DHCP OFFER: 192.168.1.100, lease time 3600s
   - Writes to /tmp/dhcp.leases:
     "1711280400 aa:bb:cc:dd:ee:ff 192.168.1.100 laptop-alice *"

4. inotify event on /tmp/dhcp.leases (IN_CLOSE_WRITE)

5. mjolnir-mesh DnsmasqWatcher wakes:
   - Reads /tmp/dhcp.leases
   - Parses new entry
   - Creates LeaseEntry:
     {
       mac: "aa:bb:cc:dd:ee:ff",
       hostname: "laptop-alice",
       expiry: 1711280400,
       router_id: "router_a_node_id_base32",
       claimed_at: 1711276810,
       duration_secs: 3600,
     }

6. Broadcasts via iroh-gossip (hot path):
   - Serializes LeaseEntry to postcard (~130 bytes)
   - Publishes to topic "dhcp-leases"
   - All connected peers (Router-B, Router-C) receive in ~20-100ms

7. Writes to iroh-docs (cold path):
   - Key: "/leases/192.168.1.100"
   - Value: postcard(LeaseEntry)
   - Durably persisted, synced to peers

8. Router-B and Router-C receive gossip:
   - Decode LeaseEntry
   - Check CRDT for existing entry (none, first claim)
   - Write to local cache: 192.168.1.100 → {aa:bb:..., laptop-alice, router_a, ...}

9. HostsFileWriter (runs every 5-10 seconds):
   - Reads all leases from CRDT
   - Filters non-expired: 192.168.1.100 is valid
   - Generates /tmp/hosts/mesh:
     ```
     192.168.1.100  laptop-alice  laptop-alice.local
     ```
   - Writes atomically via temp file + rename
   - Signals dnsmasq: killall -HUP dnsmasq

10. dnsmasq reloads (IN_CLOSE_WRITE on /tmp/hosts/mesh):
    - Reads new hosts file
    - Updates DNS cache

11. Device receives DHCP ACK:
    - Configures IP 192.168.1.100/24
    - Sends gratuitous ARP

12. Another device on Router-B queries DNS:
    - Query: "laptop-alice.local"
    - Router-B's dnsmasq looks up /tmp/hosts/mesh
    - Returns 192.168.1.100
    - Device can connect to laptop-alice on Router-A's network

13. After 3600 seconds:
    - Lease expires (T=1711280400)
    - Device should RENEW before this
    - If device is still present:
      - Sends DHCP RENEW to Router-A
      - Router-A extends lease, updates /tmp/dhcp.leases
      - Cycle repeats from step 4
    - If device is gone:
      - Lease becomes stale (expiry < now)
      - HostsFileWriter filters it out
      - DNS entry disappears

Result: Seamless shared DHCP + DNS across mesh routers.
```

---

**Document Status**: Complete specification | Ready for implementation planning | Phase 1 development can commence