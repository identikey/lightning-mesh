# Neural Bridge PLC — A Streaming Speech LM for Multi-Source Concealment

> **Status (2026-07-02):** Design note on the audio track — dormant. Current
> focus is the router mesh data plane and the service-mesh phase.

A design note for the v2 packet loss concealment lane in `mjolnir-audio`. This is
the engine that runs *after* FARGAN + DRED stop being enough — long burst losses,
multi-second mesh detours, broadcast resync. Companion to
[`self-healing-jitter-buffer.md`](self-healing-jitter-buffer.md) and to the
deployment-focused
[neural-PLC synthesis](../research/audio-models-for-neural-plc/synthesis.md).

## Intent

Build a single generative model that runs continuously in lockstep with the audio
clock, predicting the next frame at every step. Whether the next emitted frame is
*real* or *hallucinated* becomes purely a question of which input source is
authoritative at that moment — not a mode switch inside the model. The same
forward pass produces both. PLC is what happens when no authoritative source
arrives before the playback deadline.

This is structurally distinct from FARGAN. FARGAN is an extrapolator that fires
on loss. This is a predictor that is *always running*; loss just changes which
distribution is sampled from.

## Scope and relationship to existing tiers

Concealment is a cascade, not a single mechanism. After this design lands:

| Gap length    | Mechanism                              | Source                |
|---------------|----------------------------------------|-----------------------|
| 0–20 ms       | Opus heuristic / nothing               | libopus               |
| 20–80 ms      | FARGAN (deep PLC)                      | libopus 1.5+          |
| 80–1000 ms    | DRED redundancy (backward fill)        | libopus 1.5+          |
| 200 ms – 30 s | **Neural Bridge PLC (this doc)**       | new                   |
| > 30 s        | Fade to comfort noise / mesh-radio     | client-side aesthetic |

The bands overlap. Bridge PLC is also the natural home for *broadcast-mode*
recovery where audio was captured to disk and a multi-second outage needs to be
patched offline without latency pressure — same model, more compute per step.

## Mental model: one model, four input streams, one output

The engine consumes four ordered token streams and emits one:

```
   live frames   ─┐
   DRED past     ─┤
   future anchor ─┼──►  reconciliation  ──► streaming speech LM ──► tokens ──► codec decoder ──► PCM
   self-hallucin ─┘                              │
                                                 └──► metadata sidechannel
                                                       (entropy, confidence,
                                                        source provenance,
                                                        glitch level)
```

The model itself is causal and operates on a single token stream. The
reconciliation layer in front of it picks the authoritative source per frame
according to a priority order:

1. **Live frames** that arrived before deadline. Ground truth, just-in-time.
2. **DRED redundancy** that reveals the recent past. Ground truth, retroactive.
3. **Future anchor** packets that arrived late but reveal a non-immediate future.
   Not authoritative as a token-at-time-N but used as a conditioning signal that
   pulls the in-flight generation toward a known endpoint.
4. **Self-hallucination** — the model's own previous-step samples. Used when
   nothing else is available.

The model never needs to know which source any given token came from. From its
perspective, it is always being teacher-forced. Whether the "teacher" is the
network or itself is the reconciliation layer's problem.

## Components

### 1. Streaming neural codec

Raw 48 kHz audio is too high-rate to predict in a transformer. Encode causally
to discrete tokens at 12.5–50 Hz × N residual-VQ codebooks. Mimi (Kyutai,
Apache 2.0) is the reference: 12.5 Hz acoustic tokens, 8 codebooks, ~80 ms
algorithmic latency, streaming encoder/decoder. Encodec at 75 Hz is the
fallback if Mimi proves hard to integrate.

The codec's encoder runs on every received frame (real or DRED). Its decoder
runs on every output frame. The cost of encode/decode is fixed, predictable,
and small relative to the LM.

### 2. The streaming speech LM (the engine)

A causal sequence model over codec tokens. Two architecture candidates:

