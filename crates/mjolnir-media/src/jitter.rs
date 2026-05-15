//! Sequence-keyed circular jitter buffer.
//!
//! Producers push frames as they arrive from the network; a consumer pulls
//! frames at a fixed clock rate (e.g. one Opus frame every 20ms driven by the
//! audio playback clock).
//!
//! The buffer:
//! * waits until `target_depth` frames are held before releasing the first
//!   one ("warm-up"), absorbing initial network jitter;
//! * stores frames in a ring of size `capacity`, indexed by `seq % capacity`;
//! * drops frames whose sequence number is already in the past
//!   ([`PushOutcome::DroppedLate`]);
//! * evicts the oldest pending frame when a far-future sequence number would
//!   overflow the ring ([`PushOutcome::EvictedOldest`]);
//! * returns [`Pull::Gap`] when the next expected sequence has not arrived by
//!   the time the consumer asks for it — the consumer is expected to invoke
//!   codec loss concealment (Opus PLC, etc.) for that slot.

use std::collections::VecDeque;

/// Result of a [`JitterBuffer::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    /// Frame stored in its slot.
    Stored,
    /// Frame arrived after its playout deadline and was discarded.
    DroppedLate,
    /// Frame stored, but one or more older frames were evicted to make room.
    EvictedOldest,
}

/// Result of a [`JitterBuffer::pull`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pull<T> {
    /// The next frame in sequence.
    Frame(T),
    /// The next sequence number was missing at pull time. The consumer should
    /// run loss concealment for this slot. The cursor still advances.
    Gap,
    /// Buffer is still warming up to `target_depth`, or has never received a
    /// frame. The consumer should output silence and try again next tick.
    Empty,
}

/// A sequence-keyed circular buffer with playout warm-up.
///
/// `T` is the frame payload (e.g. an encoded Opus packet as `bytes::Bytes`).
pub struct JitterBuffer<T> {
    slots: VecDeque<Option<T>>,
    capacity: usize,
    target_depth: usize,
    /// Sequence number we will release on the next `pull`. `None` until the
    /// first push initialises the cursor.
    next_seq: Option<u64>,
    /// Number of occupied slots.
    count: usize,
    /// True once `count` has reached `target_depth` for the first time.
    started: bool,
}

impl<T> JitterBuffer<T> {
    /// Create a new buffer.
    ///
    /// * `target_depth` — number of frames to accumulate before releasing
    ///   any. Roughly the playout delay in frames.
    /// * `capacity` — total ring size. Must be `>= target_depth` and at
    ///   least 1.
    pub fn new(target_depth: usize, capacity: usize) -> Self {
        assert!(capacity >= 1, "capacity must be >= 1");
        assert!(
            target_depth <= capacity,
            "target_depth ({target_depth}) must be <= capacity ({capacity})"
        );
        let mut slots = VecDeque::with_capacity(capacity);
        for _ in 0..capacity {
            slots.push_back(None);
        }
        Self {
            slots,
            capacity,
            target_depth,
            next_seq: None,
            count: 0,
            started: false,
        }
    }

    /// Push a frame at sequence number `seq`.
    pub fn push(&mut self, seq: u64, frame: T) -> PushOutcome {
        // First push initialises the playout cursor to this seq.
        let next = match self.next_seq {
            Some(n) => n,
            None => {
                self.next_seq = Some(seq);
                seq
            }
        };

        // During warm-up, if a frame arrives with seq < next we lower the
        // cursor so it isn't treated as late. This handles modest reordering
        // at the start of a stream. Once we've started releasing frames the
        // cursor is fixed and any seq < next is genuinely late.
        if seq < next {
            if self.started || (next - seq) >= self.capacity as u64 {
                return PushOutcome::DroppedLate;
            }
            self.next_seq = Some(seq);
        }

        // Too far in the future: evict oldest until it fits.
        let mut evicted = false;
        while seq.saturating_sub(self.next_seq.expect("initialised above")) >= self.capacity as u64
        {
            let n = self.next_seq.expect("initialised above");
            let idx = (n % self.capacity as u64) as usize;
            if self.slots[idx].take().is_some() {
                self.count -= 1;
                evicted = true;
            }
            self.next_seq = Some(n + 1);
        }

        let idx = (seq % self.capacity as u64) as usize;
        if self.slots[idx].is_none() {
            self.count += 1;
        }
        self.slots[idx] = Some(frame);

        if !self.started && self.count >= self.target_depth {
            self.started = true;
        }

        if evicted {
            PushOutcome::EvictedOldest
        } else {
            PushOutcome::Stored
        }
    }

    /// Pull the next frame.
    ///
    /// Returns [`Pull::Empty`] until warm-up completes. After warm-up,
    /// advances the playout cursor on every call regardless of whether a
    /// frame was present — i.e. consumers MUST call this at the playout
    /// clock rate.
    pub fn pull(&mut self) -> Pull<T> {
        if !self.started {
            return Pull::Empty;
        }
        let seq = self.next_seq.expect("started implies a seq cursor");
        let idx = (seq % self.capacity as u64) as usize;
        self.next_seq = Some(seq + 1);

        match self.slots[idx].take() {
            Some(frame) => {
                self.count -= 1;
                Pull::Frame(frame)
            }
            None => Pull::Gap,
        }
    }

