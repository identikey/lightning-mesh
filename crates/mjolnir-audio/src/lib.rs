//! mjolnir-audio: Opus audio pipeline for mesh streaming.
//!
//! Capture → encode on one side; receive → jitter-buffer → decode → mix
//! → playback on the other. The transport layer (which moves encoded
//! Opus frames between peers) lives outside this crate; consumers feed
//! decoded payloads into the [`Mixer`] via [`PeerInput`].
//!
//! # Neural DSP substrate
//!
//! [`tract-onnx`](https://github.com/sonos/tract) is included as a hard
//! dependency and is the default runtime for all small-model neural DSP
//! in this crate. Sonos's pure-Rust ONNX runtime is the right substrate
//! for the audio inference thread: no dynamic library, no GPU command
//! submission overhead, smaller binary footprint than `ort`, designed
//! from the start for on-device real-time audio. See
//! `docs/research/audio-models-for-neural-plc/synthesis.md` §7 for the
//! full comparison.
//!
//! Implementors building on this crate get tract as a foundation —
//! they shouldn't need to choose an inference runtime to add a neural
//! component.
//!
//! ## Convention for new neural components
//!
//! Each tract-backed component lives in its own module named
//! `{component}_tract.rs` (e.g. [`plc_tract`] today; `vad_tract`,
//! `denoise_tract` later). Component structs follow this shape:
//!
//! - Hold a compiled `TypedSimplePlan<TypedModel>` as a field, built
//!   once at construction so per-frame inference doesn't pay model
//!   optimisation cost.
//! - Take the ONNX model path (or `&[u8]` bytes) at construction.
//!   Surface load / shape errors at peer creation, not on the audio
//!   thread.
//! - Pre-allocate input and output tensors at construction. The
//!   inference path must not allocate.
//! - Honest-fail with a clear error when a model isn't wired —
//!   silent-silence or fake output is worse than a visible "no model
//!   configured for this component" message.
//!
//! ## Future neural components (planned)
//!
//! These are the next slots the substrate is intentionally sized for.
//! Each is a `{name}_tract.rs` module that will sit alongside
//! [`plc_tract`]. None are implemented yet; this section documents
//! intent so future work picks the same patterns.
//!
//! - **PLC** (packet loss concealment) — [`plc_tract::TractPlc`].
//!   Status: scaffold today, runtime wired, model selection pending.
//!   Candidate models: **tPLCnet** (MIT, TFLite → needs ONNX conversion),
//!   **PARCnet-IS2** (small CNN + LP hybrid, 416K params, music-aware).
//!   FRN is shippable from a runtime standpoint but CC-BY-NC blocks it.
//!   See `docs/research/audio-models-for-neural-plc/`.
//!
//! - **VAD** (voice activity detection) — `vad_tract::TractVad` (planned).
//!   Use case: gate the per-peer inference task during silence so we
//!   don't burn CPU on speech-PLC for non-speech intervals; also a
//!   signal for the mixer to drop silent peers from the active set.
//!   Canonical model: **Silero VAD** (MIT, ONNX weights ship with the
//!   repo, ~2 MB, designed for real-time use, well-trodden tract path).
//!
//! - **Speech enhancement / noise suppression** —
//!   `denoise_tract::TractDenoise` (planned). Use case: clean up the
//!   captured microphone signal before encoding, so we send better
//!   audio over the wire. Candidate models: **DeepFilterNet** (MIT,
//!   small enough for real-time CPU, ONNX export available),
//!   **RNNoise** (BSD, classical small-RNN model, the long-time
//!   baseline). DeepFilterNet 2 is the modern choice; RNNoise is the
//!   "fits in 200 KB" baseline if binary size matters.
//!
//! - **Speaker embeddings** —
//!   `speaker_tract::TractSpeakerEmbedding` (planned). Use case:
//!   per-speaker conditioning for the PLC backend (make synthesised
//!   PLC audio favour the actual speaker's voice), plus speaker-keyed
//!   mute / spotlight features. Canonical models: **ECAPA-TDNN**
//!   (Apache, SpeechBrain ships ONNX exports), **WavLM-base+** (MIT,
//!   bigger but state-of-art).
//!
//! - **Neural codec post-filters** (lower priority) —
//!   `osce_tract::TractOsce` (planned). LACE / NoLACE-style postfilters
//!   that improve perceived quality of *received* low-bitrate Opus
//!   frames. Already ship in libopus 1.5+ as the `--enable-osce` build
//!   flag and run automatically on the decode path; only worth a
//!   standalone tract module if we want backends not coupled to
//!   libopus (e.g. on AIE).
//!
//! As each lands, lift any common scaffolding into a shared module
//! (likely `crates/mjolnir-audio/src/neural.rs`) — but extract on the
//! second or third instance, not preemptively.

pub mod capture;
pub mod codec;
pub mod conceal;
pub mod device;
pub mod mixer;
pub mod plc_tract;

pub use conceal::{
    default_plc_factory, silence_plc_factory, OpusPlc, PlcBackend, PlcFactory, SilencePlc,
};
pub use mixer::{Mixer, MixerHandle, PeerInput};
pub use mjolnir_media::{BufferStats, PullStatus};
pub use plc_tract::TractPlc;

/// Audio configuration for the mesh.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Sample rate in Hz. Opus supports 8000, 12000, 16000, 24000, 48000.
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Opus encoder bitrate in bits/sec.
    pub bitrate: i32,
    /// Frame duration in milliseconds. Opus supports 2.5, 5, 10, 20, 40, 60.
    pub frame_duration_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 1,
            bitrate: 64000,
            frame_duration_ms: 20,
        }
    }
}

impl AudioConfig {
    /// Number of samples per frame (per channel).
    pub fn frame_size(&self) -> usize {
        (self.sample_rate as usize * self.frame_duration_ms as usize) / 1000
    }
}
