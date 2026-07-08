//! Transport-agnostic CRDT gossip sync seam.
//!
//! This module defines the *seam* between the iroh-free substrate library and
//! the concrete gossip transport (iroh-gossip), which lives in the daemon
//! binary. It mirrors the [`DatagramConn`](crate::tun::encap::DatagramConn)
//! pattern: the lib declares an async trait ([`GossipTransport`]) plus a
//! dispatch loop ([`GossipSync`]), and the daemon provides the concrete impl.
//!
//! **Invariant:** this file is iroh-free. The transport deals in raw
//! [`bytes::Bytes`] (matching iroh-gossip's broadcast/recv shape); postcard
//! ser/de of [`GossipMessage`] is owned here, not by the transport.
//!
//! Gossip is best-effort. A decode failure on a single inbound payload is
//! logged and skipped — it does NOT terminate the receive loop. The loop only
//! exits cleanly when the transport reports [`GossipError::Closed`].

use bytes::Bytes;

use crate::crdt::gossip::GossipMessage;

/// Errors produced by the gossip sync seam.
#[derive(Debug, thiserror::Error)]
pub enum GossipError {
    /// Postcard serialization of a [`GossipMessage`] failed.
    #[error("encode: {0}")]
    Encode(postcard::Error),
    /// Postcard deserialization of an inbound payload failed.
    #[error("decode: {0}")]
    Decode(postcard::Error),
    /// The underlying transport reported an error.
    #[error("transport: {0}")]
    Transport(String),
    /// The transport has been closed; the receive loop should exit cleanly.
    #[error("transport closed")]
    Closed,
}

/// Transport abstraction over a CRDT gossip overlay.
///
/// Mirrors [`DatagramConn`](crate::tun::encap::DatagramConn): the lib defines
/// this trait; the daemon provides a concrete iroh-gossip impl. It deals in
/// raw bytes so it stays dumb — postcard framing lives in [`GossipSync`].
#[async_trait::async_trait]
pub trait GossipTransport: Send + Sync {
    /// Broadcast a raw payload to all peers on the gossip overlay.
    async fn broadcast(&self, payload: Bytes) -> Result<(), GossipError>;

    /// Receive the next raw payload from a peer.
    ///
    /// Returns [`GossipError::Closed`] when the transport is shut down.
    async fn recv(&self) -> Result<Bytes, GossipError>;
}

/// Encode a [`GossipMessage`] to postcard bytes for broadcast.
pub fn encode(msg: &GossipMessage) -> Result<Bytes, GossipError> {
    postcard::to_allocvec(msg)
        .map(Bytes::from)
        .map_err(GossipError::Encode)
}

/// Decode a postcard payload into a [`GossipMessage`].
pub fn decode(payload: &[u8]) -> Result<GossipMessage, GossipError> {
    postcard::from_bytes(payload).map_err(GossipError::Decode)
}

/// Dispatcher that bridges a [`GossipTransport`] and the postcard wire format.
///
/// Owns no CRDT state: applying decoded messages to per-type CRDT state is the
/// caller's concern. The handler passed to [`GossipSync::run`] is the seam for
/// that (the daemon / a later bead wires merge logic there).
pub struct GossipSync<T: GossipTransport> {
    transport: T,
}

impl<T: GossipTransport> GossipSync<T> {
    /// Wrap a transport in the dispatcher.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Consume the dispatcher and return the underlying transport.
    pub fn into_inner(self) -> T {
        self.transport
    }

    /// Encode `msg` and broadcast it to all peers.
    pub async fn publish(&self, msg: GossipMessage) -> Result<(), GossipError> {
        let payload = encode(&msg)?;
        self.transport.broadcast(payload).await
    }