- **Mamba / selective SSM.** Recurrent hidden state captures the running
  summary of all prior tokens. O(1) per step. Natural fit for the
  always-running pattern; no fixed context window to age out of.
- **Streaming transformer with KV cache.** Sliding window of recent context.
  More mature tooling, slightly worse for very long histories.

Default recommendation: **Mamba/SSM** for the long-history property. Either
works; the rest of the design is architecture-agnostic.

Sizing: 100M – 500M parameters. Smaller than a TTS model because the codebook
is small and the speech distribution is narrow vs. arbitrary text. Real-time
inference on a single CPU core with int8 + SIMD is feasible at the low end;
GPU offload comfortably handles the upper end.

### 3. Speculative output buffer with entropy-adaptive depth

The model runs *faster than realtime* and emits into a small pending buffer
ahead of the playback position. The buffer is a rolling window of "tokens
already generated but not yet played out." Frames inside it are revocable.

Buffer depth is a function of the model's running entropy (per-frame KL or
prediction confidence):

```rust
fn target_buffer_ms(running_entropy: f32) -> u32 {
    // confident => shallow buffer (cheap compute, low latency tax)
    // uncertain => deep buffer (more rewrite headroom for late anchors)
    let normalized = sigmoid((running_entropy - ENTROPY_MIDPOINT) / ENTROPY_SCALE);
    lerp(MIN_BUFFER_MS, MAX_BUFFER_MS, normalized)
}
```

Typical bounds: 50 ms when the model is confident, 300 ms when uncertain. The
extra latency tax is paid only when uncertainty makes it worth paying.

This is the unlock that makes future-anchor reconciliation actually possible
(§ "Future anchors and bridging" below). It also dynamically modulates per-peer
compute load — confident streams cost less.

### 4. Reconciliation state machine

Per-frame logic that picks the authoritative source and feeds it to the model.
Sketch:

```
on frame tick T:
  arrived = drain_packets_for_window(T - DRED_WINDOW, T + ANCHOR_WINDOW)

  for each token-position p in [committed_position, T + buffer_depth]:
    source = pick_source(p, arrived)
      // 1. live frame for p
      // 2. DRED-reconstructed for p
      // 3. future-anchor cross-attn (does not become a token at p, conditions instead)
      // 4. previous self-hallucinated token at p

    if source != previous_source_at(p):
      mark_for_replay(p)

  if any_replay_marked:
    rewind_state_to_earliest_replay()
    re-run_model_forward_through_buffer()

  emit_token(committed_position)
  advance committed_position
```

The replay is bounded by the speculative buffer depth. Anything older than the
committed playback position is irrevocable; the listener already heard it.

## Inference modes

The same checkpoint handles three modes by changing *what conditioning is
present*, not by changing the model.

### Forward-only hallucination

No anchor, no fresh DRED. Pure causal extrapolation from the running hidden
state. This is the degenerate case: just sample from the next-token
distribution. Quality degrades the longer it runs; mitigated by the
glitch-confidence sidechannel surfacing the degradation honestly.

### Bridging to a future anchor (the inpainting case)

A late packet arrives carrying audio that should play 500 ms – several seconds
*from now*. The gap between current playback and the anchor must be filled
with a trajectory that lands cleanly on the anchor.

The anchor is fed via a cross-attention head over its encoded codec tokens.
The main causal stream generates left-to-right; at each step it attends to the
suffix. Trained with **fill-in-the-middle (FIM)** masking (see Training,
below) — the same recipe code LLMs use. One set of weights, no separate
reversibility math.

When the anchor is inside the speculative buffer's reach, the buffer is
rewritten to interpolate toward it before any contested frame plays out loud.

### Doubly-anchored (DRED past + future anchor)

The valuable special case. DRED arrives revealing real audio for `[T - δ₁, T]`;
a future anchor reveals real audio at `[T + δ₂, T + δ₂ + δ₃]`. The current
playback position is sandwiched between two known regions.

