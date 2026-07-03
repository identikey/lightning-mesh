> **ARCHIVED 2026-07-02 — PARTIALLY IMPLEMENTED / SUPERSEDED.** The subnet-claim CRDT
> (`/subnets/{cidr}`, HLC first-writer-wins) shipped as designed and lives in
> `crates/mjolnir-mesh/src/crdt/`. The lease/DHCP/deauth machinery in this document
> (CRDT hostsfile, lease lifecycle, conflict deauth, Merkle anti-entropy) was never
> built — shipped anti-entropy is a full-claim-map rebroadcast every 20s. Kept as
> design reference for the service-mesh phase (bead `e21`).

# DHCP CRDT Architecture: Distributed Lease Synchronization

**Status:** Architecture specification | **Date:** 2026-03-26 | **Author:** mjolnir-mesh team

This document specifies the design and implementation of the DHCP CRDT layer for mjolnir-mesh. The CRDT is the coordination layer that lets every router in the mesh assign DHCP leases independently while maintaining a consistent, mesh-wide view of device reservations, DNS names, services, and routes — with no central authority.

See [mesh-network-coordination.md](mesh-network-coordination.md) for the high-level mesh vision.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Data Model](#2-data-model)
3. [CRDT Semantics](#3-crdt-semantics)
4. [Lease Lifecycle](#4-lease-lifecycle)
5. [Conflict Resolution](#5-conflict-resolution)
6. [Roaming](#6-roaming)
7. [dnsmasq Integration](#7-dnsmasq-integration)
8. [Gossip Protocol](#8-gossip-protocol)
9. [Anti-Entropy Sync](#9-anti-entropy-sync)
10. [Rust Types](#10-rust-types)
11. [Performance](#11-performance)
12. [Edge Cases](#12-edge-cases)
13. [References](#13-references)

---

## 1. Overview

Each router in the mesh runs its own instance of dnsmasq serving the same subnet and the same DHCP range. There is no leader, no pool partitioning, and no proposal window. Every router can assign any IP.

IP conflicts are prevented — not by partitioning the address space — but by the **reservations hostsfile**: a `dhcp-hostsfile` that dnsmasq consults before making any offer. The hostsfile is written and maintained by the mjolnir-mesh daemon from the CRDT state. As soon as a device gets a lease on any router, that reservation propagates via gossip to every other router within ~100ms. From that point forward, every router will offer the same IP to that MAC address.

In the rare case that two routers assign the same IP within the gossip propagation window, conflict resolution fires: the router with the higher HLC loses, deletes its lease, updates its hostsfile, SIGHUPs dnsmasq, and deauths the losing device over WiFi. The device reconnects in ~2 seconds, re-DHCPs, and gets a non-conflicting IP. The winner is undisturbed.

**Guarantees:**

- Every router eventually sees the same lease table (eventual consistency via gossip + anti-entropy).
- No device permanently holds a conflicting IP.
- Roaming devices receive the same IP at every AP without disruption.
- The mesh continues operating during network partitions; state reconciles on reconnect.

---

## 2. Data Model

### 2.1 Key Space

The CRDT is a key/value store with a structured namespace:

```
/devices/{mac}          — one entry per device, keyed by MAC
/dns/{hostname}         — hostname → IP mapping for mesh-wide DNS
/services/{name}        — mDNS-style service announcements
/subnets/{cidr}         — subnet ownership ledger (claim coordination only, not routing)
```

Where `{mac}` is the lowercase colon-separated MAC address (e.g., `aa:bb:cc:dd:ee:ff`), `{hostname}` is lowercase and DNS-safe, `{name}` is the service name, and `{cidr}` is CIDR notation with `/` escaped to `_` (e.g., `10.42.1.0_24`).

**Note:** the `/subnets/` namespace is *not* a routing table — it records which router owns which subnet range so two routers don't claim the same /24 at first boot. Actual route computation, propagation, and installation is delegated to Babel (`babeld`). See [babel-routing.md](../../network-coordination/babel-routing.md) for the rationale and integration.

### 2.2 Schema Summary

| Key prefix       | Value type      | Keyed by | Merge policy |
|------------------|-----------------|----------|--------------|
| `/devices/{mac}` | `LeaseEntry`    | MAC      | FWW on IP conflict |
| `/dns/{hostname}`| `DnsEntry`      | hostname | Last-writer-wins |
| `/services/{name}` | `ServiceEntry` | name    | Last-writer-wins |
| `/subnets/{cidr}` | `SubnetClaim`  | CIDR     | FWW (first-writer claims the subnet) |

### 2.3 Serialization

All values are serialized with **postcard** (compact binary, zero-copy friendly). Keys are UTF-8 strings. Wire messages wrap values in a `GossipMessage` enum (see §8) also serialized via postcard.

---

## 3. CRDT Semantics

### 3.1 Store Type

The CRDT is a **custom key/value store**, not iroh-docs. This is a deliberate choice: iroh-docs adds complexity (author keys, namespace management, sync protocol overhead) that is unnecessary when gossip + anti-entropy provides the replication we need.

The store is an in-memory `HashMap<String, VersionedEntry>` backed by a write-ahead log for crash recovery. Each entry carries its HLC timestamp, which is the version vector.

### 3.2 Merge Function

On receiving an entry from gossip or anti-entropy:

1. If the key does not exist locally, insert it.
2. If the key exists and the incoming HLC is **strictly greater**, replace the local entry.
3. If the key exists and the incoming HLC is **equal or less**, discard (already seen or stale).
4. If the incoming entry claims an IP already assigned to a **different MAC**, invoke conflict resolution (§5).

This is last-writer-wins for DNS/services, first-writer-wins (FWW) for the specific case of an IP collision across two different MAC entries, and FWW for subnet claims.

### 3.3 Gossip Replication

Every write to the local CRDT immediately broadcasts a `GossipMessage` to all connected peers via iroh gossip topics. Each router subscribes to the mesh gossip topic on startup.

Gossip is best-effort UDP multicast. Lost messages are recovered by anti-entropy (§9).

### 3.4 Anti-Entropy on Reconnect

When a peer (re)connects, both sides exchange a compact summary of their state (Merkle tree root or key/HLC digest map). The peer with missing or older entries requests the delta. This ensures partitioned routers converge immediately on reconnect without replaying the full gossip log.

---

## 4. Lease Lifecycle

### 4.1 Normal Flow

```
Device                  Router-A                   Mesh (all routers)
  |                        |                              |
  |--- DHCP Discover ----->|                              |
  |                        | check reservations hostsfile |
  |                        | (no entry for this MAC)      |
  |<-- DHCP Offer ---------|  assigns 192.168.1.42        |
  |--- DHCP Request ------>|                              |
  |<-- DHCP ACK -----------|                              |
  |                        |                              |
  |                        |-- dhcp-script fires -------->|
  |                        |   (add AA:BB:CC 192.168.1.42 laptop)
  |                        |                              |
  |                        | daemon receives event        |
  |                        | writes /devices/aa:bb:cc...  |
  |                        | to CRDT store                |
  |                        |                              |
  |                        |-- LeaseUpdate gossip ------->|
  |                        |                         Router-B, Router-C
  |                        |                         update hostsfile
  |                        |                         SIGHUP dnsmasq
  |                        |                              |
  ~100ms after DHCP ACK:   |                              |
  all routers now offer 192.168.1.42 to AA:BB:CC          |
```

### 4.2 Step-by-Step

1. **Device sends DHCP Discover.** Router-A's dnsmasq receives it.
2. **Hostsfile check.** dnsmasq reads `dhcp-hostsfile=/tmp/mjolnir/reservations`. If the MAC is listed, it offers the reserved IP. If not, it picks a free IP from the range.
3. **DHCP ACK.** dnsmasq sends the ACK and writes the lease to its local lease file.
4. **dhcp-script fires.** dnsmasq calls `mjolnir-mesh dhcp-event add AA:BB:CC:DD:EE:FF 192.168.1.42 laptop`. This is a thin Unix socket client that forwards the event to the running daemon process.
5. **Daemon writes CRDT.** The daemon constructs a `LeaseEntry` with the current HLC, writes it to `/devices/aa:bb:cc:dd:ee:ff`, and also writes a `DnsEntry` to `/dns/laptop`.
6. **Gossip broadcast.** The daemon immediately sends `LeaseUpdate` and `DnsUpdate` gossip messages to all peers.
7. **Peer routers update.** Each peer router receives the gossip message, merges the entry into its local CRDT, rewrites its hostsfile and DNS file, and SIGHUPs dnsmasq.
8. **Convergence.** Within ~100ms, every router in the mesh will offer `192.168.1.42` to MAC `AA:BB:CC:DD:EE:FF`.

### 4.3 Lease Release

When a device disconnects or its lease expires, dnsmasq calls `mjolnir-mesh dhcp-event del AA:BB:CC:DD:EE:FF 192.168.1.42`. The daemon publishes a `LeaseRelease` gossip message. All routers remove the entry from their CRDT, rewrite hostsfiles, and SIGHUP dnsmasq. The IP becomes available again.

### 4.4 Timing

| Event                              | Typical latency |
|------------------------------------|-----------------|
| DHCP ACK to dhcp-script call       | <1ms            |
| dhcp-script to daemon (Unix socket)| <1ms            |
| Daemon write to gossip broadcast   | <5ms            |
| Gossip delivery to peer router     | 10–80ms (WiFi)  |
| Peer hostsfile update + SIGHUP     | <10ms           |
| **Total: DHCP ACK to all-routers convergence** | **~100ms** |

---

## Service Lifecycle

Service entries are tied to their host device via `host_mac`. When a device lease expires and is reaped from the CRDT, all `/services/*` entries with matching `host_mac` are also removed. A `ServiceRemove` gossip message is broadcast to all peers.

This avoids a separate heartbeat/TTL mechanism for services — device expiry is the single source of truth.

### DNS Naming

The mesh uses `.mesh` as its DNS domain (e.g., `laptop-alice.mesh`, `wiki.mesh`). This avoids `.local`, which is reserved for mDNS/avahi and causes naming collisions when a device reconnects before its old mDNS name expires (avahi appends `-2`, `-3`, etc.).

Hostnames are sourced from DHCP Option 12 (what the client sends). If the client sends no hostname, the daemon generates a MAC-derived fallback: `dev-{last 4 hex digits}` (e.g., `dev-4d5e`). The hostname persists in the CRDT across roaming — a device keeps its name regardless of which router it connects to.

---

## 5. Conflict Resolution

### 5.1 When Conflicts Happen

A conflict occurs when two routers assign the same IP to two different devices within the ~100ms gossip propagation window. This requires:

- The same IP to be "free" on both routers simultaneously (i.e., not yet in either hostsfile).
- Two DHCP Discovers to arrive at different routers at approximately the same time.

This is rare in practice. A healthy mesh with ~100ms gossip latency and typical DHCP arrival rates (dozens per hour) will see conflicts roughly once every several thousand leases.

### 5.2 Detection

When Router-B receives a `LeaseUpdate` from Router-A for IP `192.168.1.42`, and Router-B's CRDT already contains a `LeaseEntry` for `192.168.1.42` assigned to a **different MAC**, conflict resolution fires on both routers independently (deterministically, same result).

### 5.3 Resolution: First-Writer-Wins

The router with the **lower HLC** wins — it was the first to assign the IP.

```
Router-A assigned 192.168.1.42 to AA:BB at HLC(t=1000, c=0, node="router-a")
Router-B assigned 192.168.1.42 to CC:DD at HLC(t=1001, c=0, node="router-b")

HLC comparison: t=1000 < t=1001  →  Router-A wins, Router-B loses
```

If wall clocks are equal (same millisecond), counter `c` breaks the tie. If HLC is identical, `router_id` string comparison breaks the tie deterministically.

### 5.4 Conflict Sequence

```
Router-A (winner)              Router-B (loser)              Device CC:DD
     |                               |                            |
     | receives LeaseUpdate          |                            |
     | for 192.168.1.42/CC:DD        |                            |
     | own entry wins (lower HLC)   |                            |
     | no action needed              |                            |
     |                               |                            |
     |                         receives LeaseUpdate              |
     |                         for 192.168.1.42/AA:BB            |
     |                         own entry loses (higher HLC)      |
     |                               |                            |
     |                         delete lease AA:BB from dnsmasq   |
     |                         rewrite hostsfile                  |
     |                         SIGHUP dnsmasq                    |
     |                               |                            |
     |                         deauth CC:DD via hostapd           |
     |                               |--- 802.11 Deauth -------->|
     |                               |                            |
     |                               |           ~2s: reconnect  |
     |                               |<-- DHCP Discover ---------|
     |                               |  hostsfile has no CC:DD   |
     |                               |  picks new IP (e.g. .43)  |
     |                               |--- DHCP ACK .43 --------->|
     |                               |                            |
Total elapsed: ~2.5 seconds. Winner (AA:BB at .42) undisturbed.
```

### 5.5 Why Not Propose-Confirm or Short Leases?

**Propose-confirm** adds a ~100ms window before every DHCP ACK, increases protocol complexity, and requires routers to wait for quorum before responding to the client. It solves a problem that almost never happens, at the cost of making every DHCP exchange slower and more fragile.

**Short leases** (e.g., 60s) force frequent re-DHCPs, increasing wireless traffic, waking devices unnecessarily, and complicating roaming. They also do not eliminate the conflict window — they just bound how long a conflict persists.

**Deauth-on-conflict** is better: zero overhead in the common case, ~2.5s disruption in the rare case. Devices auto-reconnect transparently. End users do not notice a 2-second blip on a freshly connected device.

---

## 6. Roaming

When a device moves from AP-1 to AP-2 (different router), its MAC is the same. Router-2's dnsmasq consults the reservations hostsfile before making any offer. The hostsfile contains `AA:BB:CC:DD:EE:FF,192.168.1.42,laptop` (propagated when the device first joined). Router-2 offers the same IP.

```
Device                 Router-1               Router-2
  |                       |                       |
  | (was connected here)  |                       |
  | moves physically ------------------------------------->|
  |                       |                       |
  |--- DHCP Discover -------------------------------------------->|
  |                       |               hostsfile: AA:BB → .42 |
  |<-- DHCP Offer (.42) ------------------------------------------|
  |--- DHCP Request --------------------------------------------->|
  |<-- DHCP ACK (.42) --------------------------------------------|
  |                       |                       |
  | same IP, no conflict, no disruption            |
```

If the device's original lease on Router-1 is still active, Router-2's dhcp-script fires an `add` event, the daemon writes an updated `LeaseEntry` with `router_id = "router-2"` and a fresh HLC. This propagates to Router-1, which updates its CRDT (new HLC is higher, so it replaces the old entry). No conflict — same MAC, same IP, different router. The CRDT key is `/devices/{mac}`, so the update is a simple replace.

---

## 7. dnsmasq Integration

### 7.1 Two Managed Files

The daemon maintains two files that dnsmasq reads:

**`/tmp/mjolnir/reservations`** — the dhcp-hostsfile. One line per device:

```
AA:BB:CC:DD:EE:FF,192.168.1.42,laptop
11:22:33:44:55:66,192.168.1.71,phone-alice
```

Format: `MAC,IP,hostname`. dnsmasq reads this before making any DHCP offer. If the MAC is listed, that IP is offered regardless of what the client requests. This is the primary mechanism for conflict prevention.

**`/tmp/mjolnir/dns`** — the addn-hosts file. One line per device:

```
192.168.1.42  laptop laptop.mesh
192.168.1.71  phone-alice phone-alice.mesh
```

Format: standard `/etc/hosts` — IP followed by one or more hostnames. dnsmasq uses this for DNS resolution of mesh devices.

### 7.2 dnsmasq Configuration Snippet

```
# DHCP range — same on every mesh router
dhcp-range=192.168.1.100,192.168.1.254,12h

# Reservations: prevents IP conflicts, enables roaming
dhcp-hostsfile=/tmp/mjolnir/reservations

# Mesh-wide DNS
addn-hosts=/tmp/mjolnir/dns

# Notify daemon of every lease event
dhcp-script=/usr/bin/mjolnir-mesh dhcp-event
```

### 7.3 dhcp-script: the `dhcp-event` Subcommand

dnsmasq calls the script with positional arguments:

```
mjolnir-mesh dhcp-event <action> <mac> <ip> <hostname>
```

Where `<action>` is `add`, `del`, or `old` (lease renewal). The `dhcp-event` subcommand is implemented as a thin Unix socket client. It connects to the daemon's control socket at `/run/mjolnir-mesh.sock`, sends the event as a serialized message, and exits. The daemon handles the event asynchronously.

This design keeps the dhcp-script process short-lived and non-blocking. dnsmasq does not wait for CRDT propagation before proceeding.

### 7.4 SIGHUP Reload

The daemon sends `SIGHUP` to dnsmasq after rewriting either managed file. dnsmasq reloads both files without restarting, without dropping active leases. This is the standard dnsmasq reload mechanism and takes <10ms.

The daemon debounces SIGHUPs: if multiple gossip updates arrive within 100ms, the files are written once and a single SIGHUP is sent.

---

## 8. Gossip Protocol

### 8.1 Transport

Gossip runs over iroh's gossip layer (iroh 0.96, pinned). Each router subscribes to a shared mesh topic derived from the network's public key. Messages are best-effort, unordered, and may be duplicated. The CRDT merge function handles all three cases correctly.

### 8.2 Message Types

```rust
enum GossipMessage {
    LeaseUpdate(LeaseEntry),
    LeaseRelease { mac: [u8; 6], hlc: HLC },
    DnsUpdate { hostname: String, entry: DnsEntry },
    ServiceUpdate { name: String, entry: ServiceEntry },
    SubnetClaimUpdate { cidr: String, entry: SubnetClaim },
    SubnetClaimRelease { cidr: String, hlc: HLC },
}
```

All variants are serialized with postcard. The enum discriminant is a single byte prefix.

### 8.3 Topic Naming

```
mjolnir/mesh/{network_id}/crdt
```

Where `{network_id}` is the hex-encoded public key of the mesh network keypair. All routers in the same mesh join the same topic.

### 8.4 Message Flow

On any CRDT write, the daemon:

1. Writes to local store.
2. Rewrites affected files (`reservations` and/or `dns`).
3. Sends SIGHUP to dnsmasq (debounced).
4. Broadcasts the appropriate `GossipMessage` to the mesh topic.

On receiving a gossip message:

1. Deserialize the `GossipMessage`.
2. Apply the merge function to the local CRDT store.
3. If the merge resulted in a change (entry was new or updated):
   a. Rewrite affected files.
   b. Schedule debounced SIGHUP.
   c. Check for IP conflicts; invoke resolution if needed.
4. If the merge resulted in no change (duplicate or stale), discard.

---

## 9. Anti-Entropy Sync

### 9.1 Purpose

Gossip is unreliable. A router that was offline, or that missed messages due to packet loss, may have a stale view of the mesh state. Anti-entropy ensures eventual convergence regardless of message loss.

### 9.2 Full State Exchange on Reconnect

When a router (re)connects to a peer:

1. Both sides send a `StateSummary`: a map of `{ key → hlc }` covering all entries in their local CRDT.
2. Each side computes the diff: entries the peer has that it does not, or entries where the peer's HLC is newer.
3. Each side requests the missing/stale entries by key.
4. The responding side sends the full `VersionedEntry` for each requested key.
5. Both sides merge the received entries.

This brings both routers to a consistent state before gossip resumes.

### 9.3 Merkle-Tree Delta Sync (Optimization)

For large meshes (hundreds of devices), exchanging a full `{ key → hlc }` map on every reconnect adds overhead. As an optimization, the CRDT store maintains a Merkle tree over its key/value pairs. On reconnect, routers exchange only the Merkle root. If roots match, no sync is needed. If they differ, routers descend the tree to identify the differing subtrees, minimizing the number of keys exchanged.

This optimization is not required for correctness and can be added incrementally after the basic anti-entropy is stable.

### 9.4 Periodic Anti-Entropy

Even without reconnects, routers run a periodic anti-entropy sweep every 30 seconds. This catches any gossip messages that were silently dropped without a disconnect event.

---

## 10. Rust Types

> **Implementation status note.** Types in this section marked ✅ exist in `crates/mjolnir-mesh/src/crdt/` today (HLC, LeaseEntry, DnsEntry, ServiceEntry, SubnetClaim, GossipMessage, plus the merge helpers). The `CrdtStore` trait (§10.8), the `DhcpScriptHandler` (§10.9), and the lease-conflict path described in §5.4 are designed but not yet implemented — they belong to the next epic (daemon wiring: iroh-gossip ↔ store ↔ dnsmasq ↔ hostapd ↔ babeld supervisor). Existing implementation is library-shaped (pure data + pure functions); the daemon glue is the lane that hasn't shipped yet.

### 10.1 HLC

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HLC {
    /// Wall clock time in milliseconds since Unix epoch
    pub wall_clock: u64,
    /// Monotonic counter, incremented when wall_clock equals last observed max
    pub counter: u32,
    /// Node ID of the router that generated this timestamp
    pub node_id: String,
}

impl Ord for HLC {
    fn cmp(&self, other: &Self) -> Ordering {
        self.wall_clock
            .cmp(&other.wall_clock)
            .then(self.counter.cmp(&other.counter))
            .then(self.node_id.cmp(&other.node_id))
    }
}

impl PartialOrd for HLC {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
```

HLC is ~30 lines of implementation. The key invariant: `new_hlc.wall_clock >= max(local_wall_clock, received_hlc.wall_clock)`. On receipt of any message, the local HLC is advanced past the received HLC before generating the next timestamp.

Entries with `wall_clock` more than `MAX_CLOCK_SKEW` (60 seconds) in the future are rejected to prevent a misconfigured router from poisoning the CRDT with far-future entries.

### 10.2 Lease Entry

```rust
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseEntry {
    pub mac: [u8; 6],
    pub ip: IpAddr,            // IpAddr::V4 today; IpAddr::V6 forward-compatible
    pub hostname: Option<String>,
    pub router_id: String,
    pub expiry: u64,
    pub hlc: HLC,
}
```

Keyed by MAC (`/devices/aa:bb:cc:dd:ee:ff`). One entry per device. The `ip` field uses `std::net::IpAddr` (an enum over `Ipv4Addr` and `Ipv6Addr`) so the data model is IP-version-agnostic. The MVP only writes `IpAddr::V4` values; v6 codepaths in dnsmasq integration are not yet wired up but the schema does not need to change when they are. `expiry` is Unix timestamp in seconds; the daemon periodically reaps expired entries and broadcasts `LeaseRelease`.

### 10.3 DNS Entry

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsEntry {
    pub ip: IpAddr,
    pub mac: [u8; 6],
}
```

Keyed by hostname (`/dns/laptop`). Derived from the corresponding `LeaseEntry` when the lease is written. Updated when the device roams (IP stays the same, but the entry is refreshed with the new HLC).

### 10.4 Service Entry

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub hostname: String,
    pub ip: IpAddr,
    pub port: u16,
    pub protocol: String,
    pub txt: HashMap<String, String>,
    pub host_mac: [u8; 6],  // tied to device lease — service expires when device expires
}
```

Keyed by service name (`/services/printer._ipp._tcp`). Used for mDNS-style service discovery across the mesh. Written by the daemon when a service is announced; gossipped to all routers.

### 10.5 Subnet Claim

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubnetClaim {
    pub cidr: IpNet,             // e.g. 10.42.1.0/24
    pub owner_node_id: String,
    pub site_name: Option<String>,
    pub claimed_at: HLC,
}
```

Keyed by CIDR with `/` escaped to `_` (e.g., `/subnets/10.42.1.0_24` or `/subnets/10.42.0.0_22`). Records which mesh node has claimed a given subnet range so other routers don't claim an overlapping range. **Not a routing table** — has no `via_node_id`, no metric, no expiry. Babel (see [babel-routing.md](../../network-coordination/babel-routing.md)) handles route computation and installation. The CRDT entry exists only to coordinate first-boot subnet claims and survive partition/heal scenarios where two routers may have picked overlapping ranges independently.

The CIDR's prefix length is configurable per router — operators pick the size their site needs (/24 for ≤254 devices, /22 for ~1 000, /20 for ~4 000, /16 for the whole base). See [network-architecture.md](../../network-coordination/network-architecture.md) "Subnet Allocation for Remote Sites" and the `mjolnir_mesh::alloc` module.

Conflicts (two routers write overlapping `/subnets/{cidr}` entries — at the same prefix or with one containing the other) resolve FWW on `claimed_at` HLC. The loser picks the next free slot at its configured prefix length and rewrites its claim.

### 10.6 Gossip Message Enum

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
enum GossipMessage {
    LeaseUpdate(LeaseEntry),
    LeaseRelease { mac: [u8; 6], hlc: HLC },
    DnsUpdate { hostname: String, entry: DnsEntry },
    ServiceUpdate { name: String, entry: ServiceEntry },
    SubnetClaimUpdate { cidr: String, entry: SubnetClaim },
    SubnetClaimRelease { cidr: String, hlc: HLC },
}
```

### 10.7 Conflict Resolution

```rust
/// First-writer-wins: lower HLC = first claimer.
/// Only called when two entries claim the same IP for different MACs.
fn resolve_conflict<'a>(a: &'a LeaseEntry, b: &'a LeaseEntry) -> (&'a LeaseEntry, &'a LeaseEntry) {
    // returns (winner, loser)
    match a.hlc.cmp(&b.hlc) {
        Ordering::Less => (a, b),
        Ordering::Greater => (b, a),
        Ordering::Equal => {
            if a.router_id <= b.router_id { (a, b) } else { (b, a) }
        }
    }
}

fn on_conflict(&self, ip: Ipv4Addr, winner: &LeaseEntry, loser: &LeaseEntry) {
    if loser.router_id == self.router_id {
        self.dnsmasq.delete_lease(&loser.mac);
        self.dnsmasq.update_hostsfile();
        self.dnsmasq.sighup();
        self.hostapd.deauth(&loser.mac);
    }
}
```

`resolve_conflict` is pure and deterministic — every router that sees the same two entries will reach the same (winner, loser) conclusion. `on_conflict` only takes action if this router is the loser's router, preventing duplicate deauths.

### 10.8 CRDT Store Trait

```rust
pub trait CrdtStore {
    fn get_lease(&self, mac: &[u8; 6]) -> Option<LeaseEntry>;
    fn put_lease(&mut self, entry: LeaseEntry) -> MergeResult;
    fn delete_lease(&mut self, mac: &[u8; 6], hlc: HLC);
    fn get_dns(&self, hostname: &str) -> Option<DnsEntry>;
    fn put_dns(&mut self, hostname: String, entry: DnsEntry);
    fn all_leases(&self) -> Vec<LeaseEntry>;
    fn all_dns(&self) -> Vec<(String, DnsEntry)>;
    fn state_summary(&self) -> HashMap<String, HLC>;
}

pub enum MergeResult {
    Inserted,
    Updated,
    Unchanged,
    Conflict { winner: LeaseEntry, loser: LeaseEntry },
}
```

### 10.9 DHCP Script Handler

```rust
pub struct DhcpScriptHandler {
    socket_path: PathBuf,
}

impl DhcpScriptHandler {
    /// Called by the `dhcp-event` subcommand process.
    /// Sends the event to the daemon over Unix socket and exits.
    pub fn send_event(&self, action: DhcpAction, mac: [u8; 6], ip: Ipv4Addr, hostname: Option<String>) {
        let event = DhcpEvent { action, mac, ip, hostname };
        let conn = UnixStream::connect(&self.socket_path).expect("daemon not running");
        postcard::to_io(&event, conn).expect("send failed");
    }
}

pub enum DhcpAction { Add, Del, Old }

pub struct DhcpEvent {
    pub action: DhcpAction,
    pub mac: [u8; 6],
    pub ip: Ipv4Addr,
    pub hostname: Option<String>,
}
```

---

## 11. Performance

### 11.1 Gossip Latency

On a WiFi mesh with 2–5 hops, gossip latency is typically 10–80ms per hop. For a 3-router mesh, a lease update reaches all routers within ~100ms of the DHCP ACK. This is the conflict window.

### 11.2 Memory

Each `LeaseEntry` is approximately 60 bytes on the wire (postcard). In memory, with Rust overhead, roughly 128 bytes per entry. A mesh with 1,000 active devices uses ~128KB of CRDT lease state. DNS entries are similarly small. Total CRDT memory for a typical home or small office mesh (50–200 devices): <1MB.

### 11.3 CRDT Store Size

The store holds one entry per MAC address (not per IP, not per lease event). There is no log; the store is a flat map. Expired entries are pruned on a 5-minute timer. Store size is bounded by the number of unique devices ever seen on the mesh.

### 11.4 SIGHUP Cost

dnsmasq SIGHUP reload takes <10ms. File rewrite (hostsfile, dns) takes <1ms for typical mesh sizes. The 100ms debounce prevents excessive reloads during gossip bursts (e.g., when a large batch of anti-entropy updates arrives).

### 11.5 dhcp-script Overhead

The `dhcp-event` Unix socket call adds <1ms to the dnsmasq lease commit path. dnsmasq does not block on the script completing CRDT propagation — it fires and forgets. The script process itself exits immediately after sending to the socket.

---

## 12. Edge Cases

### 12.1 Clock Skew

HLC is used precisely because wall clocks on embedded routers can drift. The HLC counter component ensures monotonic ordering even when `wall_clock` values are equal or slightly out of order across routers.

For extreme skew: any entry with `hlc.wall_clock > local_time + MAX_CLOCK_SKEW` (60 seconds) is rejected and logged. This prevents a router with a misconfigured clock from polluting the CRDT with entries that would "win" all future conflicts due to far-future timestamps. The 60-second threshold is generous enough to survive NTP sync delays on startup.

### 12.2 Lost Gossip Messages

Lost gossip messages are handled by anti-entropy. If Router-B misses a `LeaseUpdate` from Router-A, it will receive the entry during the next anti-entropy sweep (within 30 seconds) or immediately upon reconnect. Until then, Router-B may offer a conflicting IP for that MAC — but this is the same conflict scenario already handled by §5.

### 12.3 dnsmasq Restart

If dnsmasq restarts (crash, systemd restart, config change), it reads the managed files fresh from disk. The daemon maintains the files on disk at all times — they are not in-memory only. A fresh dnsmasq instance comes up with the full current reservation set immediately.

### 12.4 Daemon Crash

If the mjolnir-mesh daemon crashes, dnsmasq continues operating with the last-written hostsfile. No new CRDT updates are processed until the daemon restarts. On restart:

1. Daemon reads the WAL (write-ahead log) to restore local CRDT state.
2. Daemon reconnects to mesh peers.
3. Anti-entropy runs immediately, pulling any updates missed during downtime.
4. Daemon rewrites hostsfile and dns files from current CRDT state.
5. SIGHUP sent to dnsmasq.

Total recovery time: typically <5 seconds, depending on peer connectivity.

### 12.5 Network Partition

During a partition, each partition operates independently. Routers may assign the same IPs to different devices. When the partition heals:

1. Anti-entropy runs on reconnect.
2. Conflicts are detected and resolved via FWW (§5).
3. Losing devices are deauthed and re-DHCP.

The number of conflicts is bounded by the number of IPs assigned in both partitions to different MACs during the split — typically zero or small in a home/office context.

### 12.6 MAC Address Conflict

If two devices present the same MAC (rare but possible with MAC randomization bugs or cloned MACs), the CRDT treats them as one device. The second assignment overwrites the first if its HLC is higher. This is the correct behavior: one IP per MAC is the invariant.

---

## 13. References

- **iroh 0.96** (pinned): `iroh = "0.96"` — pinned due to `web-transport-iroh` compatibility constraint. Do not upgrade without verifying the transport plugin builds.
- **postcard**: `postcard = { version = "1", features = ["alloc"] }` — zero-copy binary serialization.
- **dnsmasq dhcp-hostsfile**: `man 8 dnsmasq`, `--dhcp-hostsfile` option.
- **dnsmasq addn-hosts**: `man 8 dnsmasq`, `--addn-hosts` option.
- **Hybrid Logical Clocks**: Kulkarni et al., "Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases", HotDeps 2014.
- **Related docs**:
  - `mesh-network-coordination.md` — high-level mesh vision
  - [§8 Gossip Protocol](#8-gossip-protocol) — iroh gossip transport details (this document)
  - `../../network-coordination/network-architecture.md` — subnet claim lifecycle and cross-site packet flow
  - `../../network-coordination/babel-routing.md` — Babel integration; rationale for delegating routing