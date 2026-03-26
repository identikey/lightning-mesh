use anyhow::{Context, Result};
use bytes::Bytes;

use crate::AudioConfig;

/// Opus encoder wrapper.
pub struct OpusEncoder {
    encoder: opus::Encoder,
    config: AudioConfig,
    encode_buf: Vec<u8>,
}

impl OpusEncoder {
    pub fn new(config: &AudioConfig) -> Result<Self> {
        let channels = match config.channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            n => anyhow::bail!("unsupported channel count: {n}"),
        };

        let mut encoder = opus::Encoder::new(config.sample_rate, channels, opus::Application::Audio)
            .context("failed to create opus encoder")?;

        encoder
            .set_bitrate(opus::Bitrate::Bits(config.bitrate))
            .context("failed to set bitrate")?;

        // Max opus frame is ~4000 bytes; 4096 is safe
        let encode_buf = vec![0u8; 4096];

        Ok(Self {
            encoder,
            config: config.clone(),
            encode_buf,
        })
    }

    /// Encode a frame of PCM i16 samples. Returns the opus packet.
    pub fn encode(&mut self, pcm: &[i16]) -> Result<Bytes> {
        let n = self
            .encoder
            .encode(pcm, &mut self.encode_buf)
            .context("opus encode failed")?;

        Ok(Bytes::copy_from_slice(&self.encode_buf[..n]))
    }

    pub fn config(&self) -> &AudioConfig {
        &self.config
    }
}

/// Opus decoder wrapper.
pub struct OpusDecoder {
    decoder: opus::Decoder,
    config: AudioConfig,
    decode_buf: Vec<i16>,
}

impl OpusDecoder {
    pub fn new(config: &AudioConfig) -> Result<Self> {
        let channels = match config.channels {
            1 => opus::Channels::Mono,
            2 => opus::Channels::Stereo,
            n => anyhow::bail!("unsupported channel count: {n}"),
        };

        let decoder = opus::Decoder::new(config.sample_rate, channels)
            .context("failed to create opus decoder")?;

        let decode_buf = vec![0i16; config.frame_size() * config.channels as usize];

        Ok(Self {
            decoder,
            config: config.clone(),
            decode_buf,
        })
    }

    /// Decode an opus packet into PCM i16 samples.
    pub fn decode(&mut self, packet: &[u8]) -> Result<&[i16]> {
        let n = self
            .decoder
            .decode(packet, &mut self.decode_buf, false)
            .context("opus decode failed")?;

        Ok(&self.decode_buf[..n * self.config.channels as usize])
    }

    pub fn config(&self) -> &AudioConfig {
        &self.config
    }
}
