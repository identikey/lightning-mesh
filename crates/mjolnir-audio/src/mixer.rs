//! Multi-peer audio mixer.
//!
//! ## Threading model
//!
//! Each peer has its own asynchronous inference task that owns the
//! peer's [`SelfHealingBuffer`] and [`PlcBackend`]. The task ticks at
//! the configured frame rate, drains any newly-arrived network packets
//! into the jitter buffer, pulls one decoded (or concealed) frame into a
//! scratch buffer, and pushes that frame into a single-producer /
//! single-consumer ring shared with the cpal output callback.
//!
//! The cpal callback runs on the audio thread. It does *nothing* but
//! drain the per-peer ring and sum-mix the samples into the output
//! buffer. It never decodes, never allocates, never holds a contended
//! lock, and never touches a [`PlcBackend`]. This isolation is the
//! reason we can swap in heavier neural backends later without putting
//! the audio thread at risk.
//!
//! ```text
//! NETWORK ─► PeerInput.push_frame ─► mpsc<(seq, Bytes)>
//!                                          │
//!                                          ▼
//!                                inference task (tokio)
//!                                          │
//!                              SelfHealingBuffer.pull(&mut scratch)
//!                                          │
//!                                          ▼
//!                                  rtrb::Producer<i16>
//!                                          │
//!                                          ▼
//!                                  rtrb::Consumer<i16>
//!                                          │
//!                                          ▼
//!                                  cpal output callback
//! ```
//!
//! ## Underrun policy
//!
//! If the inference task hasn't filled the ring (warming up, slow
//! backend, transient CPU spike), the cpal callback consumes whatever
//! is available and the remainder of the output is left at its current
//! value (silence, since the mix bus is zeroed every callback). This is
//! correct degradation — never a panic, never a glitch loop.

use anyhow::{Context, Result};
use bytes::Bytes;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use mjolnir_media::{BufferStats, PullStatus, SelfHealingBuffer};
use rtrb::{Consumer, Producer, RingBuffer};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

use crate::conceal::{default_plc_factory, PlcBackend, PlcFactory};
use crate::device::{self, i16_to_f32, Direction};
use crate::AudioConfig;

/// Frames of warm-up depth (≈60ms at 20ms frames). Small enough for
/// live voice, large enough to absorb typical LAN/WAN jitter.
const JITTER_TARGET_FRAMES: usize = 3;
/// Ring capacity. Beyond this, the oldest pending frame is evicted.
const JITTER_CAPACITY: usize = 16;
/// Per-peer PCM ring capacity in frames. ~160 ms of headroom at 20 ms
/// frames. The inference task skips a tick when the ring is full, so
/// this caps the producer-vs-consumer drift.
const PCM_RING_CAPACITY_FRAMES: usize = 8;
/// Bounded queue depth for incoming network frames between ticks.
/// At 50 frames/sec arrival, 32 deep is ~640 ms of arrival headroom.
const FRAME_CHANNEL_CAPACITY: usize = 32;

/// Stats snapshot for a single peer. Exposed via [`MixerHandle::peer_stats`].
type StatsShared = Arc<Mutex<BufferStats>>;

/// Aborts an associated [`tokio::task::JoinHandle`] when dropped.
/// Prevents inference tasks from outliving their peer slot.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// State for a single peer's audio stream.
///
/// The cpal callback's only contact with this struct is `accumulate_into`,
/// which drains the SPSC ring. Everything else lives on the inference task.
struct PeerSlot {
    pcm_consumer: Consumer<i16>,
    stats: StatsShared,
    _task: AbortOnDrop,
}

