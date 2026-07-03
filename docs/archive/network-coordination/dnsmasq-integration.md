> **ARCHIVED 2026-07-02 — NOT IMPLEMENTED / SUPERSEDED.** The daemon never managed
> dnsmasq files or a dhcp-script; shipped reality is stock OpenWrt dnsmasq steered by
> UCI reconciliation of the node's claimed /24 (see `reconcile_client_uci` in
> `crates/mjolnir-mesh/src/bin/mjolnir-meshd.rs` — it sets `network.lan.ipaddr` and
> restarts dnsmasq via init.d; never SIGHUP). Kept as design reference for the
> service-mesh phase (bead `e21`).

# dnsmasq Integration

## Summary

mjolnir-mesh integrates with dnsmasq (the default DHCP/DNS server on OpenWrt) through three mechanisms: a reservations file that prevents IP conflicts, a hosts file for mesh-wide DNS, and the dhcp-script hook that reports lease events. The daemon manages these files from the CRDT store and signals dnsmasq to reload via SIGHUP.

---

## The Two Files

### 1. DHCP Reservations (`dhcp-hostsfile`)

Configure dnsmasq to read from the reservations file:

```bash
# /etc/dnsmasq.conf (or UCI equivalent)
dhcp-hostsfile=/tmp/mjolnir/reservations
```

The file format — written by the mjolnir-mesh daemon from the CRDT store:

```
# /tmp/mjolnir/reservations
# Written by mjolnir-mesh daemon from CRDT store
# Format: MAC,IP,hostname
AA:BB:CC:DD:EE:01,10.42.1.50,laptop-alice
AA:BB:CC:DD:EE:02,10.42.1.51,printer-bob
CC:DD:EE:FF:00:03,10.42.1.52,phone-carol
```

What this does:

- Tells dnsmasq: "If MAC `AA:BB:CC:DD:EE:01` asks for an IP, give it `10.42.1.50`"
- Also: "Don't give `10.42.1.50` to anyone else"
- dnsmasq reads this on startup and on SIGHUP
- This is the PRIMARY mechanism that prevents IP conflicts in multi-dnsmasq setups
- Every known device→IP binding from any router in the mesh becomes a line here

**Why this prevents conflicts:**

When a device sends DHCP DISCOVER, it broadcasts to all routers on the L2. Multiple dnsmasq instances receive it. All of them check their hostsfile — they all have the same CRDT-synced data. If the MAC is known, all offer the same IP. If the MAC is new, each picks from the remaining (unreserved) pool. The only conflict case: a new device arrives and two routers pick the same unreserved IP within the ~100ms gossip propagation window.

---

### 2. DNS Hosts (`addn-hosts`)

Configure dnsmasq to read the mesh DNS file:

```bash
# /etc/dnsmasq.conf
addn-hosts=/tmp/mjolnir/dns
```

The file format — standard hosts file with optional aliases:

```
# /tmp/mjolnir/dns
# Written by mjolnir-mesh daemon from CRDT store
# Standard hosts file format: IP hostname [aliases...]
10.42.1.50  laptop-alice  laptop-alice.mesh
10.42.1.51  printer-bob   printer-bob.mesh
10.42.1.52  phone-carol   phone-carol.mesh
10.42.2.30  wiki-server   wiki-server.mesh
```

What this does:

- dnsmasq serves these as DNS A records
- Any device on the mesh can resolve `printer-bob.mesh` → `10.42.1.51`
- Includes devices from ALL routers in the mesh (local and remote)
- The `.mesh` suffix is configurable (could be `.local`, `.lan`, or custom)
- dnsmasq reads this on startup and on SIGHUP

---

## The dhcp-script Hook

```bash
# /etc/dnsmasq.conf
dhcp-script=/usr/bin/mjolnir-mesh dhcp-event
```

dnsmasq calls this synchronously on every lease event:

```bash
# New lease
/usr/bin/mjolnir-mesh dhcp-event add AA:BB:CC:DD:EE:01 10.42.1.50 laptop-alice

# Lease renewal
/usr/bin/mjolnir-mesh dhcp-event old AA:BB:CC:DD:EE:01 10.42.1.50 laptop-alice

# Lease expiry or release
/usr/bin/mjolnir-mesh dhcp-event del AA:BB:CC:DD:EE:01 10.42.1.50 laptop-alice
```

Environment variables also set by dnsmasq:

- `DNSMASQ_INTERFACE` — the network interface (e.g., `br-lan`)
- `DNSMASQ_LEASE_EXPIRES` — Unix timestamp of lease expiry
- `DNSMASQ_TIME_REMAINING` — seconds until expiry

