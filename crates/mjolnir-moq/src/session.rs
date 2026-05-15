use anyhow::Result;
use moq_lite::OriginProducer;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

// Re-export iroh types from web-transport-iroh to ensure version consistency.
// web-transport-iroh 0.2.2 uses iroh 0.96, and Session::raw() requires that version's Connection.
pub use web_transport_iroh::iroh;

use iroh::endpoint::Connection;
use iroh::{EndpointAddr, EndpointId};

use crate::MOQ_ALPN;

/// Type alias for the shared session map used by both MoqBridge and MoqHandler.
pub(crate) type SharedSessionMap = Arc<Mutex<HashMap<EndpointId, MoqSession>>>;

/// A moq-lite session over an iroh connection, wrapped via WebTransport.
pub struct MoqSession {
    wt_session: web_transport_iroh::Session,
    moq_session: moq_lite::Session,
}

impl MoqSession {
    pub fn new(wt_session: web_transport_iroh::Session, moq_session: moq_lite::Session) -> Self {
        Self {
            wt_session,
            moq_session,
        }
    }

    /// The underlying WebTransport session.
    pub fn wt_session(&self) -> &web_transport_iroh::Session {
        &self.wt_session
    }

    /// The MoQ protocol session.
    pub fn moq_session(&self) -> &moq_lite::Session {
        &self.moq_session
    }

    /// Close the MoQ session.
    pub fn close(&mut self) {
        self.moq_session.close(moq_lite::Error::Cancel);
    }
}

/// Manages MoQ sessions over iroh connections.
///
/// Holds a shared `OriginProducer` that callers use to publish broadcasts
/// and consume remote ones. Use `handler()` to get a `MoqHandler` that
/// shares the same session map and origin for accepting incoming connections.
///
/// **Note on iroh versions:** This crate uses iroh types re-exported from
/// `web_transport_iroh::iroh` (currently iroh 0.96) to ensure type compatibility
/// with `web_transport_iroh::Session::raw()`. If your application uses iroh 0.97,
/// use the re-exported types from `mjolnir_moq::iroh` for MoQ-related operations.
#[derive(Clone)]
pub struct MoqBridge {
    sessions: SharedSessionMap,
    origin: OriginProducer,
}

impl MoqBridge {
    /// Create a new MoQ bridge with a fresh Origin.
    pub fn new() -> Self {
        let origin = moq_lite::Origin::produce();
        let sessions = Arc::new(Mutex::new(HashMap::new()));
        Self { sessions, origin }
    }

    /// Connect to a peer and establish a MoQ session (outgoing).
    ///
    /// Uses the iroh 0.96 Endpoint re-exported from `web_transport_iroh::iroh`.
    pub async fn connect(
        &self,
        endpoint: &iroh::Endpoint,
        addr: impl Into<EndpointAddr>,
    ) -> Result<()> {
        let addr: EndpointAddr = addr.into();
        let peer_id = addr.id;

        // Check if already connected
        if self.sessions.lock().await.contains_key(&peer_id) {
            debug!(%peer_id, "already connected, reusing session");
            return Ok(());
        }

        info!(%peer_id, "connecting to peer");
        let conn = endpoint.connect(addr, MOQ_ALPN).await?;
        let wt_session = web_transport_iroh::Session::raw(conn);
        let moq_session = moq_lite::Client::new()
            .with_origin(self.origin.clone())
            .connect(wt_session.clone())
            .await?;

        let session = MoqSession::new(wt_session, moq_session);
        self.sessions.lock().await.insert(peer_id, session);
        info!(%peer_id, "MoQ session established");

        Ok(())
    }

    /// Accept an incoming connection and establish a MoQ session (incoming).
    ///
    /// This is the server-side counterpart to `connect()`. Pass a `Connection`
    /// from an iroh accept loop.
    pub async fn accept_connection(&self, connection: Connection) -> Result<()> {
        let peer_id = connection.remote_id();
        info!(%peer_id, "accepting incoming MoQ connection");

        let wt_session = web_transport_iroh::Session::raw(connection);
        let moq_session = moq_lite::Server::new()
            .with_origin(self.origin.clone())
            .accept(wt_session.clone())
            .await?;

        let session = MoqSession::new(wt_session, moq_session);
        self.sessions.lock().await.insert(peer_id, session);
        info!(%peer_id, "incoming MoQ session established");

        Ok(())
    }

    /// Disconnect from a peer.
    pub async fn disconnect(&self, peer_id: &EndpointId) {
        if let Some(mut session) = self.sessions.lock().await.remove(peer_id) {
            session.close();
            info!(%peer_id, "disconnected");
        }
    }

    /// Return a clone of the shared OriginProducer.
    ///
    /// Callers use this to publish broadcasts and consume remote ones.
    pub fn origin(&self) -> OriginProducer {
        self.origin.clone()
    }

    /// Number of active sessions.
    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Create a `MoqHandler` that shares this bridge's session map and origin,
    /// suitable for use as a protocol handler for incoming connections.
    pub fn handler(&self) -> crate::MoqHandler {
        crate::MoqHandler {
            origin: self.origin.clone(),
            sessions: self.sessions.clone(),
        }
    }
}
