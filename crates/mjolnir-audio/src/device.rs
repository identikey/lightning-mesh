//! Device format negotiation helpers.
//!
//! cpal devices expose a *range* of supported configurations. We want a
//! specific sample rate and channel count, and we prefer `i16` (no
//! conversion in the audio callback) but accept `f32` if that's all the
//! device offers (USB headsets often only do `f32`).

use anyhow::{anyhow, Result};
use cpal::traits::DeviceTrait;
use cpal::{Device, SampleFormat, SupportedStreamConfig};

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Input,
    Output,
}

/// Pick a supported config for `device` at `sample_rate` / `channels`,
/// preferring `i16` over `f32`.
pub fn pick_config(
    device: &Device,
    direction: Direction,
    sample_rate: u32,
    channels: u16,
) -> Result<SupportedStreamConfig> {
    let configs: Vec<_> = match direction {
        Direction::Input => device.supported_input_configs()?.collect(),
        Direction::Output => device.supported_output_configs()?.collect(),
    };

    let mut f32_fallback: Option<SupportedStreamConfig> = None;

    for cfg in configs {
        if cfg.channels() != channels {
            continue;
        }
        if cfg.min_sample_rate() > sample_rate || cfg.max_sample_rate() < sample_rate {
            continue;
        }
        let supported = cfg.with_sample_rate(sample_rate);
        match supported.sample_format() {
            SampleFormat::I16 => return Ok(supported),
            SampleFormat::F32 if f32_fallback.is_none() => {
                f32_fallback = Some(supported);
            }
            _ => {}
        }
    }

    f32_fallback.ok_or_else(|| {
        anyhow!(
            "no compatible {:?} config (need {}Hz / {}ch, i16 or f32)",
            direction,
            sample_rate,
            channels
        )
    })
}

#[inline]
pub fn f32_to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

#[inline]
pub fn i16_to_f32(s: i16) -> f32 {
    s as f32 / i16::MAX as f32
}
