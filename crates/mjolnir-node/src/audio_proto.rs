//! Direct-iroh audio protocol over QUIC datagrams.
//!
//! Each peer pair holds a single QUIC connection. Each side sends one
//! datagram per Opus frame: a u64 little-endian sequence number followed
//! by the Opus bytes.
//!
//! Why datagrams (not bidi streams):
//!
//! QUIC streams are reliable and ordered, which means the receiver never
//! sees a missing frame — only a stalled stream during retransmits.
//! That made our [`SelfHealingBuffer`](mjolnir_media::SelfHealingBuffer)
//! see "gaps" only when stalls exceeded the warm-up depth, and it made
//! Opus in-band FEC strictly dead code (FEC needs a successfully-arrived
//! *next* frame while the prior one is still missing — impossible under
//! reliable in-order delivery).
//!
//! Datagrams are unreliable and unordered. Packets may be dropped or
//! arrive out of order. The receiver's sequence number lets the jitter
//! buffer detect real loss, reorder within the window, and hand the
//! next-in-sequence packet to `decode_lost` as an FEC lookahead.
//!
//! Wire format (one datagram):
//!
//! ```text
//! [u64 little-endian seq][opus bytes...]
//! ```

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use bytes::Bytes;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use mjolnir_audio::codec::OpusEncoder;
use mjolnir_audio::{AudioConfig, MixerHandle, PeerInput};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

/// ALPN identifier for the mjolnir audio protocol.
pub const AUDIO_ALPN: &[u8] = b"mjolnir/audio/v1";

/// Sequence prefix size on the wire.
const SEQ_BYTES: usize = 8;

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

        info!(%peer_id, "accepted inbound audio connection");
        run_session(
            peer_id.to_string(),
            connection,
            ctx.mixer_handle,
            ctx.pcm_tx,
            ctx.audio_config,
        )
        .await;
        Ok(())
    }
}

/// Dial a peer and run the audio session until the connection closes
/// or either pump ends.
pub async fn dial_and_run(
    endpoint: Endpoint,
    addr: EndpointAddr,
    mixer_handle: MixerHandle,
    pcm_tx: broadcast::Sender<Vec<i16>>,
    audio_config: AudioConfig,
) -> Result<()> {
    let peer_id = addr.id;
    info!(%peer_id, "dialing audio connection");
    let conn = endpoint
        .connect(addr, AUDIO_ALPN)
        .await
        .context("audio dial failed")?;
    info!(%peer_id, "audio connection open");

    run_session(
        peer_id.to_string(),
        conn,
        mixer_handle,
        pcm_tx,
        audio_config,
    )
    .await;
    Ok(())
}

/// Shared session driver used by both dial and accept paths. Registers
/// the peer with the mixer, spawns the send/recv datagram pumps, awaits
/// either pump ending or the connection closing, then deregisters.
async fn run_session(
    peer_key: String,
    connection: Connection,
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

    let send_conn = connection.clone();
    let send_key = peer_key.clone();
    let send_task = tokio::spawn(async move {
        if let Err(e) = run_send_pump(audio_config, pcm_rx, send_conn).await {
            warn!(peer = %send_key, "audio send pump ended: {e}");
        }
    });

    let recv_conn = connection.clone();
    let recv_key = peer_key.clone();
    let recv_task = tokio::spawn(async move {
        if let Err(e) = run_recv_pump(recv_conn, peer_input).await {
            warn!(peer = %recv_key, "audio recv pump ended: {e}");
        }
    });

    tokio::select! {
        _ = send_task => {
            debug!(peer = %peer_key, "send pump ended; closing connection");
            connection.close(0u32.into(), b"send ended");
        }
        _ = recv_task => {
            debug!(peer = %peer_key, "recv pump ended; closing connection");
            connection.close(0u32.into(), b"recv ended");
        }
        _ = connection.closed() => {
            info!(peer = %peer_key, "audio connection closed by peer");
        }
    }

    mixer_handle.remove_peer(&peer_key);
}

/// Drain decoded PCM from a broadcast subscription, encode with Opus,
/// and send each frame as a datagram with a u64 LE sequence prefix.
async fn run_send_pump(
    config: AudioConfig,
    mut pcm_rx: broadcast::Receiver<Vec<i16>>,
    connection: Connection,
) -> Result<()> {
    let mut encoder = OpusEncoder::new(&config)?;
    let mut seq: u64 = 0;
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

        let mut datagram = Vec::with_capacity(SEQ_BYTES + opus.len());
        datagram.extend_from_slice(&seq.to_le_bytes());
        datagram.extend_from_slice(&opus);

        match connection.send_datagram(Bytes::from(datagram)) {
            Ok(()) => debug!(seq, len = opus.len(), "sent audio datagram"),
            Err(e) => {
                // Datagram couldn't be queued locally (queue full, too
                // large, peer doesn't support, etc.). Don't kill the
                // session; the next frame will try again.
                warn!(seq, "send_datagram failed: {e}");
            }
        }
        seq = seq.wrapping_add(1);
    }
    Ok(())
}

/// Read datagrams, parse `[u64 seq][opus]`, push into the peer's jitter
/// buffer with the wire-supplied seq (so reorder and loss are visible).
async fn run_recv_pump(connection: Connection, peer_input: PeerInput) -> Result<()> {
    loop {
        let datagram = match connection.read_datagram().await {
            Ok(d) => d,
            Err(e) => {
                debug!("read_datagram ended: {e}");
                return Ok(());
            }
        };
        if datagram.len() < SEQ_BYTES {
            warn!("datagram too small: {} bytes", datagram.len());
            continue;
        }
        let seq_bytes: [u8; SEQ_BYTES] = datagram[..SEQ_BYTES]
            .try_into()
            .expect("len already checked");
        let seq = u64::from_le_bytes(seq_bytes);
        let opus = datagram.slice(SEQ_BYTES..);
        let opus_len = opus.len();
        peer_input.push_frame(seq, opus);
        debug!(seq, len = opus_len, "received audio datagram");
    }
}