The model:

1. **Snapshots and replays state.** Rolls back to the most recent state
   snapshot before T - δ₁, re-runs forward through the *real* (DRED-revealed)
   tokens. The model's hidden state at T is now consistent with reality, even
   though the listener heard a different version a few hundred ms ago.
2. **Bridges with both ends pinned.** Forward from corrected state, attending
   to the future anchor.
3. **Plays DRED ground truth** through the speculative window before the
   bridge takes over. Cross-fade at the join.

This three-source fusion is the architectural novelty. Single-stream PLC
handles one of these signals; nothing existing handles all three coherently.

### Broadcast / offline refinement

When the latency budget is removed (post-call recovery, recorded broadcast
fill, archive re-rendering), the same model runs in iterative non-causal mode
à la MaskGIT: fill highest-confidence tokens first, recondition, fill the next
batch, repeat for 5–10 passes. Substantially better quality than the single
causal pass. Same weights, different inference loop, exposed via a `mode`
parameter on the inference call.

This means realtime PLC quality becomes a *lower bound* for any given gap, not
the only quality available. Recordings can be re-rendered offline and the
better version stored.

## Snapshot-and-replay for state correction

The model's hidden state is advanced on whatever token sequence the
reconciliation layer feeds it. When that sequence is hallucinated and later
revealed to be wrong, the state is now inconsistent with reality.

Rather than literal SSM reversibility (which is fragile under
input-dependent recurrences like Mamba), maintain periodic state snapshots:

- Checkpoint hidden state every ~200 ms during normal operation. Cheap;
  state is small (~MB for a 500M-param SSM).
- Keep a rolling ring of the last ~30 s of snapshots.
- On reveal-of-correction (DRED arrives, or anchor implies a different past),
  pick the most recent snapshot before the divergence, re-run forward with
  corrected tokens.

This is conceptually persistent-data-structure history with audio's relaxed
correctness requirement: the listener heard what they heard, but everything
downstream — bridges, confidence estimates, glitch metadata — uses the
corrected trajectory.

## Metadata sidechannel

The model emits a parallel stream of per-frame analytics alongside the audio
tokens. This is a first-class output, not an afterthought.

```
struct FrameMetadata {
    timestamp_ms: u64,
    source: Source,              // Live | Dred | Bridge | Hallucination
    entropy: f32,                // raw prediction entropy in nats
    confidence: f32,             // 0..1, calibrated against held-out validation
    bridge_kl: Option<f32>,      // KL between forward-only and anchor-conditioned
                                 //   distributions when bridging; high = anchor
                                 //   disagrees strongly with the hallucinated past
    glitch_level: f32,           // 0..1, derived from entropy + bridge_kl
    buffer_depth_ms: u32,        // current speculative buffer depth
    replay_count_recent: u16,    // number of rewrites in the last second
}
```

Uses:

- **Analytics.** Running entropy timeseries reveals link quality, speaker
  intelligibility, codec robustness. This is the metric to dashboard.
- **Adaptive buffer depth.** Closes the loop with the speculative buffer
  (already described).
- **Client-side aesthetics.** The glitch-level channel drives the
  mesh-radio UX (next section).
- **Training data triage.** When uploading recordings for post-hoc adaptation
  (next section), prioritize segments with high bridge_kl — those are the
  hard cases where the model was caught out, the highest-value training
  examples.

The sidechannel is not transmitted over the network. It is produced at the
receiver alongside the decoded audio and consumed by the audio playback layer
and local analytics.

## The mesh-radio aesthetic — surfacing uncertainty to the user

Lossy transmission has always had a visual analog (deepdream-ish fills,
artifacts, blur). The honest UX choice is to *expose* the lossiness rather
than smooth it away invisibly. Drive aesthetic mixing from the metadata
sidechannel's `glitch_level`.

