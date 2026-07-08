# Lightning Mesh

> **Naming:** Lightning Mesh is the project's public name. The repository,
> crates, and binaries keep the `mjolnir-` prefix (`mjolnir-meshd`, etc.), and
> the overlay interface is `mjolnir0`.

A decentralized router mesh that turns commodity OpenWrt hardware into a
self-organizing peer-to-peer network. Any router can join. Any router can
leave. The network keeps working. No controller, no leader, no vendor.

Built on [Iroh](https://www.iroh.computer/) (QUIC + NAT traversal + identity)
and CRDTs (conflict-free replicated state for shared network coordination).

**The North Star:** plug in a $30–$80 OpenWrt router; it joins the mesh. Same
SSID across all of them. Devices roam freely. Services broadcast on the local
mesh, discoverable by `.mesh` hostname, reachable locally *and* over the
internet via the iroh L3 overlay. Censorship-resistant, truly peer-to-peer, no
implicit authority, ad hoc join/leave, scalable — one of the most powerful
upgrades to the internet. Those aren't marketing adjectives; they're hard
requirements the design is held to.

**What's real today** (field-validated on a four-router fleet): each node owns
its own routed client `/24`, claimed via CRDT and routed between nodes with
babel over an 802.11s backhaul — client traffic flows to the internet and
across LANs. Client L2 is deliberately *not* bridged across nodes (broadcast
containment is what lets this scale), which means the roaming and
service-discovery experience is the next phase, not a shipped feature. The
current data plane is the stepping stone that phase builds on.

See [docs/vision/why-decentralized-mesh.md](docs/vision/why-decentralized-mesh.md)
for the full motivation, [docs/vision/mjolnir-integration.md](docs/vision/mjolnir-integration.md)
for how this composes with the broader Mjolnir microVM platform.

## What this repo contains

```
┌──────────────────────────────────────────────────────────────┐
│  mjolnir-mesh → mjolnir-meshd                                 │
│  THE product: the OpenWrt router daemon. 802.11s backhaul,   │
│  CRDT subnet claims, babel routing, derived overlay          │
│  addressing, overlay TUN for cross-site iroh traffic         │
├──────────────────────────────────────────────────────────────┤
│  mjolnir-meshctl → meshctl                                    │
│  Operator-side RouterOS reconciler                            │
└──────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌──────────────────────────────────────────────────────────────┐
│  Iroh — QUIC transport, NodeId identity, NAT traversal       │
└──────────────────────────────────────────────────────────────┘
```

| Crate                                          | Binary          | Role                                                                 |
|------------------------------------------------|-----------------|----------------------------------------------------------------------|
| [`mjolnir-mesh`](crates/mjolnir-mesh)          | `mjolnir-meshd` | **The deployed OpenWrt router daemon** — CRDT, gossip, babel, overlay TUN |
| [`mjolnir-meshctl`](crates/mjolnir-meshctl)    | `meshctl`       | Operator-side RouterOS reconciler                                     |

> **Voice/media lives elsewhere.** The audio side-quest (Opus pipeline, PLC
> backends, jitter buffer, Media-over-QUIC scaffolding, and the desktop voice
> node) was extracted to its own repository, `mjolnir-voice`. This repo is the
> router mesh only.

## How it composes with the broader vision

Lightning Mesh provides the *networking*. The complementary projects
provide compute and access:

- **Mjolnir** runs microVMs on cluster nodes, each with an Iroh endpoint
  that can join the mesh natively — VMs become discoverable by `.mesh`
  hostname across every router. See
  [`docs/vision/mjolnir-integration.md`](docs/vision/mjolnir-integration.md).
- **`vm.worldtree.network`** is a complementary HTTP-to-Iroh gateway for
  reaching VMs from any browser on the open internet — covered in the
  same integration doc.
- **MCP (Model Context Protocol)** is the control plane AI agents use to
  spawn VMs and coordinate work; the mesh provides the discovery layer
  that lets multiple agents see each other's spawned resources without
  ticket exchange.

The shape: one mesh of routers, one mesh of compute nodes, one DNS
namespace (`.mesh`), one transport (Iroh). Local-first when there's no
internet; globally connected when there is. Voice, video, services, VMs,
and AI agents all coexist on the same fabric.

## Documentation

### Vision
- [Philosophical outcomes of the architecture](docs/vision/philosophical-outcomes.md)
- [Why decentralized mesh networking](docs/vision/why-decentralized-mesh.md)
- [Mjolnir + Lightning Mesh integration](docs/vision/mjolnir-integration.md)

### Public / talk
- [DWeb talk source material](docs/talk/dweb-2026-technical-arc.md) — the
  public narrative; eventually the basis for website documentation

### Architecture
- [Network architecture (CRDT, routing, subnet allocation)](docs/network-coordination/network-architecture.md)
- [Radio backhaul & multi-hop discovery decisions](docs/network-coordination/radio-backhaul-and-discovery.md)
- [P2P resilience](docs/network-coordination/p2p-resilience.md)
- Archived early designs: [mesh coordination overview](docs/archive/network-coordination/mesh-network-coordination.md),
  [DHCP CRDT](docs/archive/network-coordination/dhcp-crdt.md),
  [dnsmasq integration](docs/archive/network-coordination/dnsmasq-integration.md)

### Deploy & operations
- [Node operations: management plane, in-band updates, OTA](docs/deploy/node-operations.md)
- [OpenWrt node deploy runbook](deploy/openwrt/README.md)

### Research
- [MANET dynamic addressing](docs/research/manet-dynamic-addressing/README.md)
- [OpenWrt mesh hardware](docs/research/openwrt-mesh-hardware/synthesis.md)
- [Asset pipeline & version control](docs/research/asset-pipeline-version-control/synthesis.md)

## Status

**The data plane is complete and field-validated** (July 2026) on a
four-router OpenWrt fleet: `mjolnir-meshd` (`crates/mjolnir-mesh`) runs
natively over an 802.11s backhaul (`br-mesh`); each node claims a routed
client `/24` out of `10.42.0.0/16` via CRDT (first-writer-wins,
gossip-converged over iroh); supervised babeld routes between nodes; a
derived `10.254.<blake3(node_id)>/16` overlay carries backhaul and
management; a single overlay TUN (`mjolnir0`) carries cross-site iroh
traffic. Client traffic is validated end-to-end — to the internet and
cross-LAN. Nodes are installed and updated **in-band** — staged payloads
applied detached with health-gated auto-rollback, no ethernet required
(see [node operations](docs/deploy/node-operations.md)).

A key lesson from deployment: a local mesh routes most efficiently over
its own L2 island; the iroh L3 overlay earns its keep on internet hops
and as a first-hop security gateway, not as the local fast path.

Next up (tracked in beads): the gossip address book / multi-hop discovery
(`0yb` — derived-address seeding is the first stone laid), the
service-mesh architecture pass (`e21` — broadcast peer/service discovery
plus conflict resolution), and the IPv6-vs-IPv4 addressing question
(`bsa` — IPv4 `/24` claims hand out a limited resource). Known gaps:
babeld SIGHUP respawn (`2zz`) and the validation matrices (`b9a`, `0pv`).

## Building

This is a Cargo workspace.

```bash
cargo build --workspace
cargo test --workspace
```

Binaries: `mjolnir-meshd` comes from `crates/mjolnir-mesh` (the OpenWrt
router daemon — cross-built static aarch64 with `deploy/openwrt/build.sh`
and pushed with `deploy/openwrt/install-node.sh`); `meshctl` comes from
`crates/mjolnir-meshctl`.

## License

Lightning Mesh is **dual-licensed**:

- **[GNU AGPL v3.0 or later](LICENSE)** — free for everyone. Community mesh
  networks, researchers, hobbyists, and any deployment willing to comply with
  the AGPL (including its network-use source-sharing clause, §13) can use,
  modify, and redistribute Lightning Mesh at no cost, forever. Copyleft here is
  a feature: it keeps the mesh un-enclosable.
- **[Commercial license](COMMERCIAL-LICENSE.md)** — for embedding Lightning
  Mesh in a proprietary appliance or hosted service without AGPL
  source-disclosure obligations. Contact **duke@identikey.io**.

Contributions are accepted under the **[Contributor License Agreement](CLA.md)**,
which is what makes the dual-licensing model possible — the CLA bot will ask you
to sign on your first pull request. See **[CONTRIBUTING.md](CONTRIBUTING.md)** to
get started, and [`NOTICE`](NOTICE) for third-party components (notably `iroh`,
MIT/Apache-2.0, and `babeld`, run as a separate process).
