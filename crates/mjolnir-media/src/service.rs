//! Composition of [`JitterBuffer`] with a [`Recover`] backend.
//!
//! `SelfHealingBuffer` is the "Redis-server-style" data structure
//! described in mjolnir-mesh's `docs/architecture/self-healing-jitter-buffer.md`:
//! a long-running owner of recent-encoded frames plus a warm
//! decoder/concealer that turns both delivered and missing packets into
//! a coherent stream of decoded media units. The consumer pulls at the
//! playout cadence and never sees the difference between a received
//! frame and a concealed one — but [`Pulled`] preserves provenance so
//! cross-fade and stats are possible downstream.

use anyhow::Result;
use bytes::Bytes;

use crate::jitter::{JitterBuffer, Pull, PushOutcome};
use crate::recover::Recover;

/// Outcome of [`SelfHealingBuffer::pull`].
///
/// Distinct from [`Pull`] in that it carries the *decoded* unit, not the
/// raw encoded payload, and distinguishes received vs concealed for
/// downstream consumers (mixer stats, cross-fade transitions, debug logs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pulled<T> {
    /// Buffer is warming up to its target depth; no unit produced.
    Empty,
    /// A received packet was decoded normally.
    Decoded(T),
    /// The expected packet was missing; the backend synthesised this
    /// unit (codec PLC, FEC lookahead recovery, or neural prediction).
    Concealed(T),
}

impl<T> Pulled<T> {
    /// Convert to an `Option<T>`, discarding provenance.
    pub fn into_unit(self) -> Option<T> {
        match self {
            Pulled::Empty => None,
            Pulled::Decoded(t) | Pulled::Concealed(t) => Some(t),
        }
    }

    pub fn was_concealed(&self) -> bool {
        matches!(self, Pulled::Concealed(_))
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Pulled::Empty)
    }
}

/// Running counts of buffer activity. Useful for "is PLC engaging?"
/// observability without piping events out of the audio thread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BufferStats {
    /// Frames produced from a received packet.
    pub decoded: u64,
    /// Frames produced from concealment (codec PLC or FEC).
    pub concealed: u64,
    /// Of `concealed`, the count recovered via FEC lookahead rather
    /// than pure codec PLC. (Backends must report this themselves; the
    /// buffer can't tell which mechanism the backend used.)
    pub fec_recovered: u64,
    /// Backend errors during decode or conceal.
    pub errors: u64,
}

pub struct SelfHealingBuffer<R: Recover> {
    jitter: JitterBuffer<Bytes>,
    recover: R,
    stats: BufferStats,
}

impl<R: Recover> SelfHealingBuffer<R> {
    pub fn new(target_depth: usize, capacity: usize, recover: R) -> Self {
        Self {
            jitter: JitterBuffer::new(target_depth, capacity),
            recover,
            stats: BufferStats::default(),
        }
    }

    /// Insert a freshly-arrived encoded packet at sequence `seq`.
    pub fn push(&mut self, seq: u64, packet: Bytes) -> PushOutcome {
        self.jitter.push(seq, packet)
    }

    /// Pull the next decoded unit.
    ///
    /// On a [`Pull::Gap`], the buffer peeks the next-in-sequence slot
    /// (non-destructively) and hands it to the backend's
    /// [`Recover::decode_lost`] as a lookahead. Codecs supporting FEC
    /// can recover the lost frame from the next packet's FEC payload;
    /// codecs that don't ignore the hint and fall back to codec-native
    /// concealment.
    pub fn pull(&mut self) -> Result<Pulled<R::Output>> {
        match self.jitter.pull() {
            Pull::Frame(bytes) => match self.recover.decode(&bytes) {
                Ok(out) => {
                    self.stats.decoded += 1;
                    Ok(Pulled::Decoded(out))
                }
                Err(e) => {
                    self.stats.errors += 1;
                    Err(e)
                }
            },
            Pull::Gap => {
                let lookahead = self.jitter.peek_next().map(|b| b.as_ref());
                match self.recover.decode_lost(lookahead) {
                    Ok(out) => {
                        self.stats.concealed += 1;
                        if lookahead.is_some() {
                            // Backends MAY have used FEC; we record the
                            // opportunity. Whether the backend actually
                            // did FEC vs codec-PLC is opaque here.
                            self.stats.fec_recovered += 1;
                        }
                        Ok(Pulled::Concealed(out))
                    }
                    Err(e) => {
                        self.stats.errors += 1;
                        Err(e)
                    }
                }
            }
            Pull::Empty => Ok(Pulled::Empty),
        }
    }

