use std::collections::HashMap;

use anyhow::Result;
use bytes::Bytes;
use futures_lite::StreamExt;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use iroh_gossip::api::{Event, GossipTopic};
use mjolnir_audio::{AudioConfig, Mixer};
use mjolnir_moq::MoqBridge;
use moq_lite::{Broadcast, Track};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::ticket::MeshTicket;

/// Structured gossip messages for the mesh.
///
/// **DHT/DNS opportunity:** With a distributed name system, Announce could
/// be replaced by DHT put/get — peers publish their addr under the topic key,
/// and discovery happens via DHT lookup instead of gossip flooding. Gossip
/// would still be useful for real-time presence (NeighborUp/Down) but the
/// heavy lifting of address resolution could move to DHT, reducing gossip
/// bandwidth in large rooms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipMessage {
    /// "I'm here" — sent on join and when new neighbors appear.
    Announce(EndpointAddr),

    /// "Here are all the peers I know" — sent after connecting to a new peer.
    /// Enables transitive discovery: if A knows B and C, a new peer D that
    /// connects to A immediately learns about B and C without waiting for
    /// their individual announcements.
    ///
    /// **DHCP analogy:** This is similar to a DHCP server handing out the
    /// full network topology to a new client. In a future version, a
    /// distributed DHCP-like service could maintain the authoritative peer
    /// list, with gossip as the real-time update channel.
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
    bridge: MoqBridge,
    endpoint: Endpoint,
    /// Address book wired into the iroh endpoint. We seed it whenever we
    /// learn an `EndpointAddr` from gossip (Announce / PeerList) so that
    /// subsequent dials by `EndpointId` don't hit pkarr/DNS.
    address_lookup: MemoryLookup,
    /// Known peers: EndpointId → full EndpointAddr (for ticket minting).
    peers: HashMap<EndpointId, EndpointAddr>,
    audio_config: AudioConfig,
}

impl Room {
    pub fn new(
        name: String,
        topic: GossipTopic,
        bridge: MoqBridge,
        endpoint: Endpoint,
        address_lookup: MemoryLookup,
    ) -> Self {
        info!(room = %name, "room created");
        Self {
            name,
            topic,
            bridge,
            endpoint,
            address_lookup,
            peers: HashMap::new(),
            audio_config: AudioConfig::default(),
        }
    }

    /// Generate a join ticket for this room using this peer's address
    /// and all known peer addresses. Any peer can call this.
    ///
    /// **DHT opportunity:** With DHT, this could return a minimal ticket
    /// (just the room name) since addresses would be discoverable via DHT
    /// lookup on the topic_id.
    pub fn generate_ticket(&self) -> MeshTicket {
        let our_addr = self.endpoint.addr();
        let mut addrs = vec![our_addr];
        // Include known peer addresses for multi-peer bootstrap resilience
        addrs.extend(self.peers.values().cloned());
        MeshTicket::with_peers(self.name.clone(), addrs)
    }

    /// Run the room actor loop: publish audio, announce via gossip, handle peer events.
    pub async fn run(self) -> Result<()> {
        let Room {
            name,
            topic,
            bridge,
            endpoint,
            address_lookup,
            mut peers,
            audio_config,
        } = self;

        // Publish our audio broadcast using our endpoint ID as the broadcast name
        let our_id = endpoint.id();
        let our_broadcast_name = our_id.to_string();
        let origin = bridge.origin();
        let mut broadcast = Broadcast::produce();
        let broadcast_consumer = broadcast.consume();
        origin.publish_broadcast(&our_broadcast_name, broadcast_consumer);

        // Spawn audio capture -> encode -> publish task
        let ac = audio_config.clone();
        tokio::spawn(async move {
            if let Err(e) = mjolnir_audio::publish::run_publish(&ac, &mut broadcast).await {
                warn!("audio publish error: {e}");
            }
        });

        // Start the audio mixer (one cpal output stream, per-peer jitter buffers)
        let mixer = Mixer::start(audio_config.clone())?;

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
                            &bridge,
                            &mut peers,
                            &mixer,
                            &sender,
                            &address_lookup,
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
                        info!(%peer_id, "peer left, disconnecting");
                        mixer.remove_peer(&peer_id.to_string());
                        bridge.disconnect(&peer_id).await;
                    }
                }
                Event::Lagged => {
                    warn!("gossip receiver lagged, some events dropped");
                }
            }
        }

        info!(room = %name, "room gossip stream ended");
        drop(mixer);
        Ok(())
    }
}

