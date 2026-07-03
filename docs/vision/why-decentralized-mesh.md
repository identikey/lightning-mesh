# Why Decentralized Mesh Networking

> Companion doc: [Philosophical outcomes of the architecture](philosophical-outcomes.md)
> — what the "heterogeneous link islands stitched by L3 routing" design buys, and how
> it places Lightning Mesh in the lineage of Cerf & Kahn. The public talk narrative
> lives in [docs/talk/dweb-2026-lightning-mesh.md](../talk/dweb-2026-lightning-mesh.md).

## The Problem

Today's networks are centralized by default. Your home has one router. Your office has managed switches and a network admin. Events rely on a single AP or a vendor-locked mesh system (Ubiquiti, eero, Google WiFi). If the central device fails, the network fails.

This centralization is:
- **Fragile**: Single point of failure
- **Inflexible**: Can't add capacity by just plugging in another router
- **Vendor-locked**: Mesh systems only work with their own hardware
- **Opaque**: You don't control the software, the routing, the DNS

For events like DWEB (Decentralized Web), the irony is sharp: a movement about decentralization runs on centralized infrastructure.

## The Vision

**Any router can join the mesh. Any router can leave. The network keeps working.**

Lightning Mesh (formerly mjolnir-mesh) turns commodity OpenWrt routers (like $60 GL.iNet travel routers) into nodes in a self-organizing mesh network. Plug in a router, it joins. Unplug it, the mesh adapts. No configuration. No central controller. No vendor lock-in.

What this enables:

**1. Pop-up networks for events**
DWEB conference, 200+ attendees. Organizers bring 10 routers, plug them in around the venue. Within seconds, a unified network forms. Same SSID, shared IP space, devices roam seamlessly as people move between rooms. Anyone can bring an extra router to boost coverage in a corner — just plug it in.

**2. Resilient home networks**
Your main router, a travel router, maybe a third for the garage. All coordinated. If your main router dies, the others keep serving. When you take the travel router on a trip, the home mesh shrinks gracefully. When you come back, it rejoins and syncs.

**3. Community networks**
A neighborhood mesh where each household runs a node. Shared local services — file servers, wikis, game servers — discoverable by hostname. Global connectivity via Iroh when nodes have internet access, local-only operation when they don't.

**4. Global roaming**
Your travel router connects to your home mesh via Iroh from anywhere in the world. Devices on your travel router can reach services at home by hostname. Your home devices can reach your travel network. One mesh, spanning the globe, encrypted end-to-end.

**5. An appliance you own — and can manage from anywhere**
A node is a box you plug in, not a system you administer. It joins whatever is around it (802.11s locally, Iroh over any internet egress), and from then on **the mesh manages itself over itself**: every node has a stable overlay address derived from its cryptographic identity, so its owner — authorized by key (IdentiKey), not by being in the room — can configure it and ship it updates from anywhere. Updates apply detached with a health gate and automatic rollback, so a bad push can't strand a box behind a dead radio; plugging in an ethernet cable is the recovery of last resort, never the routine. See [node operations](../deploy/node-operations.md).

## Why Now

Three things that didn't exist 5 years ago make this feasible:

**Iroh (QUIC mesh networking)**: NAT traversal, encrypted connections, peer discovery, relay fallback — all built in. Previously you'd need a VPN server, manual config, port forwarding. Iroh makes global P2P mesh networking as easy as connecting to a server.

**Affordable OpenWrt hardware**: GL.iNet routers ($30-80) run full Linux with 128MB+ RAM, USB, WiFi 6/7. Powerful enough to run a Rust mesh daemon alongside dnsmasq. Available worldwide. No vendor lock-in.

**CRDTs**: Conflict-free replicated data types give us eventually-consistent shared state without consensus protocols. No leader election, no Raft, no Paxos. Just merge. Perfect for a P2P network where nodes come and go.

## What Makes This Different

**vs. Traditional mesh WiFi (eero, Ubiquiti, Google WiFi)**:
- Those systems have a "controller" node. Lightning Mesh doesn't.
- Those systems only work with their own hardware. Lightning Mesh works with any OpenWrt device.
- Those systems don't cross the internet. Lightning Mesh does, via Iroh.