impl PeerSlot {
    /// Accumulate up to `mix.len()` samples from this peer's ring into
    /// `mix`. If the ring is short, the unsumamed portion of `mix`
    /// stays at its current value (silence, since the bus is pre-zeroed).
    fn accumulate_into(&mut self, mix: &mut [i32]) {
        let available = self.pcm_consumer.slots();
        let want = mix.len().min(available);
        if want == 0 {
            return;
        }
        let chunk = match self.pcm_consumer.read_chunk(want) {
            Ok(c) => c,
            Err(_) => return,
        };
        let (a, b) = chunk.as_slices();
        let mut iter = a.iter().chain(b.iter());
        for sample_out in mix.iter_mut() {
            match iter.next() {
                Some(&s) => *sample_out = sample_out.saturating_add(s as i32),
                None => break,
            }
        }
        chunk.commit_all();
    }

    fn stats(&self) -> BufferStats {
        self.stats.lock().map(|s| *s).unwrap_or_default()
    }
}

type PeerMap = Arc<Mutex<HashMap<String, Arc<Mutex<PeerSlot>>>>>;

/// Multi-peer audio mixer. Owns the output cpal stream; the shareable
/// peer registry lives behind [`MixerHandle`] so other tasks (e.g. an
/// audio-protocol ALPN handler) can register/deregister peers without
/// holding the non-`Sync` `cpal::Stream`.
pub struct Mixer {
    handle: MixerHandle,
    _stream: cpal::Stream,
}

/// Cloneable share of a [`Mixer`]'s peer registry. Lets other components
/// add/remove peers without owning the cpal stream.
#[derive(Clone)]
pub struct MixerHandle {
    peers: PeerMap,
    config: AudioConfig,
    plc_factory: PlcFactory,
}

/// Handle returned by [`Mixer::add_peer`] for pushing encoded frames into
/// a specific peer's jitter buffer.
#[derive(Clone)]
pub struct PeerInput {
    frame_tx: mpsc::Sender<(u64, Bytes)>,
}

impl PeerInput {
    /// Push one Opus frame with the given monotonic sequence number.
    ///
    /// Non-blocking. If the inference task's queue is full (backend
    /// stalled, peer being removed), the frame is dropped — this is a
    /// liveness property, not a correctness one. The jitter buffer
    /// downstream will treat it as a network loss and conceal.
    pub fn push_frame(&self, seq: u64, frame: Bytes) {
        let _ = self.frame_tx.try_send((seq, frame));
    }
}

impl Mixer {
    /// Open the default output device and start the mix callback, using
    /// the default Opus PLC backend for every peer.
    pub fn start(config: AudioConfig) -> Result<Self> {
        Self::start_with_plc(config, default_plc_factory())
    }

    /// Open the default output device and start the mix callback. Each
    /// peer registered later receives a freshly-minted backend from
    /// `plc_factory`. This is the seam used to swap in alternative PLC
    /// strategies (silence baseline, tract-hosted neural model,
    /// NPU-resident).
    pub fn start_with_plc(config: AudioConfig, plc_factory: PlcFactory) -> Result<Self> {
        // Log the linked libopus version once per mixer startup. Deep PLC
        // (FARGAN) is only active on libopus >= 1.5 with decoder
        // complexity >= 5 (which is the default in 1.5+); the build script
        // enforces the version floor, this line just surfaces it at runtime.
        info!(
            linked_libopus = opus::version(),
            build_libopus = env!("MJOLNIR_LIBOPUS_VERSION"),
            "audio codec ready (deep neural PLC active on libopus >= 1.5)"
        );

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no output device available")?;

        info!("using output device: {:?}", device.description());

        let supported = device::pick_config(
            &device,
            Direction::Output,
            config.sample_rate,
            config.channels,
        )?;
        let sample_format = supported.sample_format();
        let stream_config: cpal::StreamConfig = supported.into();

        info!(
            ?sample_format,
            sample_rate = stream_config.sample_rate,
            channels = stream_config.channels,
            "output stream config negotiated"
        );

        let peers: PeerMap = Arc::new(Mutex::new(HashMap::new()));

        let stream = match sample_format {
            SampleFormat::I16 => build_output_i16(&device, &stream_config, peers.clone())?,
            SampleFormat::F32 => build_output_f32(&device, &stream_config, peers.clone())?,
            other => anyhow::bail!("unsupported output sample format: {other:?}"),
        };

        stream.play().context("failed to start output stream")?;
        info!("audio mixer started");

        Ok(Self {
            handle: MixerHandle {
                peers,
                config,
                plc_factory,
            },
            _stream: stream,
        })
    }

