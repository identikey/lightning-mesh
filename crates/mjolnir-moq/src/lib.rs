//! mjolnir-moq: Bridge between iroh QUIC connections and moq-lite sessions.
//!
//! This crate provides the thin protocol layer that wraps iroh `Connection`s
//! as WebTransport sessions consumable by moq-lite's publish/subscribe API.

mod session;

pub use session::{MoqBridge, MoqSession};

/// ALPN protocol identifier for MoQ over iroh.
pub const MOQ_ALPN: &[u8] = b"moq-lite/0";