**vs. VPNs (WireGuard, Tailscale)**:
- VPNs are point-to-point or hub-and-spoke. Lightning Mesh is full mesh.
- VPNs don't coordinate DHCP or DNS across nodes. Lightning Mesh does.
- VPNs require manual peer configuration. Lightning Mesh self-organizes.
- Tailscale is close in spirit but requires their coordination server. Lightning Mesh is fully self-hosted.

**vs. Mesh networking protocols (B.A.T.M.A.N., OLSR, babel)**:
- Those operate at Layer 2/3 — they route packets between nodes but don't coordinate network services.
- Lightning Mesh coordinates DHCP, DNS, service discovery, and routing as a unified system.
- Those protocols are designed for ad-hoc wireless links. Lightning Mesh works over any transport Iroh supports (direct, relayed, internet).

## Why Flat Mesh Networks Hit a Wall (and This One Doesn't)

Most "just works" mesh systems glue every node into one big flat network — a single Layer-2
cloud where every device shares one broadcast domain. It's simple, and it's fine for a dozen
nodes. But it carries a hidden ceiling: every "who has this address?" (ARP), every DHCP request,
every "who has this name?" lookup is flooded to *the entire mesh*. Double the network and you
double that background chatter for everyone. Past a certain size the broadcast noise — and the
risk of a single loop turning into a storm that takes the whole thing down — becomes the limit.
This is the wall flat meshes (batman-adv, the classic LibreMesh cloud) run into.

This design doesn't have that cloud. Each router owns its own small slice of address space and
**routes** between them instead of bridging everything together. The consequences:

- **Local noise stays local.** Your phone's chatter reaches your router and stops there — it is
  never broadcast across the whole network. The background load on any node doesn't grow as the
  mesh grows.
- **The map stays small.** Routers tell each other "I own this block," not "here is every
  device on me." The routing table grows with the number of *routers*, not the number of phones
  and laptops.
- **Loops can't storm.** The routing math (Babel) is loop-free by construction, and with no
  giant shared segment there's nothing for a stray packet to circle inside. The two things that
  make big flat meshes fall over — broadcast storms and forwarding loops — are designed out, not
  patched over.

The trade-off is honest: pushing this structure out to every node means roaming between them is
something we solve deliberately (fast Wi-Fi handoff, plus carrying your address with you) rather
than something a flat network gives away for free. We think that's the right trade — it's what
lets the same design run a three-router apartment and a thousand-router festival without changing
shape.

## The Architecture in Brief

```
┌─────────────────────────────────────────┐
│  Applications & Devices                  │
│  (standard TCP/IP — no changes needed)   │
├─────────────────────────────────────────┤
│  dnsmasq (DHCP + DNS per router)         │
│  Serves local devices, reads mesh state  │
├─────────────────────────────────────────┤
│  mjolnir-mesh daemon                     │
│  CRDT store ←→ gossip replication        │
│  Hostsfile sync, conflict resolution     │
│  Service discovery, route management     │
├─────────────────────────────────────────┤
│  Iroh node (QUIC mesh)                   │
│  NAT traversal, encryption, tunneling    │
└─────────────────────────────────────────┘
```

Every router runs this stack. No special roles. No leaders. Fully symmetric.

## One Network on Any Substrate

Look at that stack again, bottom to top. Most mesh products *are* their radio layer — an eero mesh is eeros talking to eeros over eero's radios; pull out the radio and there is no network. Lightning Mesh inverts that. **The network is the Layer-3 overlay — identity, routing, and coordination — and it rides on top of whatever link happens to be there: an 802.11s radio mesh, a plain WiFi access point, an Ethernet cable, a fiber run, or the open internet.** The radio is plumbing. The network is the layer above it.

That sounds like a technicality. It is the most consequential design decision in the project, and here is what it buys:

**Any hardware, one mesh, as equals.** A $40 OpenWrt box, a closed-driver MikroTik that cannot even speak 802.11s, a Raspberry Pi with a USB WiFi dongle, a laptop, a virtual machine — all join the *same* mesh on equal footing, because they all speak the L3 layer. Their links can be completely different, and one node can have no radio at all and join over Ethernet; those differences live *below* the abstraction. You are never locked to one vendor's mesh tech, and you can grow a network out of whatever hardware people already have.

