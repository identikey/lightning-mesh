use anyhow::Result;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointId, SecretKey};
use iroh_gossip::net::Gossip;
use tokio::sync::Mutex;
use tracing::info;

use crate::audio_proto::{self, AudioHandler};
use crate::room::Room;
use crate::ticket::MeshTicket;

/// Top-level mesh node. Owns the iroh Router (which owns the endpoint),
/// Gossip, and the inbound audio-protocol handler.
///
/// `address_lookup` is an in-memory address book wired into the iroh endpoint.
/// We seed it from the ticket on join and from gossip Announce/PeerList
/// messages, so iroh can dial peers directly without falling back to
/// pkarr/DNS discovery (which fails for unpublished nodes — e.g. two peers
/// on the same LAN).
pub struct MeshNode {
    router: Router,
    gossip: Gossip,
    address_lookup: MemoryLookup,
    audio_handler: AudioHandler,
    room: Mutex<Option<Room>>,
}

impl MeshNode {
    /// Spawn a new mesh node with a fresh or persisted identity.
    pub async fn spawn() -> Result<Self> {
        // Use IROH_SECRET env var if set, otherwise generate a new key
        let secret_key = match std::env::var("IROH_SECRET") {
            Ok(s) => s.parse::<SecretKey>()?,
            Err(_) => SecretKey::generate(&mut rand::rng()),
        };

        let address_lookup = MemoryLookup::new();
        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .address_lookup(address_lookup.clone())
            .bind()
            .await?;

        let gossip = Gossip::builder().spawn(endpoint.clone());
        let audio_handler = AudioHandler::new();

        let router = Router::builder(endpoint)
            .accept(iroh_gossip::ALPN, gossip.clone())
            .accept(audio_proto::AUDIO_ALPN, audio_handler.clone())
            .spawn();

        info!(id = %router.endpoint().id(), "mesh node started");

        Ok(Self {
            router,
            gossip,
            address_lookup,
            audio_handler,
            room: Mutex::new(None),
        })
    }

    pub fn id(&self) -> EndpointId {
        self.router.endpoint().id()
    }

    fn endpoint(&self) -> &Endpoint {
        self.router.endpoint()
    }

    /// Enter a room. Without a ticket, creates a new room (first peer).
    /// With a ticket, joins an existing room using ticket addresses as bootstrap.
    pub async fn enter_room(&self, name: &str, ticket: Option<&str>) -> Result<String> {
        let topic_id = MeshTicket::topic_id_from_name(name);
        let gossip_topic_id = iroh_gossip::proto::TopicId::from_bytes(topic_id);

        let topic = if let Some(ticket_str) = ticket {
            let parsed_ticket: MeshTicket = ticket_str.parse()?;

            if parsed_ticket.name != name {
                anyhow::bail!(
                    "room name '{}' doesn't match ticket room '{}'",
                    name,
                    parsed_ticket.name
                );
            }

            // Seed iroh's address book with the ticket's full EndpointAddrs.
            // Without this, gossip would dial peers by EndpointId alone, which
            // forces iroh to fall back to pkarr/DNS discovery — that fails for
            // unpublished nodes (e.g. two peers on the same LAN).
            for addr in &parsed_ticket.addrs {
                self.address_lookup.add_endpoint_info(addr.clone());
            }

            let bootstrap_ids = parsed_ticket.bootstrap_peer_ids();
            info!(
                room = name,
                bootstrap_count = bootstrap_ids.len(),
                "joining room with {} bootstrap peer(s)",
                bootstrap_ids.len()
            );

            self.gossip
                .subscribe_and_join(gossip_topic_id, bootstrap_ids)
                .await
                .map_err(|e| anyhow::anyhow!("gossip subscribe_and_join failed: {e}"))?
        } else {
            info!(room = name, "creating new room");

            self.gossip
                .subscribe(gossip_topic_id, vec![])
                .await
                .map_err(|e| anyhow::anyhow!("gossip subscribe failed: {e}"))?
        };

        let room = Room::new(
            name.to_string(),
            topic,
            self.endpoint().clone(),
            self.address_lookup.clone(),
            self.audio_handler.clone(),
        );

        let our_ticket = room.generate_ticket();
        *self.room.lock().await = Some(room);

        Ok(our_ticket.to_string())
    }

    /// Run the room's actor loop. Returns when the room ends.
    pub async fn run_room(&self) -> Result<()> {
        let room = self.room.lock().await.take();
        match room {
            Some(room) => room.run().await,
            None => anyhow::bail!("no room to run"),
        }
    }

    pub async fn shutdown(self) {
        if let Err(e) = self.router.shutdown().await {
            tracing::warn!("router shutdown error: {e}");
        }
        info!("mesh node shut down");
    }
}
