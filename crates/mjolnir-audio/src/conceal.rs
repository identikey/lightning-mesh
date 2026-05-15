//! Audio packet loss concealment backends.
//!
//! The trait used here lives in [`mjolnir_media::Recover`] — it is the
//! media-generic decode-and-conceal seam. This module provides the
//! audio-specific impls and a convenience type alias [`PlcBackend`] for
//! `dyn Recover<Output = Vec<i16>> + Send`.
//!
//! Two backends ship in-tree:
//!
//! * [`OpusPlc`] — the CPU default. Uses Opus's built-in decoder PLC
//!   ([`OpusDecoder::decode_lost`]), which draws on recent codec state to
//!   synthesise a smooth fill frame. Microsecond-class on a modern CPU.
//! * [`SilencePlc`] — a baseline that emits zeros on loss. Useful as a
//!   worst-case audibility reference and in tests.
//!
//! Future backends (neural PLC on CPU, AIE-resident cascade via parakeet-aie)
//! implement the same [`Recover`] trait. See
//! `docs/architecture/self-healing-jitter-buffer.md`.

use anyhow::Result;
use mjolnir_media::Recover;
use std::sync::Arc;

use crate::codec::OpusDecoder;
use crate::AudioConfig;

/// Audio-side alias for the boxed concealment backend.
///
/// `Box<PlcBackend>` is the storage shape used throughout the audio
/// pipeline. Concrete impls (Opus, silence, AIE later) implement
/// [`Recover<Output = Vec<i16>>`](mjolnir_media::Recover).
pub type PlcBackend = dyn Recover<Output = Vec<i16>> + Send;

/// Factory closure type used by [`Mixer`](crate::Mixer) to mint a fresh
/// per-peer backend.
pub type PlcFactory =
    Arc<dyn Fn(&AudioConfig) -> Result<Box<PlcBackend>> + Send + Sync>;

/// Default factory: one [`OpusPlc`] per peer.
pub fn default_plc_factory() -> PlcFactory {
    Arc::new(|cfg| Ok(Box::new(OpusPlc::new(cfg)?) as Box<PlcBackend>))
}

/// Factory that produces [`SilencePlc`] backends. Intended for tests and
/// dropout-audibility demos.
pub fn silence_plc_factory() -> PlcFactory {
    Arc::new(|cfg| Ok(Box::new(SilencePlc::new(cfg)?) as Box<PlcBackend>))
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

impl Recover for OpusPlc {
    type Output = Vec<i16>;

    fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>> {
        Ok(self.decoder.decode(packet)?.to_vec())
    }

    fn decode_lost(&mut self, lookahead: Option<&[u8]>) -> Result<Vec<i16>> {
        // If we have the next packet, use Opus's in-band FEC to
        // reconstruct the lost frame; the lookahead is left in the
        // buffer and decoded normally at its own scheduled slot.
        match lookahead {
            Some(next) => Ok(self.decoder.decode_fec(next)?.to_vec()),
            None => Ok(self.decoder.decode_lost()?.to_vec()),
        }
    }
}

/// Silence-on-loss baseline. Decodes real packets normally; the
/// concealment path returns zeros.
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

impl Recover for SilencePlc {
    type Output = Vec<i16>;

    fn decode(&mut self, packet: &[u8]) -> Result<Vec<i16>> {
        Ok(self.decoder.decode(packet)?.to_vec())
    }

    fn decode_lost(&mut self, _lookahead: Option<&[u8]>) -> Result<Vec<i16>> {
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
        // No lookahead -> codec-native PLC.
        assert_eq!(plc.decode_lost(None).expect("conceal").len(), expected);
    }

    #[test]
    fn opus_plc_recovers_via_fec_lookahead() {
        let cfg = AudioConfig::default();
        let mut plc = OpusPlc::new(&cfg).expect("plc");
        // Prime the decoder with one frame so internal state is realistic.
        let p0 = make_encoded(&cfg, 1);
        plc.decode(&p0).expect("decode");
        // Now simulate loss of seq 1 with seq 2 available as lookahead.
        let p2 = make_encoded(&cfg, 2);
        let recovered = plc.decode_lost(Some(&p2)).expect("fec recover");
        assert_eq!(recovered.len(), cfg.frame_size() * cfg.channels as usize);
    }

    #[test]
    fn silence_plc_emits_zeros_on_loss() {
        let cfg = AudioConfig::default();
        let mut plc = SilencePlc::new(&cfg).expect("plc");
        let concealed = plc.decode_lost(None).expect("conceal");
        let expected = cfg.frame_size() * cfg.channels as usize;
        assert_eq!(concealed.len(), expected);
        assert!(concealed.iter().all(|&s| s == 0));
        // Lookahead is ignored for silence backend.
        let dummy = make_encoded(&cfg, 9);
        let concealed2 = plc.decode_lost(Some(&dummy)).expect("conceal");
        assert!(concealed2.iter().all(|&s| s == 0));
    }

    #[test]
    fn trait_object_round_trip() {
        let cfg = AudioConfig::default();
        let mut backend: Box<PlcBackend> = Box::new(OpusPlc::new(&cfg).expect("plc"));
        let packet = make_encoded(&cfg, 3);
        backend.decode(&packet).expect("decode via trait");
        backend.decode_lost(None).expect("conceal via trait");
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