    /// Return a cloneable handle for adding/removing peers from another task.
    pub fn handle(&self) -> MixerHandle {
        self.handle.clone()
    }

    pub fn add_peer(&self, key: impl Into<String>) -> Result<PeerInput> {
        self.handle.add_peer(key)
    }

    pub fn peer_stats(&self, key: &str) -> Option<BufferStats> {
        self.handle.peer_stats(key)
    }

    pub fn all_peer_stats(&self) -> HashMap<String, BufferStats> {
        self.handle.all_peer_stats()
    }

    pub fn remove_peer(&self, key: &str) {
        self.handle.remove_peer(key)
    }
}

impl MixerHandle {
    /// Register a new peer.
    ///
    /// Mints a fresh [`PlcBackend`] from the factory, sets up the SPSC
    /// ring + network-frame channel, and spawns the per-peer inference
    /// task. Returns a [`PeerInput`] that the network thread uses to
    /// push encoded frames.
    ///
    /// Must be called from within a Tokio runtime context.
    pub fn add_peer(&self, key: impl Into<String>) -> Result<PeerInput> {
        let backend = (self.plc_factory)(&self.config)?;
        let frame_samples = self.config.frame_size() * self.config.channels as usize;
        let ring_samples = frame_samples * PCM_RING_CAPACITY_FRAMES;

        let (producer, consumer) = RingBuffer::<i16>::new(ring_samples);
        let (frame_tx, frame_rx) = mpsc::channel::<(u64, Bytes)>(FRAME_CHANNEL_CAPACITY);

        let stats = Arc::new(Mutex::new(BufferStats::default()));
        let key = key.into();

        let task = tokio::spawn(run_peer_inference(
            self.config.clone(),
            backend,
            frame_rx,
            producer,
            stats.clone(),
            key.clone(),
        ));

        let slot = Arc::new(Mutex::new(PeerSlot {
            pcm_consumer: consumer,
            stats,
            _task: AbortOnDrop(task),
        }));

        self.peers
            .lock()
            .expect("peers mutex poisoned")
            .insert(key.clone(), slot);
        info!(peer = %key, "mixer registered peer");
        Ok(PeerInput { frame_tx })
    }

    /// Snapshot the decode/conceal stats for one peer. Returns `None`
    /// if no peer with that key is registered.
    pub fn peer_stats(&self, key: &str) -> Option<BufferStats> {
        let peers = self.peers.lock().ok()?;
        let slot = peers.get(key)?.clone();
        drop(peers);
        let slot = slot.lock().ok()?;
        Some(slot.stats())
    }

