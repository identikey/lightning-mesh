# Radio Backhaul & Multi-Hop Discovery — Design Notes & Decisions

Status: living design note. Captures the 2026-06-23 hardware/radio decision, the
tradeoffs behind it, and the protocol work it implies. See beads
`mjolnir-mesh-b1d` (wireless backhaul) and the multi-hop-discovery bead.

## TL;DR (decision as of 2026-06-24)

- **Guiding principle: the L3 overlay (iroh + babeld + CRDT) is the product, and it
  must run on heterogeneous hardware. The radio / L2 layer is whatever each node can
  do.** The L3 routing is the invariant; everything below it is interchangeable.
  This *is* the non-authoritative, symmetric, runs-anywhere thesis.
- **New nodes: whole OpenWrt mt76 boards** (e.g. OpenWrt One / GL-MT3000 — USB-C,
  self-enclosed, ~$80–90) running **real 802.11s mesh** backhaul + a client AP +
  the overlay **natively**. Real mesh = multi-link, mobility/churn-robust, and a
  flat L2 within the island (mDNS works there).
- **Existing MikroTiks: kept as an AP/STA segment.** `wifi-qcom` can't do 802.11s;
  interop between the AP/STA segment and the mt76 802.11s island is a **known cost**
  (gateway node + L3 routing), accepted deliberately — the L3 overlay unifies them.
- **Heterogeneous fleet ⇒ no universal flat L2 ⇒ discovery must be radio-agnostic**
  (a gossip-based address book), not flat-mDNS-dependent. This makes multi-hop
  discovery (`mjolnir-mesh-0yb`) **core, not optional** — it's what makes "runs on
  any hardware" actually true.
- **Decision history (why, for the record):** considered staying all-MikroTik
  AP/STA — protocol symmetry is satisfiable there (per-link AP/STA is transport
  plumbing, like TCP `connect()`/`accept()`, not a hierarchy), but single-uplink
  AP/STA is fragile under mobility/churn. Moving new nodes to mt76/802.11s restores
  multi-link robustness and a flat L2 within the open island, on open hardware.

## Background: the two layers

| Layer | Role | Symmetry requirement |
|---|---|---|
| **L3 overlay** (iroh + babeld + CRDT) | the actual protocol: addressing, routing, self-healing, multi-hop | **Must be symmetric / non-authoritative** — this is the thesis |
| **Radio backhaul** (WiFi) | transport: carry IP between in-range neighbours | Just needs to provide links; AP/STA labelling is below the protocol |

The contribution is the L3 protocol. The radio is plumbing. Keeping these straight
is what resolves the "designated root violates our principles" tension: a *root AP*
(one node everyone depends on) would be a protocol authority and a single point of
failure — unacceptable. A *per-link AP/STA role* on otherwise-identical nodes is
neither.

## Radio-layer findings (`wifi-qcom`, IPQ-5010 / L23UGSR), 2026-06-23

Sourced from current RouterOS docs (see `mjolnir-mesh-b1d` notes for links):