**The `dhcp-event` subcommand** is a thin Unix socket client, NOT the full daemon. It:

1. Connects to `/run/mjolnir-mesh.sock` (the running daemon's Unix socket)
2. Serializes the event as postcard bytes
3. Sends it
4. Exits

This is fast (~1ms) because it doesn't start an Iroh node or load any state. dnsmasq blocks until it returns, so speed matters.

The daemon receives the event and:

1. Writes the lease to the local CRDT store
2. Broadcasts via iroh-gossip to all peers
3. Updates the reservations file (if the entry is new)
4. Updates the DNS file
5. Sends SIGHUP to dnsmasq to reload

---

## SIGHUP Behavior

After writing either file, the daemon sends SIGHUP to dnsmasq:

```rust
// Find dnsmasq PID and signal it
let pid = std::fs::read_to_string("/var/run/dnsmasq/dnsmasq.pid")?.trim().parse::<i32>()?;
unsafe { libc::kill(pid, libc::SIGHUP); }
```

What SIGHUP does to dnsmasq:

- Re-reads `dhcp-hostsfile` (reservations)
- Re-reads `addn-hosts` (DNS hosts)
- Does NOT clear the lease table
- Does NOT disrupt active DHCP conversations
- Does NOT restart the process
- Safe to call frequently (the daemon debounces — batches updates, SIGHUPs at most once per 100ms)

---

## Conflict Resolution Flow

When two routers assign the same IP to different MACs (rare — only within ~100ms gossip window):

```
Router-A assigns 10.42.1.50 to MAC-X (HLC=100)
Router-B assigns 10.42.1.50 to MAC-Y (HLC=101)
  │
  ├─ Both dhcp-scripts fire, both daemons write to CRDT
  ├─ Gossip propagates (~100ms)
  ├─ Both daemons detect conflict: same IP, different MACs
  │
  ▼ FWW: lower HLC wins → MAC-X keeps 10.42.1.50

  On Router-B (served the loser):
  ├─ Delete MAC-Y's lease:
  │    Write empty lease file, SIGHUP, or:
  │    echo "" > /tmp/dhcp.leases.d/MAC-Y && kill -HUP $(cat /var/run/dnsmasq.pid)
  ├─ Update reservations file: 10.42.1.50 → MAC-X (the winner)
  ├─ SIGHUP dnsmasq
  ├─ Deauth MAC-Y from WiFi:
  │    hostapd_cli deauthenticate AA:BB:CC:DD:EE:YY
  ├─ MAC-Y's device auto-reconnects (~2 sec)
  ├─ Device sends DHCP DISCOVER
  ├─ dnsmasq sees 10.42.1.50 reserved for MAC-X → picks different IP
  └─ MAC-Y gets new IP. Done.

  On Router-A (served the winner):
  └─ Nothing to do. MAC-X is undisturbed.
```

Total resolution time: ~2.5 seconds. One device gets a brief WiFi blink. The other notices nothing.

---

## Lease Deletion on OpenWrt

Deleting a specific lease from dnsmasq on OpenWrt:

```bash
# Option 1: ubus (if available)
ubus call dhcp delete '{"mac":"AA:BB:CC:DD:EE:YY"}'

# Option 2: Edit lease file + SIGHUP
# /tmp/dhcp.leases format: "expiry MAC IP hostname clientid"
sed -i '/AA:BB:CC:DD:EE:YY/d' /tmp/dhcp.leases
kill -HUP $(cat /var/run/dnsmasq/dnsmasq.pid)

# Option 3: Short lease + let it expire (not recommended — too slow)
```

The daemon should try ubus first (cleanest), fall back to lease file edit.

---

## WiFi Deauth

```bash
# hostapd_cli is available on OpenWrt
hostapd_cli -i wlan0 deauthenticate AA:BB:CC:DD:EE:YY

# If multiple radio interfaces:
for iface in wlan0 wlan1; do
    hostapd_cli -i $iface deauthenticate AA:BB:CC:DD:EE:YY 2>/dev/null
done
```

The device's WiFi driver handles auto-reconnection. Most devices reconnect within 1-3 seconds.

---

## dnsmasq Configuration (Complete Example)

```bash
# /etc/dnsmasq.conf additions for mjolnir-mesh

# DHCP range — same on all routers at the same site
dhcp-range=10.42.1.100,10.42.1.254,255.255.255.0,1h

# Reservations from CRDT (prevents conflicts)
dhcp-hostsfile=/tmp/mjolnir/reservations

# Mesh-wide DNS
addn-hosts=/tmp/mjolnir/dns

# Notify daemon of lease events
dhcp-script=/usr/bin/mjolnir-mesh dhcp-event

# Domain for mesh hostnames
domain=mesh,10.42.1.0/24,local

# Don't serve DHCP on WAN interface
no-dhcp-interface=eth0.2
# (WAN interface name varies by device)

# dnsmasq reads additional config fragments from this directory
conf-dir=/tmp/mjolnir/conf.d/
```

OpenWrt UCI equivalent:

```
# /etc/config/dhcp additions
config dnsmasq
    option dhcphostsfile '/tmp/mjolnir/reservations'
    option addnhosts '/tmp/mjolnir/dns'
    list dhcp_script '/usr/bin/mjolnir-mesh dhcp-event'
```

---

## File Locations on OpenWrt

| File | Path | Written by | Read by |
|------|------|------------|---------|
| Reservations | `/tmp/mjolnir/reservations` | mjolnir-mesh daemon | dnsmasq |
| DNS hosts | `/tmp/mjolnir/dns` | mjolnir-mesh daemon | dnsmasq |
| dnsmasq leases | `/tmp/dhcp.leases` | dnsmasq | daemon (initial sync) |
| dnsmasq PID | `/var/run/dnsmasq/dnsmasq.pid` | dnsmasq | daemon (for SIGHUP) |
| Daemon socket | `/run/mjolnir-mesh.sock` | daemon | dhcp-event subcommand |
| CRDT store | `/tmp/mjolnir/crdt/` or `/mnt/sd/mjolnir/crdt/` | daemon | daemon |

All `/tmp/` files are on tmpfs (RAM). Survives reboots only if CRDT store is on SD card and peers are available for anti-entropy sync.

---

## Startup Sequence

1. dnsmasq starts with **no dhcp-range configured** (DHCP serving disabled, DNS still active)
2. mjolnir-mesh daemon starts
3. Connects to Iroh mesh, begins anti-entropy sync with peers
4. Waits for sync to complete **or** 10-second timeout (for solo first router with no peers)
5. Populates `/tmp/mjolnir/reservations` from CRDT
6. Populates `/tmp/mjolnir/dns` from CRDT
7. Writes `/tmp/mjolnir/conf.d/dhcp-range.conf` with the subnet's dhcp-range
8. Sends SIGHUP to dnsmasq (picks up reservations, DNS, and dhcp-range)
9. Reads `/tmp/dhcp.leases` for any leases from a previous daemon run
10. Opens Unix socket at `/run/mjolnir-mesh.sock`
11. Ready to receive dhcp-event messages

This sequence prevents DHCP assignment before the CRDT is populated. The 10-second timeout ensures a solo router (first at a new site) can start serving without peers.

---

## Edge Cases

- **dnsmasq restarts**: Daemon detects (inotify on PID file or process monitor), re-sends SIGHUP after dnsmasq is up
- **Daemon restarts**: On startup, reads CRDT from peers (anti-entropy), repopulates both files, SIGHUPs dnsmasq. Brief window where new hostsfile may be missing recent entries — gossip catches up within seconds.
- **Both restart**: First one up waits for the other. Daemon can start without dnsmasq (just can't SIGHUP yet). dnsmasq can start without daemon files (serves DHCP from its own state, daemon catches up).
- **WAN DHCP**: dhcp-script only fires for LAN-side DHCP. dnsmasq doesn't serve DHCP on WAN interface (`no-dhcp-interface`). No special handling needed.

---

## Multi-dnsmasq Offer Behavior

When multiple routers run dnsmasq on the same L2, a single DHCP Discover broadcast triggers Offers from ALL routers. This is by design and is not a problem.

**Per-device packet count (10 routers):**
- 1 Discover (broadcast, ~342 bytes)
- 10 Offers (one per router, ~342 bytes each)
- 1 Request (broadcast, names the selected router)
- 1 Ack (from selected router)
- **Total: 13 packets, ~4.5 KB**

**At scale (200 devices, 10 routers, 30 minutes):**
- ~2,600 DHCP packets, ~900 KB total
- WiFi at 20 Mbps can transmit this in under 0.4 seconds
- Negligible compared to beacon traffic on a busy channel

The 9 non-selected routers simply discard their tentative allocation per RFC 2131. dnsmasq handles this correctly with no side effects, no error logging, and no leaked state.

> **Testing note:** Verify with a 3-router manual test that dnsmasq logs no errors on unselected Offers.

---

## References

- CRDT design: `dhcp-crdt.md`
- Network architecture: `../../network-coordination/network-architecture.md`
- Top-level overview: `mesh-network-coordination.md`