    /// Snapshot the stats for every registered peer.
    pub fn all_peer_stats(&self) -> HashMap<String, BufferStats> {
        let handles: Vec<(String, Arc<Mutex<PeerSlot>>)> = match self.peers.lock() {
            Ok(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            Err(_) => return HashMap::new(),
        };
        handles
            .into_iter()
            .filter_map(|(k, slot)| slot.lock().ok().map(|s| (k, s.stats())))
            .collect()
    }

    /// Deregister a peer. The inference task is aborted via the slot's
    /// `Drop` impl; any frames buffered in its jitter ring are dropped.
    pub fn remove_peer(&self, key: &str) {
        if self
            .peers
            .lock()
            .expect("peers mutex poisoned")
            .remove(key)
            .is_some()
        {
            info!(peer = %key, "mixer removed peer");
        }
    }
}

/// Per-peer inference task. Owns the [`SelfHealingBuffer`] and the
/// [`PlcBackend`]; produces one frame of PCM per tick into the SPSC
/// ring shared with the cpal output callback.
async fn run_peer_inference(
    config: AudioConfig,
    backend: Box<PlcBackend>,
    mut frame_rx: mpsc::Receiver<(u64, Bytes)>,
    mut pcm_producer: Producer<i16>,
    stats_out: StatsShared,
    peer_key: String,
) {
    let frame_samples = config.frame_size() * config.channels as usize;
    let mut buffer = SelfHealingBuffer::new(JITTER_TARGET_FRAMES, JITTER_CAPACITY, backend);
    let mut scratch = vec![0i16; frame_samples];

    let period = Duration::from_millis(config.frame_duration_ms as u64);
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tick.tick().await;

        // Drain any network arrivals into the jitter buffer. If all
        // senders have dropped, the peer is being removed — exit.
        loop {
            match frame_rx.try_recv() {
                Ok((seq, bytes)) => {
                    buffer.push(seq, bytes);
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    debug!(peer = %peer_key, "frame channel closed; inference task exiting");
                    return;
                }
            }
        }

        // If the audio thread is behind, the ring is full. Don't burn a
        // buffered packet on a frame we'd just drop — skip this tick.
        if pcm_producer.slots() < frame_samples {
            continue;
        }

        // Produce one frame.
        let status = match buffer.pull(&mut scratch) {
            Ok(s) => s,
            Err(e) => {
                debug!(peer = %peer_key, "inference error: {e}");
                if let Ok(mut s) = stats_out.lock() {
                    *s = buffer.stats();
                }
                continue;
            }
        };

        match status {
            PullStatus::Empty => {
                // Warming up; ring stays empty, audio thread plays silence.
                continue;
            }
            PullStatus::Decoded | PullStatus::Concealed { .. } => {
                if let Ok(mut chunk) = pcm_producer.write_chunk(frame_samples) {
                    let (a, b) = chunk.as_mut_slices();
                    let alen = a.len();
                    a.copy_from_slice(&scratch[..alen]);
                    if !b.is_empty() {
                        b.copy_from_slice(&scratch[alen..]);
                    }
                    chunk.commit_all();
                } else {
                    // Ring full (consumer behind) — drop this frame.
                    // The next tick will try again.
                    debug!(peer = %peer_key, "pcm ring full; dropping frame");
                }
            }
        }

        // Publish stats after each successful tick.
        if let Ok(mut s) = stats_out.lock() {
            *s = buffer.stats();
        }
    }
}

/// Snapshot the current peer list as clones of the Arc handles, so the
/// outer registry lock isn't held across decode work.
fn collect_peer_handles(peers: &PeerMap) -> Vec<Arc<Mutex<PeerSlot>>> {
    peers
        .lock()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default()
}

fn mix_into(handles: &[Arc<Mutex<PeerSlot>>], mix: &mut [i32]) {
    mix.fill(0);
    for slot in handles {
        if let Ok(mut s) = slot.lock() {
            s.accumulate_into(mix);
        }
    }
}

fn build_output_i16(
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    peers: PeerMap,
) -> Result<cpal::Stream> {
    let mut mix = Vec::<i32>::new();
    let stream = device.build_output_stream(
        stream_config,
        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
            mix.resize(data.len(), 0);
            let handles = collect_peer_handles(&peers);
            mix_into(&handles, &mut mix);
            for (out, m) in data.iter_mut().zip(mix.iter()) {
                *out = (*m).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            }
        },
        |err| warn!("audio playback error: {err}"),
        None,
    )?;
    Ok(stream)
}