Client-side configuration (not the model's concern):

| `glitch_audibility` | Behavior                                                                            |
|---------------------|-------------------------------------------------------------------------------------|
| `0.0`               | Pure smoothed hallucination, no indication anything was lost                        |
| `0.5`               | Subtle "ghost in the wire" — broadband noise breathes louder during high-glitch    |
| `1.0`               | Full mesh-radio aesthetic — white noise loudness ∝ glitch level                     |
| `mode = hard_dropout` | Gate to silence when `glitch_level > threshold` instead of mixing noise            |
| `mode = multiverse` | Phaser / pitch-warble effect during high-bridge_kl moments only                     |

The model exports the signal honestly; the client chooses whether to make
loss audible, ambient, or invisible. The "transmission from the multiverse"
case (hallucination diverges and the anchor forces a contradictory bridge)
gets its own knob because it is a qualitatively different artifact from
pure-uncertainty noise.

## Training objective

### Pre-training: fill-in-the-middle (FIM) on codec tokens

Standard recipe, applied to a codec-tokenized speech corpus. Each training
example is a random rearrangement:

```
original:   [a b c d e f g h]
masked:     [a b]  +  [g h]  +  [c d e f]
input:      <prefix> a b <suffix> g h <fill> c d e f <eos>
```

Trained with normal causal next-token loss. The model learns to:

- Predict the next token given prefix only (forward extrapolation, when no
  anchor is present).
- Predict the next token given prefix + suffix (bridging, when an anchor is
  present).

Mix the rearrangement probability across the batch — e.g., 60% pure forward,
40% FIM with varying mask lengths. One checkpoint, both modes.

Corpus: conversational speech for the bulk (LibriSpeech, Common Voice,
GigaSpeech, VoxPopuli). Diversity in speaker count and acoustic conditions
matters more than total hours past some threshold.

Compute: this is the expensive part. Months of GPU time on the order of
LLaMA-7B-class budget. Done once, ahead of any production work.

### Cross-attention for anchor conditioning

When training FIM batches, the suffix tokens go through a small bidirectional
encoder; the causal decoder cross-attends to its output. At inference, this
becomes the optional "future anchor" input. When no anchor is present, the
cross-attention is masked out and the model degrades gracefully to forward-only.

## Post-hoc adaptation, not test-time training

Real-time on-device SGD is too expensive to justify here. The CPU budget for a
mesh peer is already tight, and the wins from per-call online training are
diffuse.

Instead, **save the diffs**:

```
struct AdaptationRecord {
    context_hash: u128,        // identifies state snapshot
    predicted_distribution: CompressedLogits,
    actual_token: CodecToken,
    metadata: FrameMetadata,
}
```

Every time the reconciliation layer reveals what the model *should* have
predicted (live frame arrives confirming or contradicting recent
hallucinations; DRED reveals corrected past), append a record to a local
ring. Periodically compress and upload to a server (with user consent, scoped
to opted-in contacts).

Server-side training runs nightly or weekly:

- General-purpose adapter updates: pooled across all consenting users,
  retrained as the base model's drift correction.
- Per-contact LoRA adapters: rank-8 or rank-16 deltas trained on the diffs
  recorded *during conversations with that contact*. Hot-swapped at call
  start when speaker identification matches.
- Per-aesthetic LoRA adapters: trained on different curated corpora to give
  the hallucinator a "voice" — formal news anchor, warm conversational, lo-fi
  radio, etc. Becomes a creative tool, not just a recovery mechanism.

Speaker identification at call start uses a small front-end embedding model
(ECAPA-TDNN, WavLM-style). First few seconds use the general checkpoint; the
per-contact LoRA loads once identification stabilizes.

### Why this is better than TTT

- **CPU budget.** Inference-only on the audio thread. No backward pass, no
  optimizer state, no learning-rate scheduler running in real time.
- **Better gradient quality.** Mini-batches over hours of recorded diffs
  produce stable updates. A real-time per-frame SGD step is noisy.
- **Privacy controls.** A per-contact data lifecycle becomes explicit. Users
  can purge a contact's adapter; the data was never on someone else's device
  to begin with.
- **A/B-able.** Server-side training lets you compare adapter versions,
  measure win rate on held-out diffs, roll back if a checkpoint regresses.

The cost: adaptation lags. Bob's adapter learns from yesterday's calls, not
this morning's. For the problem this is solving, that is fine.

## LoRA slots — recovery and aesthetic in one mechanism

The same hot-swappable LoRA infrastructure serves two ends:

1. **Per-contact recovery.** Improves hallucination quality when the network
   drops a packet from someone you talk to often.
2. **Per-aesthetic post-processing.** Even when no loss is occurring, the
   audio can be routed through the hallucinator in a "voice transfer" mode —
   re-tokenize incoming live audio, generate with a different LoRA loaded,
   decode. Becomes a creative voice-mod feature on top of the recovery
   stack. Costs a bit more compute but the engine is already running.

This bundles two product features into one architectural slot. Cleanly.

## State machine — the seam in `Recover`

The existing `Recover` trait in `crates/mjolnir-media/src/recover.rs` has a
single `decode_lost()` method. This design implies a richer interface:

```rust
pub trait StreamingRecover {
    /// Authoritative real frame arrived. Advance state, return decoded PCM.
    fn observe(&mut self, frame: CodecFrame) -> AudioOutput;

    /// DRED redundancy reveals past audio. Snapshot-replay state correction.
    fn observe_dred(&mut self, frames: &[CodecFrame], position: FramePos) -> Vec<ReplayHint>;

    /// Future anchor arrived. Used as cross-attention conditioning for upcoming generates.
    fn observe_anchor(&mut self, frames: &[CodecFrame], position: FramePos);

    /// No frame arrived in time. Hallucinate forward.
    fn generate(&mut self) -> AudioOutput;

    /// Stream of frame-level metadata produced alongside every observe/generate.
    fn metadata_rx(&self) -> &Receiver<FrameMetadata>;
}

pub struct AudioOutput {
    pub pcm: SmallVec<[i16; 960]>,  // no heap alloc on audio thread
    pub provenance: Source,
    pub speculative: bool,           // true if this came from buffer rewrite area
}
```

The reconciliation state machine is internal to the implementation. The trait
exposes the four input streams as four methods plus the metadata receiver.

This is a backward-incompatible change to `Recover` — the synthesis already
flagged the trait as not yet frozen, so this is the right moment to evolve it.
The existing FARGAN-backed `OpusPlc` continues to implement the simpler
`decode_lost` shape; `StreamingRecover` is an optional, richer alternative
that the bridge engine implements.

## Compute and where it runs

Per-peer cost depends on speculation depth and mode. Rough numbers for a
300M-param SSM with int8 quantization on x86:

| Mode                            | Per-frame cost (12.5 Hz)        | Notes                                      |
|---------------------------------|----------------------------------|--------------------------------------------|
| Confident forward (low entropy) | ~80 µs                           | Cheap; barely runs ahead                   |
| Uncertain forward               | ~80 µs × 2x speculation = 160 µs | Speculation depth grows                    |
| Bridging with anchor            | ~120 µs                          | Cross-attention adds cost                  |
| Doubly-anchored replay          | ~300 µs (one-off per reveal)     | Snapshot-restore + replay through buffer   |
| Broadcast iterative refinement  | ~5× per-frame, many iterations   | Offline only                               |

Multiplied across N peers and against a 12.5 Hz frame rate (80 ms per frame),
single-core x86 budget supports ~10 peers in pure-forward mode at int8. AIE
mapping (per [`self-healing-jitter-buffer.md`](self-healing-jitter-buffer.md))
remains a 2027+ target; the codec front-end and a CNN-shaped postfilter map
better to AIE than the SSM does.

## Open questions

These are the calls I don't want to make in advance.

1. **Mamba vs streaming transformer.** SSM has the cleaner mental model.
   Transformer has the more mature tooling. Benchmark both on the FIM
   objective with the same parameter budget, pick by held-out bridge quality.
2. **Codec choice — Mimi vs Encodec vs SoundStream variant.** Mimi is the
   modern reference but its Apache-2.0 license needs verification against
   our distribution model. Encodec at 75 Hz costs more compute per frame.
3. **Anchor delivery mechanism on the wire.** Future anchors are not a
   standard transport feature. Options: piggyback in mesh multipath
   duplicates; explicit forward-FEC-style anchor packets sent at lower
   priority; opportunistic anchor-on-recovery from peer caches. Co-design
   with the transport layer.
4. **Calibration of `glitch_level`.** Maps from raw entropy + bridge KL to a
   client-facing 0..1 scale. Needs subjective MOS-style validation to make
   sure the mapping aligns with perceived audio quality.
5. **Per-contact adapter cold-start.** First 1–2 seconds of a call use the
   general checkpoint. Quantify how much this matters in subjective tests
   before deciding whether a faster speaker-ID front end is worth the cost.

## Roadmap

This is a v2 lane. The synthesis's 90-day plan still owns days 0–90 (deep-PLC
enable, DRED, trait hardening, tPLCnet fallback). The Bridge PLC track is
sequenced after that:

