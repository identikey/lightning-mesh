//! mjolnir-audio: Opus audio pipeline for mesh streaming.
//!
//! Capture → encode → publish on one side; subscribe → jitter-buffer →
//! decode → mix → playback on the other. Multi-peer aware: the [`Mixer`]
//! holds a per-peer jitter buffer and sums all peer streams into the
//! output device.

pub mod capture;
pub mod codec;
pub mod conceal;
pub mod device;
pub mod mixer;
pub mod publish;
pub mod subscribe;

pub use conceal::{
    default_plc_factory, silence_plc_factory, OpusPlc, PlcBackend, PlcFactory, SilencePlc,
};
pub use mixer::{Mixer, PeerInput};
pub use mjolnir_media::{BufferStats, Pulled};
pub use publish::AUDIO_TRACK_NAME;

/// Audio configuration for the mesh.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Sample rate in Hz. Opus supports 8000, 12000, 16000, 24000, 48000.
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo).
    pub channels: u16,
    /// Opus encoder bitrate in bits/sec.
    pub bitrate: i32,
    /// Frame duration in milliseconds. Opus supports 2.5, 5, 10, 20, 40, 60.
    pub frame_duration_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 1,
            bitrate: 64000,
            frame_duration_ms: 20,
        }
    }
}

impl AudioConfig {
    /// Number of samples per frame (per channel).
    pub fn frame_size(&self) -> usize {
        (self.sample_rate as usize * self.frame_duration_ms as usize) / 1000
    }
}