    /// Run the receive loop: `recv()` → `decode()` → `on_message`.
    ///
    /// Loops indefinitely, delivering each successfully decoded
    /// [`GossipMessage`] to the caller-supplied handler. Because gossip is
    /// best-effort, a decode error on any single payload is logged and skipped
    /// — the loop continues. The loop returns `Ok(())` when the transport
    /// reports [`GossipError::Closed`], and returns any other transport error.
    pub async fn run<F>(&self, mut on_message: F) -> Result<(), GossipError>
    where
        F: FnMut(GossipMessage) + Send,
    {
        loop {
            let payload = match self.transport.recv().await {
                Ok(p) => p,
                Err(GossipError::Closed) => return Ok(()),
                Err(e) => return Err(e),
            };
            match decode(&payload) {
                Ok(msg) => on_message(msg),
                Err(e) => {
                    // Best-effort gossip: skip the malformed message, keep looping.
                    tracing::warn!(error = %e, "skipping malformed gossip payload");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    use crate::crdt::hlc::HLC;

    /// mpsc-backed test double, paired like `MockConn` in `tun::encap`.
    #[derive(Clone)]
    struct MockTransport {
        tx: Arc<mpsc::Sender<Bytes>>,
        rx: Arc<tokio::sync::Mutex<mpsc::Receiver<Bytes>>>,
    }

    impl MockTransport {
        fn pair() -> (MockTransport, MockTransport) {
            let (a_tx, b_rx) = mpsc::channel::<Bytes>(256);
            let (b_tx, a_rx) = mpsc::channel::<Bytes>(256);
            let a = MockTransport {
                tx: Arc::new(a_tx),
                rx: Arc::new(tokio::sync::Mutex::new(a_rx)),
            };
            let b = MockTransport {
                tx: Arc::new(b_tx),
                rx: Arc::new(tokio::sync::Mutex::new(b_rx)),
            };
            (a, b)
        }

        /// Inject a raw payload as if a peer had sent it (for malformed-input tests).
        async fn inject_raw(&self, payload: Bytes) {
            self.tx.send(payload).await.unwrap();
        }
    }

    #[async_trait::async_trait]
    impl GossipTransport for MockTransport {
        async fn broadcast(&self, payload: Bytes) -> Result<(), GossipError> {
            self.tx.send(payload).await.map_err(|_| GossipError::Closed)
        }

        async fn recv(&self) -> Result<Bytes, GossipError> {
            self.rx.lock().await.recv().await.ok_or(GossipError::Closed)
        }
    }

    fn sample_message() -> GossipMessage {
        GossipMessage::SubnetClaimRelease {
            cidr: "10.42.1.0_24".to_string(),
            hlc: HLC {
                wall_clock: 1_700_000_003_000,
                counter: 0,
                node_id: "router-c".to_string(),
            },
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let msg = sample_message();
        let bytes = encode(&msg).unwrap();
        let decoded = decode(&bytes).unwrap();
        // Compare via re-serialization (GossipMessage isn't Eq).
        assert_eq!(
            postcard::to_allocvec(&msg).unwrap(),
            postcard::to_allocvec(&decoded).unwrap()
        );
    }

    #[tokio::test]
    async fn publish_is_received_and_decoded_by_dispatcher() {
        let (a, b) = MockTransport::pair();
        let publisher = GossipSync::new(a);
        let receiver = GossipSync::new(b);

        let msg = sample_message();
        let expected = postcard::to_allocvec(&msg).unwrap();
        publisher.publish(msg).await.unwrap();

        // Drop the publisher's transport so the receive loop sees Closed and exits.
        drop(publisher);

        let (seen_tx, mut seen_rx) = mpsc::unbounded_channel();
        receiver
            .run(move |m| {
                seen_tx.send(postcard::to_allocvec(&m).unwrap()).unwrap();
            })
            .await
            .unwrap();

        let got = seen_rx
            .recv()
            .await
            .expect("handler should have seen the message");
        assert_eq!(got, expected);
        assert!(
            seen_rx.recv().await.is_none(),
            "exactly one message expected"
        );
    }

    #[tokio::test]
    async fn malformed_payload_is_skipped_not_fatal() {
        let (a, b) = MockTransport::pair();
        let receiver = GossipSync::new(b);

        // First a garbage payload, then a valid one.
        a.inject_raw(Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]))
            .await;
        let good = sample_message();
        let expected = postcard::to_allocvec(&good).unwrap();
        a.broadcast(encode(&good).unwrap()).await.unwrap();

        // Drop the sender so the loop terminates after draining.
        drop(a);

        let (seen_tx, mut seen_rx) = mpsc::unbounded_channel();
        receiver
            .run(move |m| {
                seen_tx.send(postcard::to_allocvec(&m).unwrap()).unwrap();
            })
            .await
            .unwrap();

        // Only the valid message reaches the handler; the garbage was skipped.
        let got = seen_rx.recv().await.expect("valid message should arrive");
        assert_eq!(got, expected);
        assert!(
            seen_rx.recv().await.is_none(),
            "malformed payload must not be delivered"
        );
    }
}
