//! Decode-and-conceal seam shared by all media types.
//!
//! `Recover` turns received wire bytes into decoded media units *and*
//! synthesises a fill unit when a packet is missing. Both responsibilities
//! live on one trait because codec-native PLC (audio: Opus's
//! `decode(None, ..)`; video: hypothetical group-resync logic) depends on
//! state that the same backend's `decode` populates; splitting them would
//! force expensive state mirroring.
//!
//! Audio implements this with `Output = Vec<i16>`; future video would
//! implement it with whatever frame type makes sense there.

use anyhow::Result;

pub trait Recover: Send {
    /// The decoded media unit type. PCM samples for audio; decoded frames
    /// (NAL units, YUV planes, whatever) for video.
    type Output;

    /// Decode a freshly-arrived encoded packet.
    fn decode(&mut self, packet: &[u8]) -> Result<Self::Output>;

    /// Synthesise output for a missing packet.
    ///
    /// `lookahead`, when present, is the next-in-sequence packet that has
    /// already arrived. Codecs supporting forward error correction
    /// (Opus's in-band FEC, redundant video slices) can decode the lost
    /// frame from the lookahead's FEC payload. Backends that don't
    /// support FEC should ignore the hint and fall back to codec-native
    /// concealment.
    ///
    /// The hint is non-destructive: the lookahead packet is left in the
    /// buffer and will be returned by the next [`Recover::decode`] call.
    fn decode_lost(&mut self, lookahead: Option<&[u8]>) -> Result<Self::Output>;

    /// Whether this backend benefits from pre-emptive prediction.
    ///
    /// Backends that can predict for free (e.g. NPU-resident cascades
    /// running every cycle anyway) return `true`. The service may then
    /// speculate ahead, discarding the prediction on successful arrival.
    fn supports_speculation(&self) -> bool {
        false
    }
}

/// Blanket impl so `Box<dyn Recover<Output = T>>` itself satisfies `Recover`,
/// which lets [`SelfHealingBuffer`](crate::SelfHealingBuffer) be parameterised
/// over a boxed trait object without an extra wrapper.
impl<R: ?Sized + Recover> Recover for Box<R> {
    type Output = R::Output;

    fn decode(&mut self, packet: &[u8]) -> Result<Self::Output> {
        (**self).decode(packet)
    }

    fn decode_lost(&mut self, lookahead: Option<&[u8]>) -> Result<Self::Output> {
        (**self).decode_lost(lookahead)
    }

    fn supports_speculation(&self) -> bool {
        (**self).supports_speculation()
    }
}