/// Handle discovery of a peer address (from Announce or PeerList).
/// Connects MoQ session, subscribes to audio, and shares our peer list.
async fn handle_peer_discovered(
    addr: EndpointAddr,
    endpoint: &Endpoint,
    bridge: &MoqBridge,
    peers: &mut HashMap<EndpointId, EndpointAddr>,
    mixer: &Mixer,
    sender: &iroh_gossip::api::GossipSender,
    address_lookup: &MemoryLookup,
) {
    let peer_id = addr.id;

    if peer_id == endpoint.id() || peers.contains_key(&peer_id) {
        return;
    }

    info!(peer = %peer_id, "discovered new peer via gossip");

    // Seed iroh's address book so MoQ (and any future dialer keyed by
    // EndpointId) can reach the peer without pkarr/DNS discovery.
    address_lookup.add_endpoint_info(addr.clone());

    // Deterministic dial tiebreak: both peers see each other's gossip
    // Announce simultaneously, so if both call bridge.connect we end up
    // with two MoQ sessions per pair and the second clobbers the first.
    // Only the side with the lower endpoint id dials; the higher-id side
    // accepts the incoming connection on its iroh router. Either way the
    // session lands in the shared bridge with the same origin.
    let we_dial = endpoint.id() < peer_id;
    if we_dial {
        if let Err(e) = bridge.connect(endpoint, addr.clone()).await {
            warn!(peer = %peer_id, "failed to connect MoQ session: {e}");
            return;
        }
    } else {
        info!(peer = %peer_id, "waiting for inbound MoQ connection (dial tiebreak)");
    }
    peers.insert(peer_id, addr);

    // Share our full peer list with the mesh so the new peer (and others)
    // can discover all existing peers transitively.
    let peer_list: Vec<EndpointAddr> = peers.values().cloned().collect();
    if let Ok(msg) = GossipMessage::PeerList(peer_list).serialize() {
        if let Err(e) = sender.broadcast(msg).await {
            warn!("failed to broadcast peer list: {e}");
        }
    }

    // Register the peer with the mixer up front; we'll start pushing frames
    // once its broadcast is announced over the MoQ session.
    let peer_input = match mixer.add_peer(peer_id.to_string()) {
        Ok(pi) => pi,
        Err(e) => {
            warn!(peer = %peer_id, "failed to register peer with mixer: {e}");
            return;
        }
    };

    // Wait asynchronously for the peer's broadcast to be announced, then
    // subscribe. moq-lite can announce, retract, and re-announce the same
    // broadcast as the underlying session state churns (we've observed this
    // at handshake time even with a single session), so the loop survives
    // run_subscribe returning: each new announcement triggers a fresh
    // subscribe attempt with the freshly-published BroadcastConsumer.
    let mut origin_consumer = bridge.origin().consume();
    let target_path = peer_id.to_string();
    tokio::spawn(async move {
        loop {
            // Wait until the next time our target broadcast is announced
            // (with a real BroadcastConsumer, not an unannounce).
            let broadcast_consumer = loop {
                match origin_consumer.announced().await {
                    Some((path, Some(c))) if path.as_str() == target_path => break c,
                    Some(_) => continue,
                    None => return,
                }
            };

            let track = Track::new(mjolnir_audio::AUDIO_TRACK_NAME);
            match broadcast_consumer.subscribe_track(&track) {
                Ok(track_consumer) => {
                    if let Err(e) = mjolnir_audio::subscribe::run_subscribe(
                        track_consumer,
                        peer_input.clone(),
                    )
                    .await
                    {
                        warn!(peer = %peer_id, "audio subscribe error: {e}; awaiting re-announce");
                    }
                }
                Err(e) => {
                    warn!(peer = %peer_id, "failed to subscribe to audio track: {e}");
                }
            }
        }
    });
}
