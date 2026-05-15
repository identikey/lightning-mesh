use anyhow::Result;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{endpoint::Connection, Endpoint, EndpointId, SecretKey};
use iroh_gossip::net::Gossip;
use tokio::sync::Mutex;
use tracing::info;

use crate::room::Room;
use crate::ticket::MeshTicket;
use mjolnir_moq::{MoqBridge, MoqHandler};

/// Thin wrapper that adapts MoqHandler to iroh's ProtocolHandler trait.
#[derive(Clone, Debug)]
struct MoqProtocol(MoqHandler);

impl ProtocolHandler for MoqProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.0
            .accept(connection)
            .await
            .map_err(|e| AcceptError::from_boxed(e.into()))
    }
}

/// Top-level mesh node. Owns the iroh Router (which owns the endpoint), Gossip, and MoQ bridge.
///
/// `address_lookup` is an in-memory address book wired into the iroh endpoint.
/// We seed it from the ticket on join and from gossip Announce/PeerList messages,
/// so iroh can dial peers directly without falling back to pkarr/DNS discovery
/// (which fails for unpublished nodes — e.g. two peers on the same LAN).
pub struct MeshNode {
    router: Router,
    gossip: Gossip,
    bridge: MoqBridge,
    address_lookup: MemoryLookup,
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

        let bridge = MoqBridge::new();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let moq_handler = MoqProtocol(bridge.handler());

        let router = Router::builder(endpoint)
            .accept(iroh_gossip::ALPN, gossip.clone())
            .accept(mjolnir_moq::MOQ_ALPN, moq_handler)
            .spawn();

        info!(id = %router.endpoint().id(), "mesh node started");

        Ok(Self {
            router,
            gossip,
            bridge,
            address_lookup,
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
    ///
    /// Returns a join ticket that others can use. Every peer is equal after this —
    /// there is no "host" at the protocol level.
    ///
    /// **DHT opportunity:** With a DHT keyed by topic_id, `enter_room("name", None)`
    /// could first check the DHT for existing peers before assuming it's a new room.
    /// This would allow rooms to survive total peer departure and be revived by name
    /// alone, without any ticket exchange.
    pub async fn enter_room(&self, name: &str, ticket: Option<&str>) -> Result<String> {
        let topic_id = MeshTicket::topic_id_from_name(name);
        let gossip_topic_id = iroh_gossip::proto::TopicId::from_bytes(topic_id);

        let topic = if let Some(ticket_str) = ticket {
            // Join existing room — use ticket addresses as gossip bootstrap
            let parsed_ticket: MeshTicket = ticket_str.parse()?;

            // Validate room name matches ticket
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
            // New room — no bootstrap peers
            info!(room = name, "creating new room");

            self.gossip
                .subscribe(gossip_topic_id, vec![])
                .await
                .map_err(|e| anyhow::anyhow!("gossip subscribe failed: {e}"))?
        };

        // Share MeshNode's bridge so the Room publishes and subscribes via
        // the same Origin that incoming MoQ sessions register against —
        // otherwise the Router accepts sessions into one Origin while the
        // Room watches a different (empty) one.
        let room = Room::new(
            name.to_string(),
            topic,
            self.bridge.clone(),
            self.endpoint().clone(),
            self.address_lookup.clone(),
        );

        // Generate ticket before storing room (we need &room for generate_ticket)
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