**Encryption that does not trust the wire.** This is the part that matters most for a network strangers might share. Security lives at L3: every node is a cryptographic identity (a public key), and every connection is end-to-end encrypted between those identities. A packet can cross a neighbor's router, a closed-firmware box you did not build, or a café's internet connection, and everyone forwarding it **sees only ciphertext** — they move it without being able to read it. Compare ordinary WiFi security (WPA/WPA3): it encrypts each radio *hop*, then hands plaintext to every router in the middle. Link encryption trusts every forwarder; L3 end-to-end encryption trusts none of them. That is what lets you build a *trusted* network on top of an *untrusted* substrate — and it is why mixing your hardware with someone else's is safe.

**Meshes can merge.** Because the unifying layer is just routing plus identity, two independently-built meshes can fuse into one — even other community-mesh projects that already speak the same routing language, like LibreMesh — by linking them at a single node and letting the routing layer stitch the address spaces together. The radios do not have to match; the L3 layer does the unifying.

**The one place the abstraction leaks: legacy clients.** A device that speaks the L3 layer — an app with the networking built in, or another router — gets all of this for free and does its own encryption, putting no load on the little routers. But a *normal* device, a phone with a web browser hitting `http://projector.mesh`, knows nothing about node-ids or end-to-end tunnels. For those, a **gateway** bridges the legacy world into the mesh: plain HTTP to the browser on one side, the L3 layer to the service on the other. That gateway is the single spot where substrate-independence stops being free — so it is worth building well, because "any normal device, any browser, just works" is most of what people actually do.

## The Power of Service Discovery

Beyond basic networking, the mesh becomes a platform for services:

- A Raspberry Pi running a wiki joins the mesh → `wiki.mesh` is resolvable from any device on any router
- Someone starts a game server → `minecraft.mesh:25565` appears on every router's DNS
- A projector with a web interface → `projector.mesh` accessible from any phone in the venue
- Mjolnir VMs join the mesh too → spin up a service in a VM, it's instantly discoverable

This turns a group of routers into a **decentralized application platform**, not just a network.

## Relationship to Mjolnir

Lightning Mesh is part of the Mjolnir ecosystem. Mjolnir provides:
- **MicroVMs** with Iroh built into their network stack
- **MCP (Model Context Protocol)** for AI agent interaction
- **BTRFS snapshots** for instant VM cloning

The mesh layer means VMs can be spawned on any node and be instantly reachable by any device on the mesh. A Mjolnir cluster distributed across mesh routers becomes a decentralized compute platform — VMs migrate, routers come and go, the mesh adapts.

## Who This Is For

- **Event organizers** who need reliable, flexible networking without enterprise infrastructure
- **Decentralization advocates** who want their network to match their values
- **Home labbers** who want seamless multi-router setups
- **Community network builders** working on local mesh infrastructure
- **Developers** building P2P applications who need a networking layer that "just works"

## Current Status

*Updated 2026-07-02.* The CRDT data plane is shipped and field-validated on a real four-router OpenWrt fleet. `mjolnir-meshd` (`crates/mjolnir-mesh`) runs natively on each router: 802.11s radio backhaul (`br-mesh`), a routed client `/24` per node claimed out of `10.42.0.0/16` via CRDT (first-writer-wins, gossip-converged over Iroh), supervised babel routing between nodes, a derived `10.254.<blake3(node_id)>/16` backhaul-and-management overlay, and a single overlay TUN carrying cross-site Iroh traffic. Client traffic is validated to the internet and cross-LAN. Deploys are in-band with health gates and automatic rollback.

One lesson from the field is worth recording: a local mesh routes most efficiently over its own L2 island. The Iroh L3 overlay is for internet hops and as a first-hop security gateway — not the local fast path.

This is a deliberate stepping stone, not the end state. Next: the gossip address book and multi-hop discovery (bead `0yb`), the service-mesh architecture pass — broadcast peer/service discovery and conflict resolution (bead `e21`), and the IPv6-vs-IPv4 addressing decision, since IPv4 `/24` claims hand out a limited resource (bead `bsa`).

## References
- Network architecture: docs/network-coordination/network-architecture.md
- Technical overview (archived): docs/archive/network-coordination/mesh-network-coordination.md
- CRDT design (archived): docs/archive/network-coordination/dhcp-crdt.md
- dnsmasq integration (archived): docs/archive/network-coordination/dnsmasq-integration.md