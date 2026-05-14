//! mjolnir-media: real-time media primitives shared across mjolnir crates.
//!
//! Currently exposes a generic [`JitterBuffer`] suitable for sequence-numbered
//! frames (Opus packets, video NALUs, etc.) arriving from a network transport
//! and drained at a fixed clock rate.

pub mod jitter;

pub use jitter::{JitterBuffer, Pull, PushOutcome};
