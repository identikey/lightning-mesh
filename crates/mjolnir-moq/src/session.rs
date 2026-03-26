use anyhow::Result;
use iroh::endpoint::Connection;
use iroh::EndpointId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info};

use crate::MOQ_ALPN;

/// A moq-lite session over an iroh connection.
pub struct MoqSession {
    connection: Connection,
    // TODO: wrap with web-transport-iroh Session + moq-lite Publisher/Subscriber
}

impl MoqSession {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    pub fn remote_id(&self) -> EndpointId {
        self.connection.remote_id()
    }
}

/// Manages MoQ sessions over iroh connections.
///
/// Handles connection pooling, deduplication, and protocol handler registration.
pub struct MoqBridge {
    sessions: Arc<Mutex<HashMap<EndpointId, MoqSession>>>,
    cmd_tx: mpsc::Sender<BridgeCmd>,
}

enum BridgeCmd {
    Connect {
        endpoint_id: EndpointId,
        reply: mpsc::Sender<Result<()>>,
    },
    Disconnect {
        endpoint_id: EndpointId,
    },
}

impl MoqBridge {
    /// Create a new MoQ bridge. Call `run()` to start the actor loop.
    pub fn new() -> (Self, BridgeActor) {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let sessions = Arc::new(Mutex::new(HashMap::new()));

        let bridge = Self {
            sessions: sessions.clone(),
            cmd_tx,
        };

        let actor = BridgeActor {
            sessions,
            cmd_rx,
        };

        (bridge, actor)
    }

    /// Connect to a peer and establish a MoQ session.
    pub async fn connect(
        &self,
        endpoint: &iroh::Endpoint,
        peer_id: EndpointId,
    ) -> Result<()> {
        // Check if already connected
        if self.sessions.lock().await.contains_key(&peer_id) {
            debug!(%peer_id, "already connected, reusing session");
            return Ok(());
        }

        info!(%peer_id, "connecting to peer");
        let conn = endpoint.connect(peer_id, MOQ_ALPN).await?;
        let session = MoqSession::new(conn);

        self.sessions.lock().await.insert(peer_id, session);
        info!(%peer_id, "MoQ session established");

        Ok(())
    }

    /// Disconnect from a peer.
    pub async fn disconnect(&self, peer_id: &EndpointId) {
        if self.sessions.lock().await.remove(peer_id).is_some() {
            info!(%peer_id, "disconnected");
        }
    }

    /// Number of active sessions.
    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }
}

/// Actor that processes bridge commands and handles incoming connections.
pub struct BridgeActor {
    sessions: Arc<Mutex<HashMap<EndpointId, MoqSession>>>,
    cmd_rx: mpsc::Receiver<BridgeCmd>,
}

impl BridgeActor {
    /// Run the actor loop. Accepts incoming connections on the endpoint.
    pub async fn run(
        mut self,
        endpoint: iroh::Endpoint,
    ) -> Result<()> {
        loop {
            tokio::select! {
                Some(cmd) = self.cmd_rx.recv() => {
                    match cmd {
                        BridgeCmd::Connect { endpoint_id, reply } => {
                            let result = async {
                                let conn = endpoint.connect(endpoint_id, MOQ_ALPN).await?;
                                let session = MoqSession::new(conn);
                                self.sessions.lock().await.insert(endpoint_id, session);
                                Ok(())
                            }.await;
                            let _ = reply.send(result);
                        }
                        BridgeCmd::Disconnect { endpoint_id } => {
                            self.sessions.lock().await.remove(&endpoint_id);
                        }
                    }
                }
                else => break,
            }
        }

        Ok(())
    }
}
