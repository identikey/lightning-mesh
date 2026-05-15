//! Multi-peer audio mixer.
//!
//! Each subscribed peer gets its own [`PeerInput`] which pushes encoded
//! Opus frames into a per-peer jitter buffer. The cpal output callback
//! polls every peer at the audio clock rate, decodes (or PLC on a gap),
//! and sums the streams into the output device.

use anyhow::{Context, Result};
use bytes::Bytes;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use mjolnir_media::{BufferStats, Pulled, SelfHealingBuffer};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use crate::conceal::{default_plc_factory, PlcBackend, PlcFactory};
use crate::device::{self, i16_to_f32, Direction};
use crate::AudioConfig;

/// Frames of warm-up depth (≈60ms at 20ms frames). Small enough for live
/// voice, large enough to absorb typical LAN/WAN jitter.
const JITTER_TARGET_FRAMES: usize = 3;
/// Ring capacity. Beyond this, the oldest pending frame is evicted.
const JITTER_CAPACITY: usize = 16;

/// State for a single peer's audio stream.
///
/// Wraps a [`SelfHealingBuffer`] from `mjolnir-media`, which owns both
/// the jitter ring and the [`PlcBackend`] decode-and-conceal backend.
/// Swapping in a different backend (silence baseline, neural CPU,
/// NPU-resident) requires no changes here — see
/// [`Mixer::start_with_plc`].
struct PeerSlot {
    buffer: SelfHealingBuffer<Box<PlcBackend>>,
    /// Decoded samples pending playback. Drained by the output callback at
    /// the device's sample-clock rate.
    pcm_tail: VecDeque<i16>,
}

impl PeerSlot {
    fn new(config: &AudioConfig, plc: Box<PlcBackend>) -> Result<Self> {
        let frame_size = config.frame_size() * config.channels as usize;
        Ok(Self {
            buffer: SelfHealingBuffer::new(JITTER_TARGET_FRAMES, JITTER_CAPACITY, plc),
            pcm_tail: VecDeque::with_capacity(frame_size * 4),
        })
    }

    fn push(&mut self, seq: u64, frame: Bytes) {
        self.buffer.push(seq, frame);
    }

    /// Accumulate this peer's PCM into `mix`. Contributes silence (no add)
    /// if the buffer is still warming up.
    fn accumulate_into(&mut self, mix: &mut [i32]) {
        for sample_out in mix.iter_mut() {
            if self.pcm_tail.is_empty() {
                self.fill_tail();
            }
            if let Some(s) = self.pcm_tail.pop_front() {
                *sample_out = sample_out.saturating_add(s as i32);
            }
        }
    }

    /// Pull one decoded frame (or a concealed one) from the buffer and
    /// push its samples into `pcm_tail`. No-op while warming up.
    fn fill_tail(&mut self) {
        match self.buffer.pull() {
            Ok(Pulled::Decoded(samples)) | Ok(Pulled::Concealed(samples)) => {
                self.pcm_tail.extend(samples);
            }
            Ok(Pulled::Empty) => {} // warming up
            Err(e) => debug!("decode error: {e}"),
        }
    }

    fn stats(&self) -> BufferStats {
        self.buffer.stats()
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
    slot: Arc<Mutex<PeerSlot>>,
}

impl PeerInput {
    /// Push one Opus frame with the given monotonic sequence number.
    pub fn push_frame(&self, seq: u64, frame: Bytes) {
        if let Ok(mut s) = self.slot.lock() {
            s.push(seq, frame);
        }
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
    /// strategies (silence baseline, neural CPU, NPU-resident).
    pub fn start_with_plc(config: AudioConfig, plc_factory: PlcFactory) -> Result<Self> {
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
    /// Register a new peer; returns a handle for the network task to push
    /// frames with. A fresh PLC backend is minted from the mixer's factory.
    pub fn add_peer(&self, key: impl Into<String>) -> Result<PeerInput> {
        let backend = (self.plc_factory)(&self.config)?;
        let slot = Arc::new(Mutex::new(PeerSlot::new(&self.config, backend)?));
        let key = key.into();
        self.peers
            .lock()
            .expect("peers mutex poisoned")
            .insert(key.clone(), slot.clone());
        info!(peer = %key, "mixer registered peer");
        Ok(PeerInput { slot })
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

    /// Deregister a peer. Any frames buffered in its jitter ring are dropped.
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
