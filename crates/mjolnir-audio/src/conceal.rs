//! Packet Loss Concealment (PLC) backend seam.
//!
//! [`PlcBackend`] is the pluggable interface the mixer talks to for decoded
//! PCM. The same trait handles both the happy path (decode a received
//! packet) and the loss path (synthesise PCM when the expected packet is
//! missing); they share state, so they live behind one trait.
//!
//! Two backends ship in-tree:
//!
//! * [`OpusPlc`] — the CPU default. Uses Opus's built-in decoder PLC
//!   ([`OpusDecoder::decode_lost`]), which draws on recent codec state to
//!   synthesise a smooth fill frame. Microsecond-class on a modern CPU.
//! * [`SilencePlc`] — a baseline that emits zeros on loss. Useful as a
//!   worst-case reference and in tests.
//!
//! Future backends (neural PLC on CPU, AIE-resident cascade) implement the
//! same trait. See `docs/architecture/self-healing-jitter-buffer.md`.

use anyhow::Result;
use std::sync::Arc;

use crate::codec::OpusDecoder;
use crate::AudioConfig;

/// The decode-and-conceal seam. Implementations own the state needed to
/// produce coherent PCM both for received packets and for missing ones.
///
/// The same instance handles both calls because codec-native PLC
/// (including Opus) depends on internal decoder state populated by
/// previous successful decodes; splitting decode and conceal across
/// independent objects would force expensive state mirroring.
pub trait PlcBackend: Send {
    /// Decode a freshly-arrived packet into PCM `i16` samples.
    fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>>;

    /// Synthesise PCM for one frame the network failed to deliver.
    fn decode_lost(&mut self) -> Result<Vec<i16>>;

    /// Whether this backend benefits from pre-emptive prediction.
    ///
    /// Backends that can predict for free (e.g. NPU-resident cascades
    /// running every cycle anyway) return `true`. The mixer may then
    /// speculate ahead, discarding the prediction if a real packet
    /// arrives. CPU backends typically return `false` (the default).
    fn supports_speculation(&self) -> bool {
        false
    }
}

/// Factory closure type used by [`Mixer`](crate::Mixer) to mint a fresh
/// per-peer backend.
pub type PlcFactory =
    Arc<dyn Fn(&AudioConfig) -> Result<Box<dyn PlcBackend>> + Send + Sync>;

/// Default factory: one [`OpusPlc`] per peer.
pub fn default_plc_factory() -> PlcFactory {
    Arc::new(|cfg| Ok(Box::new(OpusPlc::new(cfg)?) as Box<dyn PlcBackend>))
}

/// Factory that produces [`SilencePlc`] backends. Intended for tests and
/// dropout-audibility demos.
pub fn silence_plc_factory() -> PlcFactory {
    Arc::new(|cfg| Ok(Box::new(SilencePlc::new(cfg)?) as Box<dyn PlcBackend>))
}

/// Opus PLC backend. The CPU default.
pub struct OpusPlc {
    decoder: OpusDecoder,
}

impl OpusPlc {
    pub fn new(config: &AudioConfig) -> Result<Self> {
        Ok(Self {
            decoder: OpusDecoder::new(config)?,
        })
    }
}

impl PlcBackend for OpusPlc {
    fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>> {
        Ok(self.decoder.decode(packet)?.to_vec())
    }

    fn decode_lost(&mut self) -> Result<Vec<i16>> {
        Ok(self.decoder.decode_lost()?.to_vec())
    }
}

/// Silence-on-loss baseline. Still decodes real packets normally; only
/// the concealment path returns zeros.
pub struct SilencePlc {
    decoder: OpusDecoder,
    frame_samples: usize,
}

impl SilencePlc {
    pub fn new(config: &AudioConfig) -> Result<Self> {
        let frame_samples = config.frame_size() * config.channels as usize;
        Ok(Self {
            decoder: OpusDecoder::new(config)?,
            frame_samples,
        })
    }
}

impl PlcBackend for SilencePlc {
    fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>> {
        Ok(self.decoder.decode(packet)?.to_vec())
    }

    fn decode_lost(&mut self) -> Result<Vec<i16>> {
        Ok(vec![0i16; self.frame_samples])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::OpusEncoder;
    use bytes::Bytes;

    fn make_encoded(config: &AudioConfig, seed: i32) -> Bytes {
        let mut enc = OpusEncoder::new(config).expect("encoder");
        let n = config.frame_size() * config.channels as usize;
        let pcm: Vec<i16> = (0..n)
            .map(|i| ((i as i32 * 7 + seed) % 32_000) as i16)
            .collect();
        enc.encode(&pcm).expect("encode")
    }

    #[test]
    fn opus_plc_decodes_and_conceals_in_frame_shape() {
        let cfg = AudioConfig::default();
        let mut plc = OpusPlc::new(&cfg).expect("plc");
        let packet = make_encoded(&cfg, 7);
        let expected = cfg.frame_size() * cfg.channels as usize;
        assert_eq!(plc.decode(&packet).expect("decode").len(), expected);
        assert_eq!(plc.decode_lost().expect("conceal").len(), expected);
    }

    #[test]
    fn silence_plc_emits_zeros_on_loss() {
        let cfg = AudioConfig::default();
        let mut plc = SilencePlc::new(&cfg).expect("plc");
        let concealed = plc.decode_lost().expect("conceal");
        let expected = cfg.frame_size() * cfg.channels as usize;
        assert_eq!(concealed.len(), expected);
        assert!(concealed.iter().all(|&s| s == 0));
    }

    #[test]
    fn trait_object_round_trip() {
        let cfg = AudioConfig::default();
        let mut backend: Box<dyn PlcBackend> =
            Box::new(OpusPlc::new(&cfg).expect("plc"));
        let packet = make_encoded(&cfg, 3);
        backend.decode(&packet).expect("decode via trait");
        backend.decode_lost().expect("conceal via trait");
        assert!(!backend.supports_speculation());
    }

    #[test]
    fn default_factory_produces_opus_backend() {
        let cfg = AudioConfig::default();
        let factory = default_plc_factory();
        let mut backend = factory(&cfg).expect("factory");
        let packet = make_encoded(&cfg, 11);
        assert_eq!(
            backend.decode(&packet).expect("decode").len(),
            cfg.frame_size() * cfg.channels as usize
        );
    }
}
