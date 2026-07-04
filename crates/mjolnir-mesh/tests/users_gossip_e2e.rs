//! Spike (bead `mjolnir-mesh-2xd`): prove a brand-new CRDT record type — a
//! `/users` map entry — propagates end-to-end across two nodes over the
//! existing gossip layer, before building the rest of the hello.mesh front desk.
//!
//! "The existing gossip layer" here is the transport-agnostic
//! [`GossipSync`] seam that the daemon's real iroh-gossip transport plugs into
//! (`crdt::sync`). This test drives that exact seam — encode → broadcast →
//! recv → decode → merge → read — with an in-process paired transport standing
//! in for iroh-gossip, so what it validates is the record-type slice, not the
//! radio. Riding physical 802.11s across the fleet is the follow-up validation
//! (as bead `0yb` got for the address book).
//!
//! Success criterion from the bead: a value set at node A is observable at node
//! B within seconds. The test writes a `UserEntry` at A, gossips it, and asserts
//! node B's `UserBook` converges — then mutates it at A and asserts B follows
//! (last-writer-wins), proving updates and not just first insert.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::mpsc;

use mjolnir_mesh::{
    merge_user, GossipError, GossipMessage, GossipSync, GossipTransport, MergeResult, UserBook,
    UserEntry, HLC,
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

fn user(username: &str, display: &str, wall_clock: u64, node_id: &str) -> UserEntry {
    UserEntry {
        username: username.to_string(),
        display_name: display.to_string(),
        registered_by: node_id.to_string(),
        attrs: BTreeMap::new(),
        updated_at: HLC {
            wall_clock,
            counter: 0,
            node_id: node_id.to_string(),
        },
    }
}

/// Poll `book` until `username` resolves to `expected_display`, or fail after a
/// few seconds. Mirrors the bead's "within seconds over 802.11s" bar.
async fn await_display(book: &Arc<Mutex<UserBook>>, username: &str, expected_display: &str) {
    let deadline = Duration::from_secs(3);
    let poll = Duration::from_millis(10);
    let start = tokio::time::Instant::now();
    loop {
        if let Some(e) = book.lock().unwrap().get(username)
            && e.display_name == expected_display
        {
            return;
        }
        if start.elapsed() > deadline {
            let got = book
                .lock()
                .unwrap()
                .get(username)
                .map(|e| e.display_name.clone());
            panic!("node B did not converge on {username}={expected_display:?} within {deadline:?}; last saw {got:?}");
        }
        tokio::time::sleep(poll).await;
    }
}

#[tokio::test]
async fn user_record_propagates_a_to_b_end_to_end() {
    let (a_tx, b_rx) = a_to_b();
    let node_a = GossipSync::new(a_tx);
    let node_b = GossipSync::new(b_rx);

    // Node B's CRDT store for the new record type, applied via the real merge.
    let book_b: Arc<Mutex<UserBook>> = Arc::new(Mutex::new(UserBook::new()));

    // Node B runs the real dispatch loop: recv → decode → merge → store.
    let dispatch = {
        let book = book_b.clone();
        tokio::spawn(async move {
            node_b
                .run(move |msg| {
                    if let GossipMessage::UserUpdate { username, entry } = msg {
                        let mut b = book.lock().unwrap();
                        match merge_user(b.get(&username), &entry) {
                            MergeResult::Inserted | MergeResult::Updated => {
                                b.insert(username, entry);
                            }
                            MergeResult::Unchanged | MergeResult::Conflict { .. } => {}
                        }
                    }
                })
                .await
        })
    };

    // --- A writes a user, gossips it; B must observe it within seconds. ---
    let entry = user("ada", "Ada", 1_000, "node-a");
    node_a
        .publish(GossipMessage::UserUpdate {
            username: "ada".to_string(),
            entry,
        })
        .await
        .expect("publish from A");
    await_display(&book_b, "ada", "Ada").await;

    // --- A updates the same user (newer HLC); B must follow (LWW). ---
    let updated = user("ada", "Ada Lovelace", 2_000, "node-a");
    node_a
        .publish(GossipMessage::UserUpdate {
            username: "ada".to_string(),
            entry: updated,
        })
        .await
        .expect("update publish from A");
    await_display(&book_b, "ada", "Ada Lovelace").await;

    // A stale update (older HLC) must NOT roll B back.
    let stale = user("ada", "STALE", 500, "node-a");
    node_a
        .publish(GossipMessage::UserUpdate {
            username: "ada".to_string(),
            entry: stale,
        })
        .await
        .expect("stale publish from A");
    // Give the stale message time to be delivered and (correctly) discarded.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        book_b.lock().unwrap().get("ada").unwrap().display_name,
        "Ada Lovelace",
        "stale (older-HLC) update must not overwrite the newer value"
    );

    // Drop A so B's receive loop sees Closed and exits cleanly.
    drop(node_a);
    dispatch.await.expect("join dispatch").expect("dispatch loop ok");
}