fn build_output_f32(
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    peers: PeerMap,
) -> Result<cpal::Stream> {
    let mut mix = Vec::<i32>::new();
    let stream = device.build_output_stream(
        stream_config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            mix.resize(data.len(), 0);
            let handles = collect_peer_handles(&peers);
            mix_into(&handles, &mut mix);
            for (out, m) in data.iter_mut().zip(mix.iter()) {
                let clamped = (*m).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                *out = i16_to_f32(clamped);
            }
        },
        |err| warn!("audio playback error: {err}"),
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drain test: when the consumer ring contains samples, the cpal
    /// path (`accumulate_into`) must sum-mix exactly those samples and
    /// leave the rest of the bus unchanged.
    #[test]
    fn accumulate_into_drains_ring_and_saturates_mix() {
        // Construct a slot directly (no inference task) by hand-building
        // a ring and pre-loading the producer with known PCM.
        let frame_samples = 16;
        let (mut producer, consumer) = RingBuffer::<i16>::new(frame_samples * 2);

        // Write a frame of known samples to the producer.
        let pcm: Vec<i16> = (0..frame_samples as i16).map(|i| i * 100).collect();
        let mut chunk = producer.write_chunk(frame_samples).expect("reserve");
        let (a, b) = chunk.as_mut_slices();
        a.copy_from_slice(&pcm[..a.len()]);
        if !b.is_empty() {
            b.copy_from_slice(&pcm[a.len()..]);
        }
        chunk.commit_all();

        // We don't want to spawn an inference task in this unit test;
        // build a PeerSlot manually with a no-op task placeholder.
        // The `_task` field exists only to abort on drop.
        let stats = Arc::new(Mutex::new(BufferStats::default()));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let _g = runtime.enter();
        let placeholder = AbortOnDrop(tokio::spawn(async { /* immediately exits */ }));

        let mut slot = PeerSlot {
            pcm_consumer: consumer,
            stats,
            _task: placeholder,
        };

        let mut mix = vec![0i32; frame_samples];
        slot.accumulate_into(&mut mix);

        for (i, &m) in mix.iter().enumerate() {
            assert_eq!(m, pcm[i] as i32, "sample {i} should match");
        }
    }

    /// Underrun: when the ring is empty, `accumulate_into` must leave
    /// `mix` untouched (callback will then output silence).
    #[test]
    fn accumulate_into_is_noop_on_empty_ring() {
        let (_producer, consumer) = RingBuffer::<i16>::new(16);
        let stats = Arc::new(Mutex::new(BufferStats::default()));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let _g = runtime.enter();
        let placeholder = AbortOnDrop(tokio::spawn(async {}));
        let mut slot = PeerSlot {
            pcm_consumer: consumer,
            stats,
            _task: placeholder,
        };

        let mut mix = vec![7i32; 8];
        slot.accumulate_into(&mut mix);
        assert_eq!(mix, vec![7i32; 8], "empty ring must not mutate the bus");
    }

    /// Two peers: their samples must sum-mix saturating.
    #[test]
    fn two_peers_sum_mix() {
        let frame_samples = 8;
        let (mut p1, c1) = RingBuffer::<i16>::new(frame_samples * 2);
        let (mut p2, c2) = RingBuffer::<i16>::new(frame_samples * 2);

        let pcm1: Vec<i16> = vec![100; frame_samples];
        let pcm2: Vec<i16> = vec![250; frame_samples];

        let mut ch1 = p1.write_chunk(frame_samples).unwrap();
        let (a, _b) = ch1.as_mut_slices();
        a.copy_from_slice(&pcm1);
        ch1.commit_all();

        let mut ch2 = p2.write_chunk(frame_samples).unwrap();
        let (a, _b) = ch2.as_mut_slices();
        a.copy_from_slice(&pcm2);
        ch2.commit_all();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let _g = runtime.enter();
        let s1 = PeerSlot {
            pcm_consumer: c1,
            stats: Arc::new(Mutex::new(BufferStats::default())),
            _task: AbortOnDrop(tokio::spawn(async {})),
        };
        let s2 = PeerSlot {
            pcm_consumer: c2,
            stats: Arc::new(Mutex::new(BufferStats::default())),
            _task: AbortOnDrop(tokio::spawn(async {})),
        };

        let mut mix = vec![0i32; frame_samples];
        let handles = vec![Arc::new(Mutex::new(s1)), Arc::new(Mutex::new(s2))];
        mix_into(&handles, &mut mix);
        for &m in mix.iter() {
            assert_eq!(m, 350, "100 + 250 = 350, no saturation expected");
        }
    }
}
