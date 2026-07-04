# Prior Art and External Comparison

**Status:** Reference | **Date:** 2026-05-18

How mjolnir-mesh compares to existing approaches for mesh DHCP, routing, and service discovery. Written to (a) make our design choices auditable against state-of-the-art, (b) flag where we diverge from convention deliberately, and (c) preempt the "why didn't you just use X" question.

---

## 1. The Three Reference Points

### 1.1 CeroWrt + AHCP + Babel

[CeroWrt](https://www.bufferbloat.net/projects/cerowrt/wiki/Mesh/) is the canonical OpenWrt-based mesh distro from the bufferbloat.net group. Its stack:

| Layer | Choice |
|---|---|
| Address allocation | **AHCP** (Ad-Hoc Configuration Protocol) — single designated server per network, hands out IPs + DNS + NTP from configured ranges |
| Subnet partitioning | Manual: "unique subnet number per router" — operators assign ranges out-of-band |
| Routing | **Babel** (default), with OLSR / batman / Quagga BGP as alternatives |
| DNS | Distributed via AHCP-configured resolvers; no mesh-wide directory |
| Service discovery | None at the routing layer; standard DNS + per-host mDNS |
| IPv4 vs IPv6 | Dual-stack; AHCP can hand out either or both |

**Key insight:** CeroWrt separates *configuration* (AHCP) from *routing* (Babel). They are independent protocols with different roles. Our `dhcp-crdt` ↔ `babel-routing` split mirrors this.

### 1.2 AHCP (standalone)

[AHCP](https://www.irif.fr/~jch/software/ahcp/) by Juliusz Chroboczek, designed for ad-hoc networks where DHCP cannot reach every L2 broadcast domain. From the spec:

> "AHCP is an autoconfiguration protocol for IPv6 and dual-stack IPv6/IPv4 networks designed to be used in place of router discovery and DHCP on networks where it is difficult or impossible to configure a server within every link-layer broadcast domain."

| Property | AHCP |
|---|---|
| Server model | One or a few designated servers per network |
| Address allocation | Pulls from a pre-configured range; servers do not coordinate, operators ensure non-overlap |
| Routing | Explicitly **not** configured by AHCP — "designed to be run together with a routing protocol such as Babel or OLSR" |
| Replacement | Author plans to deprecate in favor of **HNCP** (Homenet) |
| IP version bias | IPv6-first, IPv4 supported |

**Key insight:** AHCP still has a writer asymmetry — one (or a small number of) servers hands out addresses. It does not solve the "every router can independently assign IPs without conflict" problem because operators ensure non-overlapping ranges out-of-band.

### 1.3 OpenWISP

[OpenWISP](https://openwisp.io/docs/24.11/tutorials/mesh.html) is a centrally-managed network configuration platform for fleets of OpenWrt devices.

| Layer | Choice |
|---|---|
| Address allocation | Punt: assumes one external LAN DHCP server, bridges mesh into that LAN |
| Routing | "Out of scope of this tutorial" |
| DNS | Not addressed |
| Service discovery | Not addressed |
| IPv4 vs IPv6 | Disables DHCPv6 explicitly, no rationale given |

**Key insight:** OpenWISP is centrally orchestrated — it solves "how do I configure 100 OpenWrt routers consistently" by having a control plane, not by making the routers themselves coordinate. Their mesh tutorial is fundamentally a different problem than ours.

---

## 2. Comparison Matrix

| Concern | CeroWrt + AHCP | OpenWISP | mjolnir-mesh |
|---|---|---|---|
| **DHCP writers** | 1 server per network | 1 external LAN server | **N** — every router |
| **Conflict prevention** | Manual range partitioning | Single writer eliminates conflicts | **Gossiped reservations hostsfile + deauth-on-conflict** |
| **Liveness model** | Leader assumed up | Leader assumed up | **Symmetric — any router can die** |
| **Routing** | Babel | None / external | **Babel** (after this revision) |
| **Service discovery** | None at mesh layer | None | **CRDT-replicated `/services/` directory** |
| **DNS scope** | Per-resolver | LAN-scoped | **Mesh-wide via CRDT** |
| **State sync** | None (config is static) | Push from controller | **iroh-gossip + anti-entropy** |
| **Identity** | IP + manual config | OpenWISP UUID | **Iroh NodeId (Ed25519)** |
| **NAT / cross-site** | Out of scope | Out of scope | **Iroh QUIC tunnels with hole-punching** |
| **IP version** | Dual-stack | IPv4 only (DHCPv6 off) | **IPv4 today, v6-ready data model** |

---

## 3. Where We Diverge — and Why

### 3.1 Symmetric multi-writer DHCP

**Diverges from:** all three references — every prior art has exactly one DHCP writer (CeroWrt one AHCP server, OpenWISP one LAN server, no one does multi-writer).

**Why we diverge:** target deployment is DWEB events with 10+ co-located routers and aggressive device roaming. A single-writer model creates a single point of failure (lose the leader, lose new leases) and complicates roaming (every roam is a renew, and the leader must know about it). Symmetric multi-writer with CRDT reconciliation eliminates the leader and makes roaming a CRDT update, not a leader-mediated handoff.

**Cost:** the ~100ms conflict window and the deauth-on-conflict path. Both are bounded: conflicts are rare, deauth recovery is ~2s, and prior art tolerates worse failure modes (lost leader = no new leases until manual intervention).

### 3.2 Mesh-wide service directory

**Diverges from:** prior art doesn't address this at all. Standard practice is per-host mDNS + avahi reflectors for cross-router visibility.

**Why we diverge:** mDNS reflectors are operationally fragile — loops, name collisions on reconnect, multicast storms in larger meshes. A gossiped `/services/{name}` directory sidesteps the entire reflector mess and is naturally tied to device lease lifecycle (via `host_mac`), so service cleanup is automatic.

**The client edge (the part mDNS keeps):** stock devices speak mDNS/DNS-SD natively and won't switch to unicast queries for service browsing, so the node's role on its own /24 is *translator, not repeater*: harvest local mDNS announcements into the CRDT (ingest), answer `.mesh` unicast queries from the local replica, and optionally re-announce remote CRDT entries into the local mDNS domain as a proxy (serve out). Broadcast never crosses a segment; the directory does.

**The translator pattern has precedent** — we recombine two proven pieces rather than inventing one:

- **DNS-SD was designed for unicast.** [RFC 6763](https://www.rfc-editor.org/rfc/rfc6763) defines service discovery over *both* multicast and ordinary unicast DNS, and Apple's Wide-Area Bonjour deployed exactly the "harvest locally, serve by unicast DNS" split two decades ago. The industry kept the multicast half and forgot the unicast half. A `.mesh` responder serving CRDT-backed service records is unicast DNS-SD with a gossiped, ownerless backend where Wide-Area Bonjour assumed a conventional authoritative DNS server.
- **OpenThread border routers ship the same shape today:** Thread devices register services with an SRP (Service Registration Protocol) server on the border router, which re-announces them to the home LAN through an mDNS *advertising proxy*. Registration in, proxy out, no multicast across the constrained network — the identical translator, with our CRDT standing in for SRP's single registrar (and removing its authority).

The reflector critique above sharpens accordingly: reflectors fail because they extend a *broadcast domain*; the translator works because it replicates a *database* and re-publishes at each edge. The CRDT is the reflector done right.

### 3.3 IPv4-only

**Diverges from:** AHCP and CeroWrt are dual-stack with an IPv6 lean; Homenet/HNCP (the planned AHCP successor) is IPv6-first.

**Why we diverge:** the project's UX premise — *"my laptop is `10.42.1.50`, my printer is `printer.mesh`"* — is built on memorable addresses. Home/office IoT devices still have buggy IPv6 support in 2026. The novel work here is the symmetric coordination layer, not the protocol stack; running v4-only lets us focus that work without IPv6 corner-case maintenance.

**Cost:** out of step with the mesh-routing research community. Mitigated by making the data model IP-version-agnostic (`LeaseEntry.ip` is `std::net::IpAddr`, not `Ipv4Addr`) so v6 is additive when added.

### 3.4 CRDT for coordination, not routing

**Diverges from:** an earlier draft of mjolnir-mesh itself, which tried to use the CRDT as a routing table (`/routes/{subnet}` with `via_node_id` and TTLs).

**Why we converged with prior art:** Babel exists, ships in OpenWrt, has 15+ years of validation, handles loop-free reconvergence in seconds, and has explicit support for non-multicast (tunnel) interfaces. Reinventing this is a multi-year project for no user-visible win.

**See:** [babel-routing.md](babel-routing.md) for the full integration.

---

## 4. What We Match

### 4.1 Babel for cross-site routing
Adopted from CeroWrt's lead. Same protocol, similar role — routing over wireless/tunneled links between sites.

### 4.2 Separation of configuration and routing
Mirrors AHCP's design: address management and routing are independent protocols. Our `dhcp-crdt` ↔ `babel-routing` split is the same pattern with different mechanics.

### 4.3 dnsmasq as the DHCP/DNS frontend
Standard OpenWrt choice. We don't reinvent the local-protocol layer. (Shipped integration is even lighter than this doc designed: the daemon reconciles UCI — `network.lan.ipaddr` — to the claimed /24 and restarts dnsmasq via init.d, never SIGHUP. The `dhcp-hostsfile`/`addn-hosts` feed is the planned `e21` lease/DNS lane.)

---

## 5. What Doesn't Exist Anywhere Yet

The contribution of mjolnir-mesh, beyond the integration work:

1. **Symmetric multi-writer DHCP across a mesh with CRDT-driven conflict resolution.** Nobody does this — prior art either has a single writer or partitions the IP pool. Our deauth-on-conflict path is novel enough to be worth a paper if anyone cared to write one.

2. **CRDT-replicated cross-mesh service directory bound to device lease lifecycle.** Existing mesh service discovery is mDNS-reflector-based and fragile. Our gossiped `/services/` namespace with `host_mac` tying is a cleaner model.

3. **Iroh NodeId as the cross-NAT identity substrate.** Existing mesh protocols assume L2 adjacency or pre-configured IP peering. Iroh gives us a stable cryptographic identity that traverses NAT, which is what makes "your home router and the conference WiFi join the same mesh" feasible without VPN-server provisioning.

The first two are explicit design contributions; the third is leverage from the Iroh dependency.

(One caveat to the "nobody does this" framing: the *CRDT-for-mesh-state* idea is not ours alone — LibreMesh independently built one for the same shared-state problem. See §6 for what that does and does not concede.)

---

## 6. LibreMesh shared-state — a CRDT we converged on independently

[LibreMesh](https://libremesh.org/) is the most-deployed OpenWrt-based community-mesh firmware. Buried in its package set is `shared-state` — and by its own description it is a CRDT for exactly the problem our `dhcp-crdt` solves: keep host / lease / service state consistent across a leaderless mesh, then feed it to dnsmasq. Two projects, no shared lineage, the same answer. **That convergence is the point of this section; the differences are the interop story.**

### 6.1 What `shared-state` is

`shared-state` is a CRDT daemon: JSON-structured, owner-keyed, eventually-consistent. The docs state it "ensures data consistency across nodes without a central authority or lock mechanism; nodes exchange data directly," and each node stores its own info about itself and its links. That is the same shape as ours — owner-keyed entries, no leader, eventual convergence — expressed in JSON over the routed network instead of postcard over gossip.

It ships as a family of typed instances:

| LibreMesh instance | Carries | Our analogue |
|---|---|---|
| `shared-state-dnsmasq_hosts` | host / name records synced into dnsmasq | `/dns/{hostname}` → `addn-hosts` |
| `shared-state-dnsmasq_leases` | DHCP leases | `/devices/{mac}` → `dhcp-hostsfile` |
| `shared-state-bat_hosts` | batman-adv host table | (no direct analogue) |
| `shared-state-babeld_hosts` | babeld peer discovery | roster / gossip address-book |
| `shared-state-nodes_and_links` | mesh topology | Babel + `/subnets/` claims |

`shared-state-async` is a C++20-coroutine reimplementation of the same daemon. Sync peer-discovery rides the routed network via `babeld_hosts` / `bat_hosts`; within the batman-adv L2 cloud, plain mDNS also works — so LibreMesh, like us, keeps mDNS for the flat-L2 case and a routed sync layer for everything beyond it (compare [radio-backhaul-and-discovery.md](radio-backhaul-and-discovery.md)).

### 6.2 The convergence, and where it stops

**The convergence is real, and it is independent validation.** Both projects, with no shared code or design lineage, reached for a CRDT for leaderless host / lease / service state — owner-keyed, eventually-consistent, ultimately dumped into dnsmasq. When two unrelated mesh efforts pick the same primitive for the same problem, that is the strongest available evidence the primitive fits. It also sharpens what §5 actually claims as novel: not "a CRDT for mesh state" (LibreMesh has one too), but the narrower symmetric **multi-writer DHCP with deauth-on-conflict** and a service directory **bound to device lease lifecycle via `host_mac`**. LibreMesh's shared-state is host / name / lease sync, not a per-IP conflict-resolution protocol, and its service discovery leans on mDNS + batman rather than lease-tied service expiry.

**The two CRDTs are wire-incompatible.** LibreMesh syncs shared-state in its own JSON format over the routed network; mjolnir gossips its CRDT over iroh-gossip — QUIC, end-to-end encrypted, NAT-traversing. Different transport, different schema, different merge. There is no path where a LibreMesh node and a mjolnir node replicate the same CRDT. (This is also the one place our §3.2 critique of mDNS reflectors does *not* land: LibreMesh isn't reflector-based — it built a CRDT instead — it is just not *our* CRDT.)

**They meet at dnsmasq.** Both ends ultimately write the same two dnsmasq integration points — `addn-hosts` and `dhcp-hostsfile`. So the cheapest possible interop is not protocol bridging at all: it is a dnsmasq hostsfile exchange at a single gateway node that sits in both meshes. No JSON↔postcard translator, no shared gossip topic — just the two files copied across one seam.

### 6.3 The addressing inversion (context)

LibreMesh's surrounding stack is, in one revealing respect, the inverse of ours. Its routing historically defaults to **bmx7** (an IPv6 distance-vector protocol), with **babeld** as a selectable alternative; 2025 Freifunk/GSoC work is moving the default toward native **babeld** plus an **L3-only (no-batman) profile** — which is the configuration that could actually share a routing domain with us. At L2 it runs **802.11s by default with batman-adv (`bat0`)** layered on top; we run 802.11s *without* batman.

Addressing is the sharpest contrast. LibreMesh derives both a ULA `/64` and an IPv4 `/16` from `hash(ap_ssid)` — the address block is **per-cloud / shared**, and a node's identity lives only in the host bits (from its MAC). It also exposes `anygw`, a shared anycast gateway IP+MAC (a distributed default gateway). mjolnir does the **inverse**: each node derives its own address from `blake3(node_id)` — **per-node**, not per-cloud. The two addressing philosophies do not compose; you renumber one side, you do not merge them.

**The full routing / addressing / discovery interop breakdown** — which babeld version and interface types must match, the `10.<hash(ap_ssid)>.0.0/16` collision guard, where the meshes physically join, whether to dual-stack IPv6, and the prioritized "minimum viable interop vs full interop" list — **lives in bead `mjolnir-mesh-0vc` (Investigate LibreMesh interop), not here.** This section's job is the prior-art framing and the CRDT-convergence story; the how-to is tracked there.

---

## 7. Open Reading

- **Babel RFC 8966** (the protocol).
- **HNCP RFC 7788** (Homenet — the IPv6-first AHCP successor; worth understanding before our v6 work).
- **CeroWrt Mesh wiki** (above).
- **Freifunk / LibreMesh** — community-mesh projects built on batman-adv. We're explicitly not using batman (L2 protocol, less observable than L3). LibreMesh additionally ships a CRDT (`shared-state`) for host / lease / service state — the closest prior art to our `dhcp-crdt`; see §6.
- **B.A.T.M.A.N.-V** — modernized batman with Babel-like properties; we still chose Babel for the broader OpenWrt tooling support.

---

## 8. References

**Internal:**

- [babel-routing.md](babel-routing.md) — our Babel integration spec
- [dhcp-crdt.md](../archive/network-coordination/dhcp-crdt.md) — CRDT data model (archived; subnet-claim lane shipped, lease/DHCP lane is bead `e21`)
- [network-architecture.md](network-architecture.md) — cross-site topology
- [mesh-network-coordination.md](../archive/network-coordination/mesh-network-coordination.md) — original overall architecture (archived, superseded)
- [radio-backhaul-and-discovery.md](radio-backhaul-and-discovery.md) — radio L2 / multi-hop discovery
- bead `mjolnir-mesh-0vc` — full LibreMesh routing / addressing / discovery interop breakdown (§6)

**LibreMesh `shared-state` (§6):**

- shared-state overview — https://libremesh.org/packages/shared-state.html
- shared-state-async (C++20 reimplementation) — https://github.com/libremesh/lime-packages/tree/master/packages/shared-state-async
- shared-state-dnsmasq_hosts — https://libremesh.org/packages/shared-state-dnsmasq_hosts.html
- LibreMesh config / addressing (`lime-example.txt`) — https://github.com/libremesh/lime-packages/blob/master/packages/lime-docs/files/www/docs/lime-example.txt
- 2025 native-babeld / L3-only work (Freifunk/GSoC final report) — https://blog.freifunk.net/2025/08/25/final-report-simplify-libremesh-and-get-it-closer-to-openwrt/

**Unicast DNS-SD / the translator pattern (§3.2):**

- RFC 6763, DNS-Based Service Discovery — defines DNS-SD over unicast as well as multicast — https://www.rfc-editor.org/rfc/rfc6763
- Wide-Area Bonjour / DNS-SD over unicast DNS — https://www.dns-sd.org/
- OpenThread border router: SRP server + mDNS advertising proxy — https://openthread.io/guides/border-router/services
