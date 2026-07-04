//! Story S2 (bead `mjolnir-mesh-7jb`): prove a service record — a `/services`
//! map entry — propagates end-to-end across two nodes over the existing gossip
//! layer, the focused e21 slice the hello.mesh directory needs.
//!
//! "The existing gossip layer" here is the transport-agnostic [`GossipSync`]
//! seam that the daemon's real iroh-gossip transport plugs into (`crdt::sync`).
//! This test drives that exact seam — encode → broadcast → recv → decode →
//! merge → read — with an in-process paired transport standing in for
//! iroh-gossip, so what it validates is the record-type slice, not the radio.
//! Riding physical 802.11s across the fleet is the follow-up validation (bead
//! `2uq`), mirroring how the `/users` record was validated.
//!
//! Success criterion from the bead: a service published at node A appears in
//! node B's converged state within seconds; keyed by service name; LWW so a
//! newer publish wins and a stale one does not roll B back.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc;

use mjolnir_mesh::{
    merge_service, GossipError, GossipMessage, GossipSync, GossipTransport, MergeResult,
    ServiceBook, ServiceEntry, HLC,
};

/// One-directional in-process transport standing in for iroh-gossip.
///
/// `broadcast` pushes to the paired peer's inbox; `recv` pops this node's inbox.
/// Deliberately dumb (raw `Bytes`, no framing) — postcard ser/de lives in
/// `GossipSync`, exactly as the real `IrohGossipTransport` is dumb about it.
struct ChannelTransport {
    outbound: mpsc::Sender<Bytes>,
    inbound: tokio::sync::Mutex<mpsc::Receiver<Bytes>>,
}

#[async_trait::async_trait]
impl GossipTransport for ChannelTransport {
    async fn broadcast(&self, payload: Bytes) -> Result<(), GossipError> {
        self.outbound.send(payload).await.map_err(|_| GossipError::Closed)
    }

    async fn recv(&self) -> Result<Bytes, GossipError> {
        self.inbound.lock().await.recv().await.ok_or(GossipError::Closed)
    }
}

/// Wire A→B so A can publish into B's receive loop (B never publishes here).
fn a_to_b() -> (ChannelTransport, ChannelTransport) {
    let (a_out, b_in) = mpsc::channel::<Bytes>(64);
    let (b_out, a_in) = mpsc::channel::<Bytes>(64);
    let a = ChannelTransport {
        outbound: a_out,
        inbound: tokio::sync::Mutex::new(a_in),
    };
    let b = ChannelTransport {
        outbound: b_out,
        inbound: tokio::sync::Mutex::new(b_in),
    };
    (a, b)
}

fn service(hostname: &str, port: u16, wall_clock: u64, node_id: &str) -> ServiceEntry {
    ServiceEntry {
        hostname: hostname.to_string(),
        ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
        port,
        protocol: "_ipp._tcp".to_string(),
        txt: BTreeMap::new(),
        host_mac: [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
        updated_at: HLC {
            wall_clock,
            counter: 0,
            node_id: node_id.to_string(),
        },
    }
}

/// Poll `book` until `name` resolves to a record on `expected_port`, or fail
/// after a few seconds. Mirrors the bead's "within seconds over 802.11s" bar.
async fn await_port(book: &Arc<Mutex<ServiceBook>>, name: &str, expected_port: u16) {
    let deadline = Duration::from_secs(3);
    let poll = Duration::from_millis(10);
    let start = tokio::time::Instant::now();
    loop {
        if let Some(e) = book.lock().unwrap().get(name)
            && e.port == expected_port
        {
            return;
        }
        if start.elapsed() > deadline {
            let got = book.lock().unwrap().get(name).map(|e| e.port);
            panic!("node B did not converge on {name}=port:{expected_port} within {deadline:?}; last saw {got:?}");
        }
        tokio::time::sleep(poll).await;
    }
}

#[tokio::test]
async fn service_record_propagates_a_to_b_end_to_end() {
    let (a_tx, b_rx) = a_to_b();
    let node_a = GossipSync::new(a_tx);
    let node_b = GossipSync::new(b_rx);

    // Node B's CRDT store for the service record type, applied via the real merge.
    let book_b: Arc<Mutex<ServiceBook>> = Arc::new(Mutex::new(ServiceBook::new()));

    // Node B runs the real dispatch loop: recv → decode → merge → store.
    let dispatch = {
        let book = book_b.clone();
        tokio::spawn(async move {
            node_b
                .run(move |msg| {
                    if let GossipMessage::ServiceUpdate { name, entry } = msg {
                        let mut b = book.lock().unwrap();
                        match merge_service(b.get(&name), &entry) {
                            MergeResult::Inserted | MergeResult::Updated => {
                                b.insert(name, entry);
                            }
                            MergeResult::Unchanged | MergeResult::Conflict { .. } => {}
                        }
                    }
                })
                .await
        })
    };

    let name = "printer._ipp._tcp";

    // --- A publishes a service, gossips it; B must observe it within seconds. ---
    node_a
        .publish(GossipMessage::ServiceUpdate {
            name: name.to_string(),
            entry: service("printer", 631, 1_000, "node-a"),
        })
        .await
        .expect("publish from A");
    await_port(&book_b, name, 631).await;

    // --- A updates the same service (newer HLC); B must follow (LWW). ---
    node_a
        .publish(GossipMessage::ServiceUpdate {
            name: name.to_string(),
            entry: service("printer", 9100, 2_000, "node-a"),
        })
        .await
        .expect("update publish from A");
    await_port(&book_b, name, 9100).await;

    // A stale update (older HLC) must NOT roll B back.
    node_a
        .publish(GossipMessage::ServiceUpdate {
            name: name.to_string(),
            entry: service("printer", 1, 500, "node-a"),
        })
        .await
        .expect("stale publish from A");
    // Give the stale message time to be delivered and (correctly) discarded.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        book_b.lock().unwrap().get(name).unwrap().port,
        9100,
        "stale (older-HLC) update must not overwrite the newer value"
    );

    // Drop A so B's receive loop sees Closed and exits cleanly.
    drop(node_a);
    dispatch.await.expect("join dispatch").expect("dispatch loop ok");
}
