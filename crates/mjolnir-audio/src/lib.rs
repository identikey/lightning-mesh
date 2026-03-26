//! mjolnir-audio: Opus audio pipeline for mesh streaming.
//!
//! Provides capture → encode → publish and subscribe → decode → playback paths.

pub mod capture;
pub mod codec;
pub mod playback;

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