- **Months 3–6: spike.** Take Mimi off the shelf. Train a small (~100M-param)
  FIM speech LM on LibriSpeech + Common Voice. Wire it behind
  `StreamingRecover` with a feature flag. Forward-only mode first.
  Kill criterion: per-peer compute exceeds audio-thread budget on target
  hardware.
- **Months 6–9: bridging mode.** Add cross-attention conditioning on future
  anchors. Define the on-wire anchor packet format. A/B against FARGAN+DRED on
  bursts > 200 ms.
- **Months 9–12: metadata sidechannel + speculative buffer.**
  Entropy-adaptive depth, glitch-confidence calibration, mesh-radio client
  UX.
- **Months 12–18: LoRA infrastructure.** Per-contact adapter pipeline,
  speaker-ID front end, server-side training loop. Per-aesthetic adapters
  shipped as a creative feature.
- **Months 18+: broadcast-mode iterative refinement.** Offline re-render of
  recordings. Archive recovery API.

The "bridging mode" step is the inflection point. If bridge quality
substantively beats FARGAN+DRED on real burst losses, this whole track earns
its keep. If not, it stays a research artifact and FARGAN+DRED remains the
production answer.

## References

- [`docs/research/audio-models-for-neural-plc/synthesis.md`](../research/audio-models-for-neural-plc/synthesis.md)
  — the deployment-focused survey; this design lives "after" its 90-day plan
- [`docs/architecture/self-healing-jitter-buffer.md`](self-healing-jitter-buffer.md)
  — the buffer-as-service framing this design extends
- Kyutai Moshi / Mimi — closest existing system; the engine repurposes their
  RQ-Transformer-over-Mimi-tokens architecture for PLC instead of full-duplex
  conversation
- DRED (`draft-ietf-mlcodec-opus-dred`) — the backward-redundancy mechanism
  this design treats as one of its four input streams
- Fill-in-the-middle training as in OpenAI's "Efficient Training of Language
  Models to Fill in the Middle" — the masking recipe for one-checkpoint
  forward + bridging
- MaskGIT — the iterative refinement schedule used in broadcast mode

## Verification

This is a design doc, not a tested implementation. Claims with numerical
estimates (compute per frame, parameter counts, latency bounds) are
back-of-envelope; benchmarking is part of the months-3–6 spike. Architectural
choices (FIM over reversible SSM, snapshot-replay over numerical inversion,
post-hoc adaptation over TTT) are taken from the conversation that produced
this doc and have specific rationales recorded in each section. Open
questions are explicit and bounded.