    /// Non-destructive peek at the slot at the current playout cursor.
    ///
    /// Returns `Some(&frame)` if the next-expected sequence is present,
    /// `None` if it would be a [`Pull::Gap`] (the expected packet is
    /// missing) or if the buffer is still warming up.
    ///
    /// Used by [`SelfHealingBuffer`](crate::SelfHealingBuffer) for
    /// FEC lookahead: after a gap is detected, the buffer peeks the next
    /// slot to pass to the backend's concealment path as a recovery hint
    /// without removing it (it will still be returned by the subsequent
    /// `pull`).
    pub fn peek_next(&self) -> Option<&T> {
        if !self.started {
            return None;
        }
        let seq = self.next_seq?;
        let idx = (seq % self.capacity as u64) as usize;
        self.slots[idx].as_ref()
    }

    /// Number of frames currently held.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Total ring capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Reset to initial state (no frames, awaiting warm-up).
    pub fn reset(&mut self) {
        for s in self.slots.iter_mut() {
            *s = None;
        }
        self.count = 0;
        self.next_seq = None;
        self.started = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warmup_then_in_order_release() {
        let mut jb = JitterBuffer::<u32>::new(3, 8);
        assert_eq!(jb.pull(), Pull::Empty);
        assert_eq!(jb.push(0, 100), PushOutcome::Stored);
        assert_eq!(jb.pull(), Pull::Empty, "below target_depth");
        assert_eq!(jb.push(1, 101), PushOutcome::Stored);
        assert_eq!(jb.pull(), Pull::Empty, "still below target_depth");
        assert_eq!(jb.push(2, 102), PushOutcome::Stored);
        assert_eq!(jb.pull(), Pull::Frame(100));
        assert_eq!(jb.pull(), Pull::Frame(101));
        assert_eq!(jb.pull(), Pull::Frame(102));
        assert_eq!(jb.pull(), Pull::Gap, "no more frames, cursor advances");
    }

    #[test]
    fn out_of_order_arrival_is_reordered() {
        let mut jb = JitterBuffer::<u32>::new(2, 8);
        jb.push(2, 102);
        jb.push(0, 100);
        jb.push(1, 101);
        assert_eq!(jb.pull(), Pull::Frame(100));
        assert_eq!(jb.pull(), Pull::Frame(101));
        assert_eq!(jb.pull(), Pull::Frame(102));
    }

    #[test]
    fn late_frame_dropped() {
        let mut jb = JitterBuffer::<u32>::new(1, 4);
        jb.push(5, 500);
        assert_eq!(jb.pull(), Pull::Frame(500));
        // Next expected seq is 6; a frame at seq=5 is late.
        assert_eq!(jb.push(5, 999), PushOutcome::DroppedLate);
        // Frame at seq=4 is also late.
        assert_eq!(jb.push(4, 999), PushOutcome::DroppedLate);
    }

    #[test]
    fn far_future_evicts_oldest() {
        let mut jb = JitterBuffer::<u32>::new(2, 4);
        jb.push(0, 0);
        jb.push(1, 1);
        // Now push way in the future — should evict seqs 0..=(seq - capacity).
        let outcome = jb.push(10, 10);
        assert_eq!(outcome, PushOutcome::EvictedOldest);
        // After eviction the cursor should be at seq 7 (10 - capacity + 1 = 7).
        // We had frames at 0 and 1; both are gone. Slot for 10 is filled.
        assert_eq!(jb.pull(), Pull::Gap); // seq 7
        assert_eq!(jb.pull(), Pull::Gap); // seq 8
        assert_eq!(jb.pull(), Pull::Gap); // seq 9
        assert_eq!(jb.pull(), Pull::Frame(10));
    }

    #[test]
    fn gap_signals_missing_frame() {
        let mut jb = JitterBuffer::<u32>::new(2, 8);
        jb.push(0, 0);
        // skip seq 1
        jb.push(2, 2);
        assert_eq!(jb.pull(), Pull::Frame(0));
        assert_eq!(jb.pull(), Pull::Gap);
        assert_eq!(jb.pull(), Pull::Frame(2));
    }

    #[test]
    fn reset_clears_state() {
        let mut jb = JitterBuffer::<u32>::new(1, 4);
        jb.push(10, 10);
        assert_eq!(jb.pull(), Pull::Frame(10));
        jb.reset();
        assert_eq!(jb.pull(), Pull::Empty);
        jb.push(100, 100);
        assert_eq!(jb.pull(), Pull::Frame(100));
    }

    #[test]
    fn target_depth_one_releases_immediately() {
        let mut jb = JitterBuffer::<u32>::new(1, 4);
        jb.push(0, 42);
        assert_eq!(jb.pull(), Pull::Frame(42));
    }

    #[test]
    #[should_panic]
    fn target_depth_exceeding_capacity_panics() {
        let _: JitterBuffer<u32> = JitterBuffer::new(5, 4);
    }

    #[test]
    fn occupancy_accounting() {
        let mut jb = JitterBuffer::<u32>::new(2, 4);
        assert_eq!(jb.len(), 0);
        jb.push(0, 0);
        assert_eq!(jb.len(), 1);
        jb.push(1, 1);
        assert_eq!(jb.len(), 2);
        // Overwriting the same seq slot doesn't change count.
        jb.push(1, 11);
        assert_eq!(jb.len(), 2);
        assert_eq!(jb.pull(), Pull::Frame(0));
        assert_eq!(jb.len(), 1);
        assert_eq!(jb.pull(), Pull::Frame(11));
        assert_eq!(jb.len(), 0);
    }
}
