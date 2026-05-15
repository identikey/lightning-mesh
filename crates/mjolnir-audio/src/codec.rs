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

        // Enable in-band FEC. The encoder reserves a small portion of each
        // packet for FEC data covering the *previous* frame, which the
        // receiver can use to reconstruct a lost frame from the next
        // arrival (see `OpusDecoder::decode_fec`). Cost: small bitrate
        // overhead and a slight encoding-quality hit at the configured
        // bitrate. Standard for production voice.
        encoder
            .set_inband_fec(true)
            .context("failed to enable inband FEC")?;
        // Tell the encoder what loss rate to optimise FEC redundancy for.
        // 5% is a reasonable default for mixed LAN/WAN voice; the encoder
        // adapts redundancy energy to this hint.
        encoder
            .set_packet_loss_perc(5)
            .context("failed to set packet loss perc")?;

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

    /// Generate one frame of packet loss concealment samples using the
    /// decoder's internal state alone (no FEC data). Used when no
    /// lookahead packet is available at playout time.
    pub fn decode_lost(&mut self) -> Result<&[i16]> {
        let n = self
            .decoder
            .decode(&[], &mut self.decode_buf, false)
            .context("opus PLC decode failed")?;

        Ok(&self.decode_buf[..n * self.config.channels as usize])
    }

    /// Reconstruct a lost frame from the FEC payload carried by the
    /// *next* in-sequence packet (Opus in-band FEC). Does not advance
    /// the decoder's primary state — the caller should still call
    /// [`Self::decode`] on the same packet at its scheduled slot.
    pub fn decode_fec(&mut self, next_packet: &[u8]) -> Result<&[i16]> {
        let n = self
            .decoder
            .decode(next_packet, &mut self.decode_buf, true)
            .context("opus FEC decode failed")?;

        Ok(&self.decode_buf[..n * self.config.channels as usize])
    }

    pub fn config(&self) -> &AudioConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioConfig;

    #[test]
    fn opus_encode_decode_roundtrip() {
        let config = AudioConfig::default();
        let mut encoder = OpusEncoder::new(&config).expect("encoder creation failed");
        let mut decoder = OpusDecoder::new(&config).expect("decoder creation failed");

        // Generate a sine wave PCM buffer (one frame worth)
        let frame_size = config.frame_size() * config.channels as usize;
        let pcm: Vec<i16> = (0..frame_size)
            .map(|i| {
                let t = i as f64 / config.sample_rate as f64;
                (f64::sin(t * 440.0 * 2.0 * std::f64::consts::PI) * 16000.0) as i16
            })
            .collect();

        let encoded = encoder.encode(&pcm).expect("encode failed");
        assert!(!encoded.is_empty(), "encoded packet should not be empty");

        let decoded = decoder.decode(&encoded).expect("decode failed");
        assert_eq!(
            decoded.len(),
            pcm.len(),
            "decoded length should match input length"
        );
        // Don't compare exact values - Opus is lossy
    }
}
