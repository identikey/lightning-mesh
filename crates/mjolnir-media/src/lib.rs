//! mjolnir-media: real-time media primitives shared across mjolnir crates.
//!
//! - [`JitterBuffer`] — generic sequence-keyed reorder/dejitter ring.
//! - [`Recover`] — the decode-and-conceal trait shared by all media types.
//! - [`SelfHealingBuffer`] — composition of jitter + recover, the
//!   "Redis-style" served data structure described in
//!   `docs/architecture/self-healing-jitter-buffer.md` of mjolnir-mesh.

pub mod jitter;
pub mod recover;
pub mod service;

pub use jitter::{JitterBuffer, Pull, PushOutcome};
pub use recover::Recover;
pub use service::{BufferStats, Pulled, SelfHealingBuffer};
