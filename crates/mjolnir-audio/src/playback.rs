use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::AudioConfig;

/// Plays back decoded PCM audio to the default output device.
pub struct AudioPlayback {
    stream: cpal::Stream,
    tx: mpsc::Sender<Vec<i16>>,
}

impl AudioPlayback {
    /// Start playback. Send decoded PCM frames to the returned sender.
    pub fn start(config: &AudioConfig) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no output device available")?;

        info!("using output device: {:?}", device.description());

        let stream_config = cpal::StreamConfig {
            channels: config.channels,
            sample_rate: config.sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let (tx, mut rx) = mpsc::channel::<Vec<i16>>(32);

        // Ring buffer shared between async receiver and audio callback
        let ring = Arc::new(Mutex::new(std::collections::VecDeque::<i16>::with_capacity(
            config.frame_size() * config.channels as usize * 8,
        )));

        // Drain mpsc into ring buffer on a background task
        let ring_writer = ring.clone();
        tokio::spawn(async move {
            while let Some(samples) = rx.recv().await {
                if let Ok(mut buf) = ring_writer.lock() {
                    buf.extend(samples);
                    // Cap at ~200ms of buffered audio
                    while buf.len() > 48000 / 5 {
                        buf.pop_front();
                    }
                }
            }
        });

        let ring_reader = ring;
        let stream = device.build_output_stream(
            &stream_config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                if let Ok(mut buf) = ring_reader.lock() {
                    for sample in data.iter_mut() {
                        *sample = buf.pop_front().unwrap_or(0);
                    }
                } else {
                    data.fill(0);
                }
            },
            move |err| {
                warn!("audio playback error: {err}");
            },
            None,
        )?;

        stream.play()?;
        info!("audio playback started");

        Ok(Self { stream, tx })
    }

    /// Get a sender to push decoded PCM frames for playback.
    pub fn sender(&self) -> mpsc::Sender<Vec<i16>> {
        self.tx.clone()
    }
}
