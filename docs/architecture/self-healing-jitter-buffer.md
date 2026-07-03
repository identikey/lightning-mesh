# Self-Healing Jitter Buffer

> **Status (2026-07-02):** Design note on the audio track — dormant. Current
> focus is the router mesh data plane and the service-mesh phase. The core of
> this design shipped (see "Sequence of work" below); the neural backends did
> not, and are paused with the rest of the audio track.

A design note for the audio jitter buffer in `mjolnir-audio` and its eventual offload
to NPU dataflow hardware via [parakeet-aie](https://github.com/duke/parakeet-aie).

## Intent

Build a jitter buffer that is more than a reorder/dejitter queue: when packets are lost
or arrive too late, the buffer *generates* plausible fill audio from recent context
rather than emitting silence, a click, or a repeated frame. Ship a CPU implementation
first. Design the seam so the same buffer can later be hosted as a resident workload on
AMD XDNA2 (AI Engine) via the parakeet-aie runtime, where the dataflow tile model is
an unusually clean match for streaming PLC.

## The mental model: a Redis server, but the served data structure is the buffer itself

Redis is interesting not because it's fast but because of its *shape*: a long-running
process holds a data structure in memory, exposes a small command vocabulary over a
network protocol, and is responsible for maintaining the structure's invariants under
churn. Clients don't reach into the structure; they submit ops and read replies.

Apply that shape to the jitter buffer:

- The **served structure** is a self-healing PCM stream: a ring of recent audio frames,
  a small neural model warmed up on top of that ring, and the loss/arrival statistics
  the model is conditioned on.
- The **server** is a long-running, always-warm process. On CPU today, a Tokio task
  inside `mjolnir-audio`. On XDNA2 tomorrow, a persistent kernel cascade resident on
  the tile array, served via parakeet-aie's host runtime.
- The **clients** are the audio capture/decode side (which submits incoming Opus frames
  with sequence numbers and arrival times) and the playback side (which pulls PCM at a
  steady cadence).
- The **invariant** is: the playback side always gets a fresh frame of PCM on its
  cadence. Whether that frame came from a real network packet, a redundant FEC packet,
  a neural-PLC prediction, or a graceful cross-fade — the consumer doesn't care and
  doesn't see the difference.

The framing matters because it shapes the API. The buffer isn't a dumb FIFO with
hooks; it's a small service with a stable command surface, and the implementation
behind that surface can move from CPU to NPU without the rest of mjolnir-mesh
noticing.

## Served structure and command surface

```
Client                                  Buffer service
─────────                               ───────────────────────────────
push_packet(seq, arrival_ts, payload)  ─►  decode, place in ring, update loss stats
pop_frame(playback_ts)                 ─►  return PCM (real | FEC | neural | crossfade)
state()                                ─►  depth, recent loss rate, prediction count
configure(target_depth_ms, mode)       ─►  tuning hints
```

Two minimal commands carry the load — `push_packet` and `pop_frame`. The buffer is
responsible for: reordering by sequence, adaptive depth, FEC redundancy decoding,
loss detection, and neural concealment. The consumer never asks "was this frame
predicted?" — it just consumes a clean stream.

Internal state held by the service:

- A bounded ring of recent decoded PCM (the prediction model's context window).
- The Opus decoder (or whatever codec is in use) with its own state — critically, the
  decoder is *also* always-warm; PLC depends on it.
- A model handle: on CPU, the LACE/NoLACE weights mmap'd from disk. On AIE, a
  persistent kernel cascade and its DMA descriptors.
- A small ledger of recent sequence numbers and arrival timestamps, used to drive
  adaptive depth and to decide *when* to predict versus wait.

## Why the AIE dataflow model fits this workload unusually well

The structural argument behind parakeet-aie's ASR thesis — Parakeet is conv-heavy,
streaming-native, no KV cache — applies even more strongly to neural PLC:

- **Hard-streaming cadence.** 50 frames/sec, fixed per-frame deadline. AIE's dataflow
  scheduling gives bounded latency by construction; CPU/GPU defend determinism against
  scheduler jitter rather than provide it natively.
- **Tiny resident model.** LACE/NoLACE-class PLC is sub-1M params; even an ambitious
  design sits in low tens of millions. Fits in tile SRAM with no DRAM round-trips per
  frame.
- **The pipeline is literally a cascade.** PCM ring → context selector → PLC layers →
  cross-fade → output. That maps 1:1 onto a tile-row chained by stream connections,
  which is exactly the shape AIE wants.
- **Speculative fill is free.** On CPU, running PLC every frame "in case the next one
  is lost" wastes cycles. On a dedicated tile-row that's already there and warm, it
  costs nothing — predict ahead, throw the prediction away on success, use it on loss.
  Zero added latency on the loss path because the work is already done.
- **No autoregressive state growth.** Causal feed-forward or short recurrence. The
  KV-cache problem that defeats LLM decode on AIE doesn't exist here.

This is a smaller, sharper first workload for parakeet-aie than full Parakeet ASR. It
forces the runtime to solve the things that matter for *any* real workload (warm
kernels, host↔NPU streaming submit/return, bounded latency contracts) without the
distraction of a 600M-param model port.

## The seam — shipped

The seam lives in two crates: the *generic* decode-and-conceal trait is in
`mjolnir-media` (so a future `mjolnir-video` can share the same shape); the
*audio-specific* impls and ergonomic aliases live in `mjolnir-audio`. This
section originally sketched the design; what follows is what shipped. One
deliberate divergence from the original sketch: instead of an associated
`type Output` returned by value, output is written into a **caller-owned
`&mut [i16]` slice** — backends must not allocate on the inference path.

In `mjolnir-media/src/recover.rs` (implemented):

```rust
pub trait Recover: Send {
    /// Decode a freshly-arrived encoded packet, writing PCM into `out`.
    fn decode(&mut self, packet: &[u8], out: &mut [i16]) -> Result<()>;
    /// `lookahead` is the next-in-sequence packet (if it has already
    /// arrived); codecs supporting forward error correction can use it
    /// to reconstruct the lost frame. The lookahead is non-destructive:
    /// it remains in the buffer and is decoded normally at its own slot.
    fn decode_lost(&mut self, lookahead: Option<&[u8]>, out: &mut [i16]) -> Result<()>;
    fn supports_speculation(&self) -> bool { false }
}

// blanket impl so Box<dyn Recover> itself satisfies Recover
impl<R: ?Sized + Recover> Recover for Box<R> { ... }
```

The same trait carries both `decode` and `decode_lost` because codec-native PLC
(including Opus) depends on internal decoder state populated by previous
successful decodes; splitting the two across independent objects would force
expensive state mirroring. Backends that want explicit context (a neural PLC
conditioned on recent PCM) maintain it internally — they observe each decoded
frame inside their own `decode` impl.

In `mjolnir-media/src/service.rs` (implemented):

```rust
pub enum PullStatus {
    Empty,                              // warming up; slice untouched
    Decoded,                            // from a real received packet
    Concealed { fec_lookahead: bool },  // synthesised; flag = lookahead was available
}

pub struct BufferStats {
    pub received: u64,       // raw arrival count from the wire
    pub decoded: u64,
    pub concealed: u64,
    pub fec_recovered: u64,  // concealments where a lookahead was available
    pub errors: u64,
}

pub struct SelfHealingBuffer<R: Recover> { /* jitter + recover + stats */ }

impl<R: Recover> SelfHealingBuffer<R> {
    pub fn push(&mut self, seq: u64, packet: Bytes) -> PushOutcome { ... }
    pub fn pull(&mut self, out: &mut [i16]) -> Result<PullStatus> { ... }
    pub fn stats(&self) -> BufferStats { ... }
}
```

On `Pull::Gap` the buffer non-destructively peeks the next slot and
hands it to `decode_lost` as a recovery hint. Provenance flows back to
the consumer via the `PullStatus` variants, enabling cross-fade and
observability without leaking codec specifics. `BufferStats` is the
"Redis INFO" surface — running counts the mixer or any other consumer
can snapshot.

In `mjolnir-audio/src/conceal.rs` (implemented):

```rust
pub type PlcBackend = dyn Recover + Send;
pub type PlcFactory =
    Arc<dyn Fn(&AudioConfig) -> Result<Box<PlcBackend>> + Send + Sync>;
```

Backends, implemented and planned:

- `OpusPlc` — **shipped** (`conceal.rs`). Wraps the Opus decoder; `decode_lost`
  hands a lookahead packet to Opus's in-band FEC decode when one is available,
  and falls back to codec-native concealment (LACE/NoLACE in Opus 1.5+)
  otherwise. The CPU default; microsecond-class, zero new dependencies.
- `SilencePlc` — **shipped** (`conceal.rs`). Emits zeros on loss. Useful as a
  worst-case audibility reference and in tests.
- `TractPlc` — **scaffold shipped** (`plc_tract.rs`). The CPU neural backend,
  built on [tract-onnx](https://github.com/sonos/tract): it loads and compiles
  an ONNX plan today, but no PLC model architecture is wired yet —
  `decode_lost` bails with a pointer to the research notes. This is the real
  name of what earlier drafts called `CpuNeuralPlc`; the reference
  implementation against which AIE output will be validated.
- `AiePlc` — **future, does not exist**. Talks to a persistent kernel cascade
  via parakeet-aie's host runtime. Shares weights with `TractPlc`; the two
  should produce numerically close output (within quantization tolerance) for
  the same input. `supports_speculation()` returns `true`, enabling the mixer
  to drive the NPU every frame and discard predictions on successful packet
  arrival.

`default_plc_factory()` / `silence_plc_factory()` helpers thread the choice
through `Mixer::start_with_plc`; each registered peer mints its own backend
instance, wrapped in a `SelfHealingBuffer<Box<PlcBackend>>` inside the mixer's
per-peer slot.

A future `mjolnir-video` will declare its own trait alias (e.g.
`pub type VideoRecover = dyn Recover<Output = VideoFrame> + Send`) and reuse the
same `SelfHealingBuffer` machinery with a video-shaped output type.

The host↔NPU API parakeet-aie needs to expose for this to work is small and stays
small:

```
load_kernel(elf, layout)         -> kernel_handle
start_persistent(kernel_handle)  -> session
submit(session, context_ring)    -> request_id
poll(request_id)                 -> PCM | NotReady
shutdown(session)
```

That's it. Same surface as a tiny inference server. The persistent-kernel + streaming-
submit pattern is exactly what makes this a "Redis-style server" instead of a
per-frame kernel-launch model.

## Integration point in mjolnir-mesh today

The buffer sits between the network recv pump and the cpal output stream:

```
network ─► audio_proto recv pump ─► JitterBuffer (mjolnir-media)
                                         │
                                         ▼
                                   PlcBackend ─► Mixer ─► cpal output
                                  (Opus | Silence | …)
```

The recv pump in `crates/mjolnir-node/src/audio_proto.rs` reads QUIC datagrams
(`[u64 LE seq][opus bytes]`) off the peer's connection and pushes them into the
per-peer jitter buffer via `PeerInput`. Datagrams are unreliable and unordered,
so the wire-supplied seq is the buffer's authoritative source of truth for
detecting real loss and reorder — both required for FEC-driven concealment to
have anything to do. The cpal output callback in
`crates/mjolnir-audio/src/mixer.rs` drains each peer's buffer at the audio clock
rate, decoding present frames with Opus's regular path and calling the
`PlcBackend` for gaps. Opus in-band FEC is enabled on the encoder side
(`crates/mjolnir-audio/src/codec.rs`), and the `decode_fec` lookahead path is
wired through the buffer's pull logic: on a gap, `SelfHealingBuffer::pull`
peeks the next-in-sequence packet and `OpusPlc::decode_lost` reconstructs the
lost frame from its FEC payload.

## Sequence of work

1. **CPU jitter buffer + Opus FEC.** ✅ Done. FEC on the encoder side, the
   FEC-lookahead `decode_lost` path on the decoder side, and an adaptive-depth
   reorder buffer in the middle. `PlcBackend` default is `OpusPlc`. This closed
   the embarrassing gap where the earlier code silently dropped late packets.

2. **Design the `PlcBackend` trait and the buffer's command surface.** ✅ Done.
   The `Recover` trait (`mjolnir-media/src/recover.rs`), `SelfHealingBuffer`
   (`service.rs`), and the `PlcFactory` seam in the mixer all shipped. This is
   the seam other backends slot into later.

3. **Reference neural PLC on CPU.** ◐ Scaffold shipped. `TractPlc`
   (`plc_tract.rs`, tract-onnx) loads and compiles an ONNX plan, but the actual
   PLC model architecture is not wired yet. Picking and wiring a small
   published model (LACE/NoLACE port, or a tiny custom one) remains open. This
   is the ground truth that the AIE port must match within tolerance.

4. **parakeet-aie host runtime — minimal slice.** Open. Define and implement the small
   `load_kernel / start_persistent / submit / poll / shutdown` surface in parakeet-aie,
   targeted specifically at this workload. This gives parakeet-aie a real first
   integration test that isn't ASR-scale.

5. **AIE kernel cascade for the neural PLC.** Open. Port the reference model
   layer-by-layer to AIE tiles. Validate numerically against `TractPlc` on
   identical input.

6. **`AiePlc` backend in mjolnir-audio.** Open. Trivial once steps 4 and 5 exist —
   it's just wrapping the parakeet-aie session in the `PlcBackend` trait.

Steps 1–2 are done. The rest is paused with the audio track; step 3 can run in
parallel with step 4 whenever it resumes, and steps 5–6 are downstream of
parakeet-aie progress.

## Open questions

- **Speculative prediction policy.** On AIE, always-on speculative fill is free. On
  CPU it costs real cycles. Does the trait expose "predict speculatively every frame"
  vs. "predict only on detected loss" as a backend-chosen behavior, or as a buffer-
  level policy with the backend just providing `predict()`? Leaning toward the
  former: the backend knows whether speculation is cheap for it.

- **Multi-track buffering.** When mjolnir-mesh grows to video and screen-share tracks
  (see assessment in `../archive/network-coordination/mesh-network-coordination.md` and the moq-lite group story), the
  jitter buffer abstraction probably wants to be generalized — different deadlines,
  different concealment strategies, but the same Redis-style service shape.

- **Cross-fade and discontinuity handling.** When a real frame finally arrives after
  one or more predicted frames, naive concatenation will click. The buffer needs a
  short cross-fade or a phase-aware blend. This is buffer logic, not backend logic.

- **Per-peer vs. shared service.** In a multi-peer room, each remote peer's audio is
  an independent stream needing its own buffer. On CPU that's just N instances. On
  AIE, do we run N tile cascades, one shared cascade with batched submit, or a hybrid?
  This is a parakeet-aie scheduling question, deferred until the single-stream case
  works.

## Related

- [parakeet-aie](https://github.com/duke/parakeet-aie) — the AIE runtime this design
  depends on for the NPU backend
- `crates/mjolnir-node/src/audio_proto.rs` — QUIC-datagram recv pump that feeds the jitter buffer
- `crates/mjolnir-audio/src/mixer.rs` — per-peer jitter pull + PLC dispatch + sum-mix
- `crates/mjolnir-audio/src/codec.rs` — Opus encoder (in-band FEC enabled) and decoder (PLC + FEC decode paths)
- Opus 1.5 neural PLC: LACE and NoLACE (Xiph publication, MIT/BSD license)
- Interspeech PLC Challenge — public benchmark for neural concealment quality
