use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::AudioConfig;

/// Captures audio from the default input device and sends PCM frames.
pub struct AudioCapture {
    stream: cpal::Stream,
}

impl AudioCapture {
    /// Start capturing audio. Sends complete frames (frame_size samples) over the channel.
    pub fn start(config: &AudioConfig) -> Result<(Self, mpsc::Receiver<Vec<i16>>)> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no input device available")?;

        info!("using input device: {:?}", device.description());

        let stream_config = cpal::StreamConfig {
            channels: config.channels,
            sample_rate: config.sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let frame_size = config.frame_size() * config.channels as usize;
        let (tx, rx) = mpsc::channel::<Vec<i16>>(32);

        // Accumulate samples into complete frames
        let mut frame_buf: Vec<i16> = Vec::with_capacity(frame_size);

        let stream = device.build_input_stream(
            &stream_config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                frame_buf.extend_from_slice(data);

                while frame_buf.len() >= frame_size {
                    let frame: Vec<i16> = frame_buf.drain(..frame_size).collect();
                    if tx.try_send(frame).is_err() {
                        debug!("capture channel full, dropping frame");
                    }
                }
            },
            move |err| {
                warn!("audio capture error: {err}");
            },
            None,
        )?;

        stream.play()?;
        info!("audio capture started");

        Ok((Self { stream }, rx))
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        info!("audio capture stopped");
    }
}
