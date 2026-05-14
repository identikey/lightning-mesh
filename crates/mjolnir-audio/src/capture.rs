use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::device::{self, f32_to_i16, Direction};
use crate::AudioConfig;

/// Captures audio from the default input device and sends PCM frames as `i16`.
///
/// Negotiates the device format: prefers `i16` natively, falls back to `f32`
/// with on-the-fly conversion.
pub struct AudioCapture {
    _stream: cpal::Stream,
}

impl AudioCapture {
    /// Start capturing audio. Sends complete frames (`frame_size * channels`
    /// samples) over the channel.
    pub fn start(config: &AudioConfig) -> Result<(Self, mpsc::Receiver<Vec<i16>>)> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no input device available")?;

        info!("using input device: {:?}", device.description());

        let supported = device::pick_config(
            &device,
            Direction::Input,
            config.sample_rate,
            config.channels,
        )?;
        let sample_format = supported.sample_format();
        let stream_config: cpal::StreamConfig = supported.into();

        info!(
            ?sample_format,
            sample_rate = stream_config.sample_rate,
            channels = stream_config.channels,
            "input stream config negotiated"
        );

        let frame_size = config.frame_size() * config.channels as usize;
        let (tx, rx) = mpsc::channel::<Vec<i16>>(32);

        let stream = match sample_format {
            SampleFormat::I16 => build_input_i16(&device, &stream_config, frame_size, tx)?,
            SampleFormat::F32 => build_input_f32(&device, &stream_config, frame_size, tx)?,
            other => anyhow::bail!("unsupported input sample format: {other:?}"),
        };

        stream.play().context("failed to start input stream")?;
        info!("audio capture started");

        Ok((Self { _stream: stream }, rx))
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        info!("audio capture stopped");
    }
}

fn build_input_i16(
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    frame_size: usize,
    tx: mpsc::Sender<Vec<i16>>,
) -> Result<cpal::Stream> {
    let mut frame_buf: Vec<i16> = Vec::with_capacity(frame_size);
    let stream = device.build_input_stream(
        stream_config,
        move |data: &[i16], _: &cpal::InputCallbackInfo| {
            frame_buf.extend_from_slice(data);
            flush_frames(&mut frame_buf, frame_size, &tx);
        },
        |err| warn!("audio capture error: {err}"),
        None,
    )?;
    Ok(stream)
}

fn build_input_f32(
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    frame_size: usize,
    tx: mpsc::Sender<Vec<i16>>,
) -> Result<cpal::Stream> {
    let mut frame_buf: Vec<i16> = Vec::with_capacity(frame_size);
    let stream = device.build_input_stream(
        stream_config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            frame_buf.extend(data.iter().copied().map(f32_to_i16));
            flush_frames(&mut frame_buf, frame_size, &tx);
        },
        |err| warn!("audio capture error: {err}"),
        None,
    )?;
    Ok(stream)
}

fn flush_frames(buf: &mut Vec<i16>, frame_size: usize, tx: &mpsc::Sender<Vec<i16>>) {
    while buf.len() >= frame_size {
        let frame: Vec<i16> = buf.drain(..frame_size).collect();
        if tx.try_send(frame).is_err() {
            debug!("capture channel full, dropping frame");
        }
    }
}
