# mjolnir-mesh

A decentralized router mesh that turns commodity OpenWrt hardware into a
self-organizing peer-to-peer network. Any router can join. Any router can
leave. The network keeps working. No controller, no leader, no vendor.

Built on [Iroh](https://www.iroh.computer/) (QUIC + NAT traversal + identity)
and CRDTs (conflict-free replicated state for shared DHCP/DNS/service
discovery). Plug in a $30–$80 OpenWrt router; it joins the mesh. Same SSID
across all of them. Devices roam. Services discoverable by `.mesh` hostname.
Iroh tunnels stitch the local mesh to remote nodes across the internet when
they have connectivity, and the mesh keeps functioning when they don't.

See [docs/vision/why-decentralized-mesh.md](docs/vision/why-decentralized-mesh.md)
for the full motivation, [docs/vision/mjolnir-integration.md](docs/vision/mjolnir-integration.md)
for how this composes with the broader Mjolnir microVM platform.

## What this repo contains

```
┌──────────────────────────────────────────────────────────────┐
│  mjolnir-node                                                 │
│  The daemon: mesh membership, gossip, room/peer management,  │
│  CRDT state, transport setup                                  │
├──────────────────────────────────────────────────────────────┤
│  mjolnir-audio                                                │
│  Real-time multi-peer voice: Opus capture/encode/decode,      │
│  PLC backends (FARGAN today, neural bridge engine planned),   │
│  jitter-buffer-fed mixer driving cpal output                  │
├──────────────────────────────────────────────────────────────┤
│  mjolnir-media                                                │
│  Transport-agnostic media primitives: jitter buffer, the     │
│  Recover trait (decode + conceal seam), SelfHealingBuffer    │
├──────────────────────────────────────────────────────────────┤
│  mjolnir-moq                                                  │
│  Media-over-QUIC broadcast scaffolding for one-to-many       │
│  streaming (distinct from the direct bidi audio path)         │
└──────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌──────────────────────────────────────────────────────────────┐
│  Iroh — QUIC transport, NodeId identity, NAT traversal       │
└──────────────────────────────────────────────────────────────┘
```

| Crate                                    | Role                                                                 |
|------------------------------------------|----------------------------------------------------------------------|
| [`mjolnir-node`](crates/mjolnir-node)    | The mesh daemon binary — membership, rooms, gossip, transport wiring |
| [`mjolnir-audio`](crates/mjolnir-audio)  | Voice pipeline — Opus codec, PLC backends, mixer, capture/playback   |
| [`mjolnir-media`](crates/mjolnir-media)  | Transport-agnostic media primitives — jitter, `Recover`, self-healing buffer |
| [`mjolnir-moq`](crates/mjolnir-moq)      | Media-over-QUIC broadcast transport (one-to-many)                     |

## The audio sub-problem

A decentralized mesh is exactly the kind of network where real-time voice
will *break* the way a centralized network won't. Packets take multiple
paths. Paths fail and reconverge. A router at the edge of the room briefly
loses its uplink and rejoins via a different neighbor. The wire is
adversarial in ways that VoIP designed for a single broadband uplink never
sees.

The audio layer is built to keep voice intelligible under exactly those
conditions. Three problems, three answers in this repo:

1. **Reorder and jitter under multipath delivery.** Solved by a
   sequence-keyed jitter buffer with adaptive depth
   ([`mjolnir-media`](crates/mjolnir-media), design in
   [`docs/architecture/self-healing-jitter-buffer.md`](docs/architecture/self-healing-jitter-buffer.md)).
2. **Short packet losses (< 80 ms).** Solved by Opus's built-in decoder
   PLC — heuristic LPC today, FARGAN neural PLC once linked against
   libopus 1.5+ — exposed behind a `Recover` trait so the implementation
   can evolve without touching call sites
   ([`mjolnir-audio::OpusPlc`](crates/mjolnir-audio/src/conceal.rs)).
3. **Long burst losses (200 ms – multi-second), DRED redundancy, and
   out-of-order packets from a future moment.** Designed as a streaming
   speech LM with fill-in-the-middle training, a speculative output
   buffer whose depth tracks model entropy, and a metadata sidechannel
   that surfaces uncertainty to the client. See
   [`docs/architecture/neural-bridge-plc.md`](docs/architecture/neural-bridge-plc.md).
   The deployment-focused survey of existing options that motivated this
   design is in
   [`docs/research/audio-models-for-neural-plc/synthesis.md`](docs/research/audio-models-for-neural-plc/synthesis.md).

Voice in a mesh is not just "VoIP that happens to be P2P." It is the
acid test for whether the rest of the architecture — multipath routing,
self-healing paths, decentralized membership — actually delivers
something a person can hear cleanly. The PLC work is where the rubber
meets the road.

## How it composes with the broader vision

mjolnir-mesh provides the *networking*. The complementary projects
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
- [Why decentralized mesh networking](docs/vision/why-decentralized-mesh.md)
- [Mjolnir + mjolnir-mesh integration](docs/vision/mjolnir-integration.md)

### Architecture
- [Network architecture (CRDT, routing, subnet allocation)](docs/network-coordination/network-architecture.md)
- [Radio backhaul & multi-hop discovery decisions](docs/network-coordination/radio-backhaul-and-discovery.md)
- [DHCP CRDT design](docs/network-coordination/dhcp-crdt.md)
- [dnsmasq integration](docs/network-coordination/dnsmasq-integration.md)
- [P2P resilience](docs/network-coordination/p2p-resilience.md)
- [Self-healing jitter buffer](docs/architecture/self-healing-jitter-buffer.md)
- [Neural bridge PLC design](docs/architecture/neural-bridge-plc.md)
- [Mesh network coordination overview](docs/network-coordination/mesh-network-coordination.md)

### Deploy & operations
- [Node operations: management plane, in-band updates, OTA](docs/deploy/node-operations.md)
- [OpenWrt node deploy runbook](deploy/openwrt/README.md)

### Research
- [Audio models for neural PLC — deployment-focused synthesis](docs/research/audio-models-for-neural-plc/synthesis.md)

## Status

In active development. A real fleet exists: `mjolnir-meshd`
(`crates/mjolnir-mesh`) runs natively on OpenWrt MT7981 routers over an
802.11s backhaul, with babel routing, derived `10.254.x/16` overlay
addressing, and a single-overlay-TUN data plane for cross-site traffic
(`buw`). Nodes are installed and updated **in-band** — staged payloads
applied detached with health-gated auto-rollback, no ethernet required
(see [node operations](docs/deploy/node-operations.md)). The
CRDT/DHCP/DNS coordination layer is designed and partially implemented;
gossip-propagated peer announcement and IdentiKey-authorized remote
management/OTA are the active trajectory. The audio pipeline
(`mjolnir-audio` + `mjolnir-media`) exists and runs; the neural bridge
PLC engine is a forward-looking design (v2 lane) sequenced after the
FARGAN/DRED production answer.

## Building

This is a Cargo workspace.

```bash
cargo build --workspace
cargo test --workspace
```

The mesh daemon binaries are `mjolnir-node` (desktop/VM) and
`mjolnir-meshd` (`crates/mjolnir-mesh`, the OpenWrt router daemon —
cross-built static with `deploy/openwrt/build.sh` and pushed with
`deploy/openwrt/install-node.sh`).

## License

See `Cargo.toml` workspace metadata for license information.