    pub fn stats(&self) -> BufferStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.jitter.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jitter.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.jitter.capacity()
    }

    /// Clear all state. Stats are preserved (they're observational).
    pub fn reset(&mut self) {
        self.jitter.reset();
    }

    /// Reset stats counters to zero. Does not clear buffered packets.
    pub fn reset_stats(&mut self) {
        self.stats = BufferStats::default();
    }

    pub fn recover(&self) -> &R {
        &self.recover
    }

    pub fn recover_mut(&mut self) -> &mut R {
        &mut self.recover
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counting {
        decoded: u32,
        concealed: u32,
        last_lookahead: Option<Vec<u8>>,
    }

    impl Recover for Counting {
        type Output = (u32, bool);
        fn decode(&mut self, _packet: &[u8]) -> Result<Self::Output> {
            self.decoded += 1;
            Ok((self.decoded, false))
        }
        fn decode_lost(&mut self, lookahead: Option<&[u8]>) -> Result<Self::Output> {
            self.concealed += 1;
            self.last_lookahead = lookahead.map(|s| s.to_vec());
            Ok((self.concealed, true))
        }
    }

    fn pkt(b: u8) -> Bytes {
        Bytes::copy_from_slice(&[b])
    }

    #[test]
    fn warmup_returns_empty_until_target_depth() {
        let mut buf = SelfHealingBuffer::new(
            2,
            4,
            Counting { decoded: 0, concealed: 0, last_lookahead: None },
        );
        assert!(buf.pull().unwrap().is_empty());
        buf.push(0, pkt(0));
        assert!(buf.pull().unwrap().is_empty());
        buf.push(1, pkt(1));
        let pulled = buf.pull().unwrap();
        assert!(!pulled.was_concealed());
        assert!(matches!(pulled, Pulled::Decoded(_)));
    }

    #[test]
    fn gap_routes_through_decode_lost_with_lookahead() {
        let mut buf = SelfHealingBuffer::new(
            1,
            4,
            Counting { decoded: 0, concealed: 0, last_lookahead: None },
        );
        buf.push(0, pkt(0));
        buf.push(2, pkt(2)); // seq 1 skipped, seq 2 sits in the ring
        assert!(matches!(buf.pull().unwrap(), Pulled::Decoded(_))); // seq 0
        let pulled = buf.pull().unwrap();
        assert!(pulled.was_concealed(), "seq 1 should be concealed");
        // The lookahead handed to the backend should be the seq 2 bytes,
        // because seq 2 is the next available slot after the gap.
        assert_eq!(buf.recover().last_lookahead.as_deref(), Some(&[2u8][..]));
        // Seq 2 is still in the ring — should now decode normally.
        assert!(matches!(buf.pull().unwrap(), Pulled::Decoded(_)));
        assert_eq!(buf.recover().decoded, 2);
        assert_eq!(buf.recover().concealed, 1);
    }

    #[test]
    fn gap_without_lookahead_passes_none() {
        let mut buf = SelfHealingBuffer::new(
            1,
            4,
            Counting { decoded: 0, concealed: 0, last_lookahead: None },
        );
        buf.push(0, pkt(0));
        assert!(matches!(buf.pull().unwrap(), Pulled::Decoded(_))); // seq 0
        // Nothing else in the ring; pull at seq 1 → gap, no lookahead.
        let pulled = buf.pull().unwrap();
        assert!(pulled.was_concealed());
        assert!(buf.recover().last_lookahead.is_none());
    }

    #[test]
    fn stats_accumulate() {
        let mut buf = SelfHealingBuffer::new(
            1,
            4,
            Counting { decoded: 0, concealed: 0, last_lookahead: None },
        );
        buf.push(0, pkt(0));
        buf.push(2, pkt(2));
        let _ = buf.pull(); // decoded seq 0
        let _ = buf.pull(); // concealed seq 1 (with lookahead)
        let _ = buf.pull(); // decoded seq 2
        let stats = buf.stats();
        assert_eq!(stats.decoded, 2);
        assert_eq!(stats.concealed, 1);
        assert_eq!(stats.fec_recovered, 1, "lookahead was present");
        assert_eq!(stats.errors, 0);
    }

    #[test]
    fn boxed_backend_satisfies_recover_via_blanket_impl() {
        let mut buf: SelfHealingBuffer<Box<dyn Recover<Output = (u32, bool)>>> =
            SelfHealingBuffer::new(
                1,
                4,
                Box::new(Counting { decoded: 0, concealed: 0, last_lookahead: None }),
            );
        buf.push(0, pkt(0));
        let pulled = buf.pull().unwrap();
        assert!(!pulled.was_concealed());
    }
}
