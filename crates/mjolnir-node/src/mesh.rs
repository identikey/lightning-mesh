use anyhow::Result;
use iroh::{Endpoint, EndpointId, SecretKey};
use iroh::endpoint::presets;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::room::Room;
use crate::ticket::MeshTicket;
use mjolnir_moq::MoqBridge;

/// Top-level mesh node. Owns the iroh endpoint and active room.
pub struct MeshNode {
    endpoint: Endpoint,
    bridge: MoqBridge,
    room: Arc<Mutex<Option<Room>>>,
}

impl MeshNode {
    /// Spawn a new mesh node with a fresh or persisted identity.
    pub async fn spawn() -> Result<Self> {
        // Use IROH_SECRET env var if set, otherwise generate a new key
        let secret_key = match std::env::var("IROH_SECRET") {
            Ok(s) => s.parse::<SecretKey>()?,
            Err(_) => SecretKey::generate(&mut rand::rng()),
        };

        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(secret_key)
            .bind()
            .await?;

        info!(id = %endpoint.id(), "mesh node started");

        let (bridge, _actor) = MoqBridge::new();

        Ok(Self {
            endpoint,
            bridge,
            room: Arc::new(Mutex::new(None)),
        })
    }

    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Host a new room and return a join ticket.
    pub async fn host_room(&self, name: &str) -> Result<String> {
        let addr = self.endpoint.addr();
        let ticket = MeshTicket {
            name: name.to_string(),
            addr,
        };

        let room = Room::new(name.to_string(), self.endpoint.clone());
        *self.room.lock().await = Some(room);

        info!(name, "hosting room");
        Ok(ticket.to_string())
    }

    /// Join a room by ticket string.
    pub async fn join_room(&self, ticket_str: &str) -> Result<()> {
        let ticket: MeshTicket = ticket_str.parse()?;

        info!(name = ticket.name, peer = %ticket.addr.id, "joining room");

        // Connect to the host
        self.bridge.connect(&self.endpoint, ticket.addr.id).await?;

        let room = Room::new(ticket.name.clone(), self.endpoint.clone());
        *self.room.lock().await = Some(room);

        Ok(())
    }

    pub async fn shutdown(self) {
        self.endpoint.close().await;
        info!("mesh node shut down");
    }
}
