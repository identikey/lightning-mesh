use std::collections::HashMap;

use anyhow::Result;
use bytes::Bytes;
use futures_lite::StreamExt;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use iroh_gossip::api::{Event, GossipTopic};
use mjolnir_audio::{AudioConfig, Mixer};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::audio_proto::AudioHandler;
use crate::ticket::MeshTicket;

/// Structured gossip messages for the mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipMessage {
    /// "I'm here" — sent on join and when new neighbors appear.
    Announce(EndpointAddr),

    /// "Here are all the peers I know" — sent after connecting to a new peer.
    /// Enables transitive discovery: if A knows B and C, a new peer D that
    /// connects to A immediately learns about B and C without waiting for
    /// their individual announcements.
    PeerList(Vec<EndpointAddr>),
}

impl GossipMessage {
    pub fn serialize(&self) -> Result<Bytes> {
        Ok(Bytes::from(postcard::to_allocvec(self)?))
    }

    pub fn deserialize(data: &[u8]) -> Result<Self> {
        Ok(postcard::from_bytes(data)?)
    }
}

/// A mesh room — manages gossip peer discovery and audio streams.
pub struct Room {
    pub name: String,
    topic: GossipTopic,
    endpoint: Endpoint,
    /// Address book wired into the iroh endpoint. We seed it whenever we
    /// learn an `EndpointAddr` from gossip (Announce / PeerList) so that
    /// subsequent dials by `EndpointId` don't hit pkarr/DNS.
    address_lookup: MemoryLookup,
    /// Inbound audio protocol handler; rebound to this room's mixer
    /// + capture broadcast when [`Room::run`] starts.
    audio_handler: AudioHandler,
    /// Known peers: EndpointId → full EndpointAddr (for ticket minting).
    peers: HashMap<EndpointId, EndpointAddr>,
    audio_config: AudioConfig,
}

impl Room {
    pub fn new(
        name: String,
        topic: GossipTopic,
        endpoint: Endpoint,
        address_lookup: MemoryLookup,
        audio_handler: AudioHandler,
    ) -> Self {
        info!(room = %name, "room created");
        Self {
            name,
            topic,
            endpoint,
            address_lookup,
            audio_handler,
            peers: HashMap::new(),
            audio_config: AudioConfig::default(),
        }
    }

    /// Generate a join ticket for this room using this peer's address
    /// and all known peer addresses. Any peer can call this.
    pub fn generate_ticket(&self) -> MeshTicket {
        let our_addr = self.endpoint.addr();
        let mut addrs = vec![our_addr];
        addrs.extend(self.peers.values().cloned());
        MeshTicket::with_peers(self.name.clone(), addrs)
    }