| Capability | Available? |
|---|---|
| 802.11s / HWMPplus mesh | **No** (docs: "not supported on WiFi interfaces") |
| Legacy `wireless` package on this 802.11ax SoC | **No** (conflicts with `wifi-qcom`, can't run) |
| IBSS / ad-hoc | **No** (not in the `wifi-qcom` mode set) |
| AP, station, station-bridge, station-pseudobridge | **Yes** |
| Concurrent AP + station on one radio | **Yes**, but channel-locked to the AP VIF |
| Station auto-reassoc among same-SSID APs | **Yes** (signal-based; topology-blind) |

**OpenWrt is not a shortcut.** It's the wall the project already hit: no board
port exists for the L23UGSR, MikroTik uses RouterBOOT (brick risk, not U-Boot),
`ath11k` support for the IPQ-5010 + QCN-6102 is incomplete upstream, and even if
it booted, `ath11k` mesh support for these chips is uncertain and IBSS is absent.
High effort, real brick risk, uncertain payoff. Not recommended.

## Channels & radios (the recurring question)

**A "true mesh" does not give you more channels.** Any *single-radio* mesh —
802.11s or AP/STA — is inherently **single-channel**: all peers must share one
frequency to hear each other. What 802.11s adds is symmetric peering + native
multi-hop path selection, not channel diversity.

| Topology | Channels | Notes |
|---|---|---|
| One radio, concurrent AP+STA | 1 (shared by all backhaul + any clients) | simplest; airtime contention grows with nodes/hops |
| Dual radio: dedicated backhaul + client | 1 backhaul + 1 client | **practical sweet spot** on the L23UGSR; isolates backhaul airtime from clients; still one backhaul channel mesh-wide |
| Multi-radio backhaul + channel planning | N | true frequency reuse; complex/expensive; only for large/high-throughput meshes |

So: **backhaul on one radio = one channel, period.** The dual-radio split's value
is isolating client traffic from the backhaul, not multiplying backhaul channels.

## Mixed fleet / interop (planned: real-mesh nodes + AP/STA)

- **L3 (the protocol): interoperates with anything.** It's a Linux process over
  IP — runs identically on a RouterOS container, OpenWrt, a Raspberry Pi, a laptop.
  A MikroTik node and an OpenWrt node in the same mesh interoperate perfectly here.
- **L2 (radio): only same-mode interoperates.** Plain **AP + managed-station is
  standard WiFi and interoperates across vendors**; **802.11s ↔ `wifi-qcom` does
  not**. Rule: don't mix radio *modes* within one radio domain.
- **Hybrid plan** (some open routers running real 802.11s, bridged into the AP/STA
  MikroTik fleet): workable, but needs a **gateway node** that participates in both
  radio domains (e.g. an OpenWrt node doing 802.11s on one radio and AP/STA on
  another, bridging them). The L3 overlay spans the whole fleet regardless.

## Open-source narrative (for the talk)

The protocol is open and containerised; the host is irrelevant. Turn the
closed-driver MikroTik from a wart into a **portability demonstration**: run one
node on an open board (RPi + USB WiFi, or an OpenWrt SBC) in the same AP/STA mesh.
The demo becomes *"a symmetric, non-authoritative protocol, in a container, running
identically across a closed-driver MikroTik and an open Linux node, interoperating
in one mesh."* The closed Qualcomm driver lives below the abstraction.

## Hardware / packaging

- The **NetMetal-ax enclosure** variant is already a weatherproof single unit —
  good for "3D-print a case and throw it in the forest." A *separate* mesh radio
  would break the single-unit goal; a point in favour of the integrated MikroTik.
- MikroTiks are sunk cost (past the return window) — kept, not wasted, since
  protocol symmetry is satisfied on them as-is.
- For forest distance: **2.4 GHz omni** is the better backhaul (range + foliage
  penetration) over 5 GHz; omni is mandatory for a mesh (can't aim a directional at
  all neighbours). Reserve 5 GHz + higher-gain antennas for any fixed long hops.

## Forward: multi-hop discovery — the babeld/mDNS synchronization work

**This is the next real protocol problem, and it's platform-independent.**

Everything validated so far (containers, the switch bench) sits on **one shared L2
segment**: mDNS floods to every node, so every node discovers every other directly,
and iroh dials anyone. **A spread-out forest has no such shared segment.** Nodes
that aren't radio-neighbours aren't on a common L2, so:

- **Flat mDNS only reaches direct neighbours.** Node A discovers B and C (in range),
  not distant D.
- **babeld routes *traffic* multi-hop fine** (A→B→C→D at the IP layer), but A
  *learning how to address* D — the address iroh needs to dial — is unsolved by
  mDNS alone.

**Likely direction:** stop relying on flat mDNS for mesh-wide discovery. Use the
**roster** (known node ids) plus **propagate peer addresses over the CRDT gossip
overlay**, which itself rides the babeld-routed IP network. mDNS stays for
direct-neighbour bootstrap; the gossip layer becomes a mesh-wide, eventually-
consistent **address book** (node id → reachable address), synchronized the same
way subnet claims already are. That synchronization between the link-local
discovery (mDNS) and the routed overlay (babeld + gossip) is the "babeld/mDNS
synchronization protocol" to design and build.

This is squarely the kind of non-authoritative, eventually-consistent, symmetric
mechanism the project is about — a good problem, not a blocker.

## Hardware options for open WiFi 6 mesh nodes (2026-06-23 survey)

For *additional* open, mesh-capable nodes (the MikroTiks stay AP/STA-only). The
overlay runs **natively** on these (no container needed — they're real Linux);
512 MB RAM is enough for native iroh + babeld, tight for containers.

**Driver vs enclosure is the core tradeoff:**
- **mt76 (MediaTek Filogic mt7915/7916/7981/7986)** — best *driver*: full 802.11s
  **and** IBSS, in mac80211 kernelspace (reliable, same path as ath9k), dual-radio
  concurrent. Weak ready-made *outdoor* options.
- **ath11k IPQ6018** — *partial* 802.11s (firmware-path; minor quirks like ignoring
  `mesh_dtim_period`, but babeld doesn't need tight beacon timing). Has ready
  **outdoor PoE WiFi 6 enclosures** off the shelf.
- **IPQ-5010/5018, rtw89, Intel iwlwifi** — not viable for open mesh.

| Pick | Device | SoC / driver | RAM | Outdoor/PoE | 802.11s | Notes |
|---|---|---|---|---|---|---|
| 🥇 | TP-Link EAP610-Outdoor (~$60–80) | IPQ6018 / ath11k | 512 MB | Yes, 802.3at | partial | dual-radio (gateway-capable); OpenWrt merged — pin 24.10 stable; bench-validate mesh |
| 🥈 | TP-Link EAP625-Outdoor HD (~$80–100) | IPQ6018 / ath11k | 512 MB | Yes, **IP67**, PoE+ | partial | detachable antennas (swap high-gain omni for forest NLOS); newer OpenWrt 25.12+ |
| 🥉 | Banana Pi BPI-R3 (+ enclosure) | MT7986 / mt76 | **2 GB** | DIY | **full** | best driver + compute (runs containers); gateway/relay/compute node |
| gw | GL.iNet GL-MT6000 (indoor) | MT7986 / mt76 | 512 MB | No | **full** | indoor bridgehead: 802.11s island ↔ MikroTik AP/STA fleet |

**Hybrid gateway pattern** (confirmed on both IPQ6018 and MT7986): one radio =
802.11s mesh point (babeld), the other = AP or STA toward the MikroTik fleet.

**Strategic upside:** a couple of these stand up a *real* 802.11s island bridged to
the MikroTiks — demonstrating the protocol spanning open-mesh + closed-AP/STA
hardware, which is the portability story for the talk. Full survey + sources in the
`mjolnir-mesh-b1d` notes.

## Status & next steps

- **Validated:** L3 mesh + derived-IPv4 backhaul + direct iroh tunnels + mDNS
  discovery, end-to-end on a single L2 segment (armv7 Linux containers).
- **Next concrete step:** `mjolnir-mesh-2j6` — 4-node mesh on the wired switch
  (real babeld + client data path). Hardware-agnostic; proves the protocol on the
  known-good shared segment. Precursor to multi-hop discovery.
- **Then:** multi-hop discovery (babeld/mDNS synchronization) — its own bead.
- **Radio:** AP/STA identical-node baseline on the MikroTiks; experiment with
  real-mesh open nodes (hybrid) in parallel; reconcile via L3.
