//! Direct-iroh audio protocol — replaces the MoQ data plane.
//!
//! Each peer pair shares a single bidirectional QUIC stream. Each side
//! writes length-prefixed Opus packets at the local frame cadence and
//! reads the same on its recv half, pushing decoded payloads into the
//! mixer's per-peer jitter buffer.
//!
//! Wire format (one frame, repeated forever per stream):
//!
//! ```text
//! [u32 little-endian length][opus bytes]
//! ```
//!
//! Sequence numbers are *not* sent on the wire: QUIC streams preserve
//! order, so the recv pump just monotonically counts arrivals and feeds
//! that count to the jitter buffer (it still needs a seq to track gaps
//! caused by encoder/decoder errors, not by network loss).

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use bytes::Bytes;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use mjolnir_audio::codec::OpusEncoder;
use mjolnir_audio::{AudioConfig, MixerHandle, PeerInput};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// ALPN identifier for the mjolnir audio protocol.
pub const AUDIO_ALPN: &[u8] = b"mjolnir/audio/v1";

/// Hard cap on a single Opus packet on the wire. Real packets are well
/// under 1 KiB at our bitrate; this is just defensive parsing.
const MAX_FRAME_LEN: usize = 4096;

/// Per-room state the [`AudioHandler`] needs to handle inbound
/// connections. Bound after the room is created via [`AudioHandler::bind`].
#[derive(Clone)]
struct BoundContext {
    mixer_handle: MixerHandle,
    pcm_tx: broadcast::Sender<Vec<i16>>,
    audio_config: AudioConfig,
}

/// iroh `ProtocolHandler` for the audio ALPN. Constructed at mesh node
/// startup before any room exists; bound to a room's mixer and capture
/// broadcast when a room is entered.
#[derive(Clone)]
pub struct AudioHandler {
    inner: Arc<Mutex<Option<BoundContext>>>,
}

impl std::fmt::Debug for AudioHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bound = self.inner.lock().map(|g| g.is_some()).unwrap_or(false);
        f.debug_struct("AudioHandler").field("bound", &bound).finish()
    }
}

impl AudioHandler {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    /// Wire up the mixer + capture-broadcast for the current room.
    /// Subsequent inbound audio connections will register with this mixer.
    pub fn bind(
        &self,
        mixer_handle: MixerHandle,
        pcm_tx: broadcast::Sender<Vec<i16>>,
        audio_config: AudioConfig,
    ) {
        *self.inner.lock().expect("audio handler poisoned") = Some(BoundContext {
            mixer_handle,
            pcm_tx,
            audio_config,
        });
    }

    fn context(&self) -> Option<BoundContext> {
        self.inner.lock().ok()?.clone()
    }
}

impl Default for AudioHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolHandler for AudioHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer_id = connection.remote_id();
        let ctx = match self.context() {
            Some(c) => c,
            None => {
                warn!(%peer_id, "audio handler not bound to a room; closing connection");
                connection.close(0u32.into(), b"audio handler unbound");
                return Ok(());
            }
        };

        let (send, recv) = connection
            .accept_bi()
            .await
            .map_err(AcceptError::from_err)?;
        info!(%peer_id, "accepted inbound audio stream");

        run_session(
            peer_id.to_string(),
            send,
            recv,
            ctx.mixer_handle,
            ctx.pcm_tx,
            ctx.audio_config,
        )
        .await;

        // Keep the Connection alive until pumps end.
        drop(connection);
        Ok(())
    }
}

/// Dial a peer and run the audio session until either pump ends.
pub async fn dial_and_run(
    endpoint: Endpoint,
    addr: EndpointAddr,
    mixer_handle: MixerHandle,
    pcm_tx: broadcast::Sender<Vec<i16>>,
    audio_config: AudioConfig,
) -> Result<()> {
    let peer_id = addr.id;
    info!(%peer_id, "dialing audio stream");
    let conn = endpoint
        .connect(addr, AUDIO_ALPN)
        .await
        .context("audio dial failed")?;
    let (send, recv) = conn.open_bi().await.context("open_bi failed")?;
    info!(%peer_id, "audio stream open");

    run_session(
        peer_id.to_string(),
        send,
        recv,
        mixer_handle,
        pcm_tx,
        audio_config,
    )
    .await;
    drop(conn);
    Ok(())
}

/// Shared session driver used by both dial and accept paths. Registers
/// the peer with the mixer, spawns the send/recv pumps, awaits either's
/// completion, then deregisters.
async fn run_session(
    peer_key: String,
    send: SendStream,
    recv: RecvStream,
    mixer_handle: MixerHandle,
    pcm_tx: broadcast::Sender<Vec<i16>>,
    audio_config: AudioConfig,
) {
    let peer_input = match mixer_handle.add_peer(peer_key.clone()) {
        Ok(pi) => pi,
        Err(e) => {
            warn!(peer = %peer_key, "failed to register peer with mixer: {e}");
            return;
        }
    };

    let pcm_rx = pcm_tx.subscribe();
    let send_key = peer_key.clone();
    let send_task = tokio::spawn(async move {
        if let Err(e) = run_send_pump(audio_config, pcm_rx, send).await {
            warn!(peer = %send_key, "audio send pump ended: {e}");
        }
    });
    let recv_key = peer_key.clone();
    let recv_task = tokio::spawn(async move {
        if let Err(e) = run_recv_pump(recv, peer_input).await {
            warn!(peer = %recv_key, "audio recv pump ended: {e}");
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    mixer_handle.remove_peer(&peer_key);
}

/// Drain decoded PCM from a broadcast subscription, encode with Opus,
/// and write length-prefixed packets to `send`.
async fn run_send_pump(
    config: AudioConfig,
    mut pcm_rx: broadcast::Receiver<Vec<i16>>,
    mut send: SendStream,
) -> Result<()> {
    let mut encoder = OpusEncoder::new(&config)?;
    loop {
        let pcm = match pcm_rx.recv().await {
            Ok(p) => p,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(dropped = n, "send pump lagged behind capture");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let opus = encoder.encode(&pcm).context("opus encode failed")?;
        send.write_u32_le(opus.len() as u32)
            .await
            .context("write length")?;
        send.write_all(&opus).await.context("write opus payload")?;
        debug!(len = opus.len(), "sent audio frame");
    }
    Ok(())
}

/// Read length-prefixed Opus packets from `recv` and push each into
/// the peer's jitter buffer with a monotonic local sequence number.
async fn run_recv_pump(mut recv: RecvStream, peer_input: PeerInput) -> Result<()> {
    let mut seq: u64 = 0;
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    loop {
        let len = match recv.read_u32_le().await {
            Ok(n) => n as usize,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!("recv stream closed cleanly");
                return Ok(());
            }
            Err(e) => return Err(anyhow::anyhow!("read length: {e}")),
        };
        if len > MAX_FRAME_LEN {
            anyhow::bail!("audio frame too large: {len} > {MAX_FRAME_LEN}");
        }
        buf.resize(len, 0);
        recv.read_exact(&mut buf)
            .await
            .context("read opus payload")?;
        peer_input.push_frame(seq, Bytes::copy_from_slice(&buf));
        debug!(seq, len, "received audio frame");
        seq += 1;
    }
}