    /// Run the room actor loop: capture audio, announce via gossip, handle peer events.
    pub async fn run(self) -> Result<()> {
        let Room {
            name,
            topic,
            endpoint,
            address_lookup,
            audio_handler,
            mut peers,
            audio_config,
        } = self;

        // Start the audio mixer (one cpal output stream, per-peer jitter buffers)
        let mixer = Mixer::start(audio_config.clone())?;

        // Single cpal capture; fan-out via a tokio broadcast channel so every
        // per-peer audio session (dialed or accepted) gets its own subscriber.
        let (capture, mut capture_rx) =
            mjolnir_audio::capture::AudioCapture::start(&audio_config)?;
        let (pcm_tx, _) = broadcast::channel::<Vec<i16>>(64);
        let pcm_tx_for_drain = pcm_tx.clone();
        tokio::spawn(async move {
            while let Some(frame) = capture_rx.recv().await {
                let _ = pcm_tx_for_drain.send(frame);
            }
        });
        // Keep the cpal stream alive for the room's lifetime.
        let _capture_keepalive = capture;

        // Bind the inbound audio protocol handler to this room's mixer
        // and capture broadcast. Inbound audio sessions will register
        // peers with the mixer and pump frames through the same broadcast.
        audio_handler.bind(mixer.handle(), pcm_tx.clone(), audio_config.clone());

        // Periodic stats heartbeat: every 5 seconds, log per-peer
        // decode/conceal counts so PLC engagement is visible during
        // demos and integration runs. Aborted when the room ends.
        let stats_mixer = mixer.handle();
        let stats_task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await; // first tick fires immediately; consume it
            loop {
                tick.tick().await;
                let snapshot = stats_mixer.all_peer_stats();
                if snapshot.is_empty() {
                    continue;
                }
                for (peer, s) in snapshot {
                    info!(
                        peer = %peer,
                        decoded = s.decoded,
                        concealed = s.concealed,
                        fec = s.fec_recovered,
                        errors = s.errors,
                        "peer audio stats"
                    );
                }
            }
        });

        // Announce ourselves via gossip
        let our_addr = endpoint.addr();
        let announce = GossipMessage::Announce(our_addr.clone());
        let announce_bytes = announce.serialize()?;

        let (sender, mut receiver) = topic.split();
        sender
            .broadcast(announce_bytes.clone())
            .await
            .map_err(|e| anyhow::anyhow!("gossip broadcast failed: {e}"))?;

        // Process gossip events
        while let Some(event) = receiver.next().await {
            let event = match event {
                Ok(e) => e,
                Err(e) => {
                    warn!("gossip event error: {e}");
                    continue;
                }
            };

            match event {
                Event::Received(msg) => {
                    let gossip_msg = match GossipMessage::deserialize(&msg.content) {
                        Ok(m) => m,
                        Err(e) => {
                            warn!("failed to parse gossip message: {e}");
                            continue;
                        }
                    };

                    let addrs_to_process = match gossip_msg {
                        GossipMessage::Announce(addr) => vec![addr],
                        GossipMessage::PeerList(addrs) => addrs,
                    };

                    for addr in addrs_to_process {
                        handle_peer_discovered(
                            addr,
                            &endpoint,
                            &mut peers,
                            &mixer,
                            &sender,
                            &address_lookup,
                            &pcm_tx,
                            &audio_config,
                        )
                        .await;
                    }
                }
                Event::NeighborUp(_peer_id) => {
                    // Re-announce our address to new neighbors
                    if let Err(e) = sender.broadcast(announce_bytes.clone()).await {
                        warn!("gossip re-announce failed: {e}");
                    }
                }
                Event::NeighborDown(peer_id) => {
                    if peers.remove(&peer_id).is_some() {
                        info!(%peer_id, "peer left");
                        mixer.remove_peer(&peer_id.to_string());
                    }
                }
                Event::Lagged => {
                    warn!("gossip receiver lagged, some events dropped");
                }
            }
        }

        info!(room = %name, "room gossip stream ended");
        stats_task.abort();
        drop(mixer);
        Ok(())
    }
}

/// Handle discovery of a peer address (from Announce or PeerList).
///
/// Seeds iroh's address book and — if our endpoint id is lower than
/// the peer's — opens an outbound audio session. The higher-id side
/// just waits; its [`AudioHandler`] will accept the inbound stream.
#[allow(clippy::too_many_arguments)]
async fn handle_peer_discovered(
    addr: EndpointAddr,
    endpoint: &Endpoint,
    peers: &mut HashMap<EndpointId, EndpointAddr>,
    mixer: &Mixer,
    sender: &iroh_gossip::api::GossipSender,
    address_lookup: &MemoryLookup,
    pcm_tx: &broadcast::Sender<Vec<i16>>,
    audio_config: &AudioConfig,
) {
    let peer_id = addr.id;

    if peer_id == endpoint.id() || peers.contains_key(&peer_id) {
        return;
    }

    info!(peer = %peer_id, "discovered new peer via gossip");

    // Seed iroh's address book so the audio dial doesn't fall back to DNS.
    address_lookup.add_endpoint_info(addr.clone());
    peers.insert(peer_id, addr.clone());

    // Share our full peer list with the mesh so the new peer (and others)
    // can discover all existing peers transitively.
    let peer_list: Vec<EndpointAddr> = peers.values().cloned().collect();
    if let Ok(msg) = GossipMessage::PeerList(peer_list).serialize() {
        if let Err(e) = sender.broadcast(msg).await {
            warn!("failed to broadcast peer list: {e}");
        }
    }

    // Deterministic dial tiebreak. With a single bidi stream per pair we
    // only need one initiator; the other side accepts via AudioHandler.
    let we_dial = endpoint.id() < peer_id;
    if !we_dial {
        info!(peer = %peer_id, "waiting for inbound audio stream (dial tiebreak)");
        return;
    }

    let endpoint = endpoint.clone();
    let mixer_handle = mixer.handle();
    let pcm_tx = pcm_tx.clone();
    let cfg = audio_config.clone();
    tokio::spawn(async move {
        if let Err(e) =
            crate::audio_proto::dial_and_run(endpoint, addr, mixer_handle, pcm_tx, cfg).await
        {
            warn!(%peer_id, "audio dial failed: {e}");
        }
    });
}
