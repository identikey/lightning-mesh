//! Story S4.2 (bead `mjolnir-mesh-e21.7`): multi-node service-mesh simulation
//! harness. Automates two PRD metrics against the v2 owner-bound service CRDT
//! lane (e21.2.1-e21.2.4): "Conflict resolution correctness" (zero split-brain
//! across every node, regardless of gossip delivery order) and "Convergence
//! bound" (one full anti-entropy flush is enough).
//!
//! This is a *synchronous* simulation rather than the async
//! [`GossipTransport`]/[`GossipSync`] pattern used by
//! `tests/services_gossip_e2e.rs`: partition-heal orderings need to be
//! adversarially controlled message-by-message (A-first, B-first,
//! interleaved, duplicated), which is awkward to pin down against tokio task
//! scheduling. Instead each simulated node's state is driven directly through
//! the same lib seams the real daemon dispatch arm calls
//! (`apply_service_publish_v2_tracking_loss`, `apply_service_unpublish_v2`,
//! from `crdt::service_apply`, bead e21.2.2/e21.2.3/e21.2.4), and the "wire"
//! is real: every message is postcard-encoded/decoded via the same
//! `crdt::sync::encode`/`decode` the real `GossipSync` uses, it is just
//! queued per (from, to) link instead of pushed through an mpsc channel, so a
//! test can hold a link (the partition lever) and flush links in any chosen
//! order.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};

use bytes::Bytes;

use mjolnir_mesh::crdt::sync::{decode, encode};
use mjolnir_mesh::{
    GossipMessage, HLC, LostNameMap, ServiceBookV2, ServiceEntryV2, ServiceTombstoneBook,
    apply_service_publish_v2_tracking_loss, apply_service_unpublish_v2,
};

// --- simulated node ---

/// One simulated node's full v2 service-mesh state — the same triple of
/// stores the daemon holds behind `Arc<Mutex<_>>` (bead e21.2.3).
struct SimNode {
    id: String,
    book: ServiceBookV2,
    tombstones: ServiceTombstoneBook,
    lost_names: LostNameMap,
}

impl SimNode {
    fn new(id: &str) -> Self {
        SimNode {
            id: id.to_string(),
            book: ServiceBookV2::new(),
            tombstones: ServiceTombstoneBook::new(),
            lost_names: LostNameMap::new(),
        }
    }

    /// Apply a decoded gossip message exactly as the real dispatch arm does
    /// (e21.2.3's daemon match arms over `GossipMessage::ServicePublishV2` /
    /// `ServiceUnpublishV2`).
    fn apply(&mut self, msg: &GossipMessage) {
        match msg {
            GossipMessage::ServicePublishV2 { name, entry } => {
                let _ = apply_service_publish_v2_tracking_loss(
                    &mut self.book,
                    &self.tombstones,
                    &mut self.lost_names,
                    &self.id,
                    name,
                    entry.clone(),
                );
            }
            GossipMessage::ServiceUnpublishV2 {
                name,
                owner_node_id,
                hlc,
            } => {
                let _ = apply_service_unpublish_v2(
                    &mut self.book,
                    &mut self.tombstones,
                    name,
                    owner_node_id,
                    hlc.clone(),
                );
            }
            _ => {}
        }
    }
}

// --- simulated in-memory gossip fabric ---

/// N-node in-memory gossip fabric. Generalizes the `ChannelTransport`
/// pattern from `tests/services_gossip_e2e.rs` (and `MockTransport` in
/// `crdt::sync`'s own unit tests) from a fixed A-to-B pair to N nodes with an
/// explicit, test-controllable per-link queue standing in for delivery: a
/// `broadcast` enqueues postcard bytes on every (from, to) link without
/// touching the peer's state, and nothing is observed by a node until its
/// inbound link is explicitly flushed. Withholding a flush IS the partition
/// lever; healing a partition is just flushing the links that were held,
/// in whatever order the test wants to stress.
struct Network {
    nodes: Vec<SimNode>,
    links: BTreeMap<(usize, usize), Vec<Bytes>>,
}

impl Network {
    fn new(ids: &[&str]) -> Self {
        let n = ids.len();
        let mut links = BTreeMap::new();
        for a in 0..n {
            for b in 0..n {
                if a != b {
                    links.insert((a, b), Vec::new());
                }
            }
        }
        Network {
            nodes: ids.iter().map(|id| SimNode::new(id)).collect(),
            links,
        }
    }

    /// Node `from` locally applies `msg` (mirrors a daemon's own publish
    /// mutating its local store synchronously, FR25) and enqueues it for
    /// every other node's inbound link. Peers do not see it until their link
    /// is flushed.
    fn publish_from(&mut self, from: usize, msg: GossipMessage) {
        self.nodes[from].apply(&msg);
        let payload = encode(&msg).expect("encode gossip message");
        for to in 0..self.nodes.len() {
            if to != from {
                self.links
                    .get_mut(&(from, to))
                    .unwrap()
                    .push(payload.clone());
            }
        }
    }

    /// Payloads currently queued on link (from, to), without draining them.
    fn peek_link(&self, from: usize, to: usize) -> Vec<Bytes> {
        self.links[&(from, to)].clone()
    }

    /// Decode and apply `payloads`, in order, to node `to` — without
    /// touching any queue. The seam that lets a test replay/reorder/duplicate
    /// deliveries freely.
    fn deliver(&mut self, to: usize, payloads: &[Bytes]) {
        for payload in payloads {
            let msg = decode(payload).expect("decode gossip message");
            self.nodes[to].apply(&msg);
        }
    }

    /// Flush every message currently queued on link (from, to) to node `to`,
    /// in FIFO order, then clear the queue (the message has been "delivered").
    fn flush_link(&mut self, from: usize, to: usize) {
        let queued = self
            .links
            .get_mut(&(from, to))
            .unwrap()
            .drain(..)
            .collect::<Vec<_>>();
        self.deliver(to, &queued);
    }

    /// Flush every link in the fabric, in ascending (from, to) order — the
    /// no-partition baseline: "everyone eventually delivers."
    fn flush_all(&mut self) {
        let keys: Vec<(usize, usize)> = self.links.keys().copied().collect();
        for (from, to) in keys {
            self.flush_link(from, to);
        }
    }

    fn book(&self, node: usize) -> &ServiceBookV2 {
        &self.nodes[node].book
    }

    fn lost_names(&self, node: usize) -> &LostNameMap {
        &self.nodes[node].lost_names
    }
}

// --- fixtures ---

fn hlc(wall_clock: u64, counter: u32, node_id: &str) -> HLC {
    HLC {
        wall_clock,
        counter,
        node_id: node_id.to_string(),
    }
}

fn entry(owner: &str, first_claimed: u64, updated: u64, port: u16) -> ServiceEntryV2 {
    ServiceEntryV2 {
        owner_node_id: owner.to_string(),
        first_claimed_at: hlc(first_claimed, 0, owner),
        updated_at: hlc(updated, 0, owner),
        ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
        port,
        protocol: "_ipp._tcp".to_string(),
        txt: BTreeMap::new(),
        host_mac: None,
    }
}

fn publish_msg(name: &str, e: ServiceEntryV2) -> GossipMessage {
    GossipMessage::ServicePublishV2 {
        name: name.to_string(),
        entry: e,
    }
}

fn unpublish_msg(name: &str, owner: &str, at: HLC) -> GossipMessage {
    GossipMessage::ServiceUnpublishV2 {
        name: name.to_string(),
        owner_node_id: owner.to_string(),
        hlc: at,
    }
}

/// Assert all `nodes` (by index into `net`) hold byte-identical books —
/// structural equality plus the strong postcard-encode form the bead spec
/// calls for.
fn assert_books_converged(net: &Network, nodes: &[usize], what: &str) {
    let first = net.book(nodes[0]);
    for &n in &nodes[1..] {
        assert_eq!(
            net.book(n),
            first,
            "{what}: node {n} diverged from node {}",
            nodes[0]
        );
        assert_eq!(
            postcard::to_allocvec(net.book(n)).unwrap(),
            postcard::to_allocvec(first).unwrap(),
            "{what}: node {n} not byte-identical to node {}",
            nodes[0]
        );
    }
}

// --- 1. CONVERGENCE ---

#[test]
fn publish_on_one_node_converges_on_all_within_one_flush() {
    let mut net = Network::new(&["a", "b", "c"]);
    net.publish_from(0, publish_msg("wiki.mesh", entry("a", 1_000, 1_000, 8080)));

    // Before the flush, only A has it.
    assert!(net.book(0).contains_key("wiki.mesh"));
    assert!(!net.book(1).contains_key("wiki.mesh"));
    assert!(!net.book(2).contains_key("wiki.mesh"));

    net.flush_all();

    assert_books_converged(&net, &[0, 1, 2], "single publish");
    for n in 0..3 {
        assert_eq!(net.book(n).get("wiki.mesh").unwrap().owner_node_id, "a");
    }

    // Convergence bound: a second flush_all (no new messages queued) must be
    // a no-op — one full round was already enough.
    let before = postcard::to_allocvec(net.book(0)).unwrap();
    net.flush_all();
    assert_eq!(
        postcard::to_allocvec(net.book(0)).unwrap(),
        before,
        "flush is idempotent once converged"
    );
}

// --- 2. PARTITION DOUBLE-CLAIM ---

/// Build the partition scenario fresh each time (Network isn't reusable
/// across orderings since flushing drains queues): {A} vs {B,C}; A claims
/// wiki.mesh at t1, B claims wiki.mesh at t2>t1, while partitioned each
/// answers itself. B<->C stay connected throughout (not partitioned from
/// each other) and are flushed immediately; every link touching A is left
/// queued for the caller to heal in whatever order it wants to stress.
fn setup_partition_double_claim() -> Network {
    let mut net = Network::new(&["a", "b", "c"]);

    // A publishes first (earlier HLC) while partitioned from B and C.
    net.publish_from(0, publish_msg("wiki.mesh", entry("a", 1_000, 1_000, 8080)));
    // B publishes second (later HLC) while partitioned from A, but still
    // connected to C.
    net.publish_from(1, publish_msg("wiki.mesh", entry("b", 2_000, 2_000, 9090)));

    // Heal B<->C immediately (they were never partitioned from each other):
    // within-partition, C converges onto B's claim.
    net.flush_link(1, 2);
    net.flush_link(2, 1); // no-op (C never published), but symmetric for realism.

    assert_eq!(
        net.book(0).get("wiki.mesh").unwrap().owner_node_id,
        "a",
        "A only knows its own claim so far"
    );
    assert_eq!(
        net.book(1).get("wiki.mesh").unwrap().owner_node_id,
        "b",
        "B only knows its own claim so far"
    );
    assert_eq!(
        net.book(2).get("wiki.mesh").unwrap().owner_node_id,
        "b",
        "C converged onto B within the B/C partition"
    );

    net
}

/// Every one of the 4 links touching A (A->B, A->C, B->A, C->A) is still
/// queued with exactly the one relevant publish. Flushing them in ANY order
/// must land every node on the same winner: A, whose first_claimed_at (1000)
/// predates B's (2000) — first-writer-wins, not last-gossip-wins.
fn assert_healed_to_a(net: &Network) {
    assert_books_converged(net, &[0, 1, 2], "post-heal partition double-claim");
    for n in 0..3 {
        let winner = net.book(n).get("wiki.mesh").unwrap();
        assert_eq!(
            winner.owner_node_id, "a",
            "node {n} must converge on A (earlier first-claim), zero split-brain"
        );
    }
    // B's node must record the loss with A as winner (FR32).
    let lost = net
        .lost_names(1)
        .get("wiki.mesh")
        .expect("B must record the conflict loss");
    assert_eq!(lost.winner_node_id, "a");
}

#[test]
fn partition_double_claim_heals_to_earlier_first_claim_a_first_order() {
    let mut net = setup_partition_double_claim();
    net.flush_link(0, 1);
    net.flush_link(0, 2);
    net.flush_link(1, 0);
    net.flush_link(2, 0);
    assert_healed_to_a(&net);
}

#[test]
fn partition_double_claim_heals_to_earlier_first_claim_b_first_order() {
    let mut net = setup_partition_double_claim();
    net.flush_link(1, 0);
    net.flush_link(2, 0);
    net.flush_link(0, 1);
    net.flush_link(0, 2);
    assert_healed_to_a(&net);
}

#[test]
fn partition_double_claim_heals_to_earlier_first_claim_interleaved_order() {
    let mut net = setup_partition_double_claim();
    net.flush_link(0, 1);
    net.flush_link(1, 0);
    net.flush_link(0, 2);
    net.flush_link(2, 0);
    assert_healed_to_a(&net);
}

#[test]
fn partition_double_claim_heals_to_earlier_first_claim_reverse_interleaved_order() {
    let mut net = setup_partition_double_claim();
    net.flush_link(2, 0);
    net.flush_link(0, 2);
    net.flush_link(1, 0);
    net.flush_link(0, 1);
    assert_healed_to_a(&net);
}

#[test]
fn partition_double_claim_heals_to_earlier_first_claim_duplicated_delivery() {
    let mut net = setup_partition_double_claim();

    // Capture every queued payload before the normal heal so it can be
    // redelivered afterward.
    let a_to_b = net.peek_link(0, 1);
    let a_to_c = net.peek_link(0, 2);
    let b_to_a = net.peek_link(1, 0);
    let c_to_a = net.peek_link(2, 0);

    // Normal heal, one adversarial order.
    net.flush_link(0, 1);
    net.flush_link(1, 0);
    net.flush_link(0, 2);
    net.flush_link(2, 0);
    assert_healed_to_a(&net);

    // Redeliver every captured message twice more, once forward and once in
    // reverse order, simulating gossip's at-least-once/best-effort replay.
    net.deliver(1, &a_to_b);
    net.deliver(0, &b_to_a);
    net.deliver(2, &a_to_c);
    net.deliver(0, &c_to_a);
    net.deliver(0, &c_to_a);
    net.deliver(0, &b_to_a);
    net.deliver(2, &a_to_c);
    net.deliver(1, &a_to_b);

    assert_healed_to_a(&net);
}

// --- 3. EQUAL-HLC TIEBREAK ---

#[test]
fn equal_first_claimed_at_ties_break_on_node_id_deterministically_on_every_node() {
    let mut net = Network::new(&["a", "b", "c"]);

    // Identical first_claimed_at (wall_clock, counter) for both claimants;
    // only owner_node_id differs. "zulu" > "alpha" lexicographically, so
    // resolve_service_conflict_v2's tiebreak picks "alpha".
    let claim_hlc = 5_000;
    net.publish_from(
        0,
        publish_msg("wiki.mesh", entry("zulu-owner", claim_hlc, claim_hlc, 111)),
    );
    net.publish_from(
        1,
        publish_msg("wiki.mesh", entry("alpha-owner", claim_hlc, claim_hlc, 222)),
    );

    net.flush_all();

    assert_books_converged(&net, &[0, 1, 2], "equal-HLC tiebreak");
    for n in 0..3 {
        let winner = net.book(n).get("wiki.mesh").unwrap();
        assert_eq!(
            winner.owner_node_id, "alpha-owner",
            "node {n} must apply the same deterministic node-id tiebreak"
        );
    }
}

// --- 4. TOMBSTONE PROPAGATION ---

#[test]
fn tombstone_propagates_revives_and_rejects_forged_unpublish_on_every_node() {
    let mut net = Network::new(&["a", "b", "c"]);

    // Publish + converge.
    net.publish_from(0, publish_msg("wiki.mesh", entry("a", 1_000, 1_000, 8080)));
    net.flush_all();
    assert_books_converged(&net, &[0, 1, 2], "before unpublish");
    assert!(net.book(1).contains_key("wiki.mesh"));

    // Owner (A) unpublishes; all nodes must drop it after one flush.
    net.publish_from(0, unpublish_msg("wiki.mesh", "a", hlc(2_000, 0, "a")));
    net.flush_all();
    for n in 0..3 {
        assert!(
            net.book(n).get("wiki.mesh").is_none(),
            "node {n} must have dropped the unpublished name"
        );
    }

    // A non-owner's forged unpublish (claims to be "a" while sent by node C,
    // pretending a different owner already had it and is un-claiming) is
    // exercised as: C fabricates an unpublish for a *different* owner id than
    // whoever currently holds any (already-vacant) tombstone/live entry —
    // here there is no live entry, so the forged unpublish from a bogus
    // owner must not disturb the real tombstone.
    net.publish_from(
        2,
        unpublish_msg("wiki.mesh", "forger-node", hlc(1_500, 0, "forger-node")),
    );
    net.flush_all();
    for n in 0..3 {
        assert!(net.book(n).get("wiki.mesh").is_none());
    }
    // The real tombstone (owner "a") must be untouched by the forgery on
    // every node that tracks it directly (A, which owns it; B and C never
    // store tombstones locally in this harness beyond what apply gives them
    // — re-derive via a revive attempt below instead, which only succeeds if
    // the forged unpublish did NOT overwrite the tombstone's owner/hlc).

    // Revive: same owner (A) republishes with a newer HLC than the
    // tombstone -> all nodes must serve it again.
    net.publish_from(0, publish_msg("wiki.mesh", entry("a", 1_000, 3_000, 8081)));
    net.flush_all();
    assert_books_converged(&net, &[0, 1, 2], "after revive");
    for n in 0..3 {
        let e = net
            .book(n)
            .get("wiki.mesh")
            .expect("revived name must be answerable on every node");
        assert_eq!(e.owner_node_id, "a");
        assert_eq!(e.port, 8081);
    }

    // A genuine non-owner forged unpublish AFTER the revive (live entry now
    // present, owned by "a") must be ignored everywhere.
    net.publish_from(1, unpublish_msg("wiki.mesh", "b", hlc(9_000, 0, "b")));
    net.flush_all();
    for n in 0..3 {
        let e = net
            .book(n)
            .get("wiki.mesh")
            .expect("forged non-owner unpublish must be ignored, entry stays live");
        assert_eq!(e.owner_node_id, "a");
        assert_eq!(e.port, 8081);
    }
}

// --- 5. DUPLICATE / REPLAY IDEMPOTENCE AT SYSTEM LEVEL ---

#[test]
fn duplicate_and_reversed_redelivery_leaves_final_state_unchanged() {
    let mut net = Network::new(&["a", "b", "c"]);

    net.publish_from(0, publish_msg("wiki.mesh", entry("a", 1_000, 1_000, 8080)));
    net.publish_from(0, publish_msg("wiki.mesh", entry("a", 1_000, 2_000, 8081))); // same-owner refresh
    net.publish_from(
        1,
        publish_msg("printer.mesh", entry("b", 1_500, 1_500, 631)),
    );

    // Capture every link's payloads before the first flush so they can be
    // replayed verbatim afterward.
    let mut captured = Vec::new();
    for from in 0..3 {
        for to in 0..3 {
            if from != to {
                captured.push((from, to, net.peek_link(from, to)));
            }
        }
    }

    net.flush_all();
    let converged_snapshot: Vec<Vec<u8>> = (0..3)
        .map(|n| postcard::to_allocvec(net.book(n)).unwrap())
        .collect();
    assert_eq!(converged_snapshot[0], converged_snapshot[1]);
    assert_eq!(converged_snapshot[0], converged_snapshot[2]);

    // Re-deliver every message twice: once forward, once reversed.
    for &(_, to, ref payloads) in &captured {
        net.deliver(to, payloads);
    }
    for &(_, to, ref payloads) in captured.iter().rev() {
        let mut reversed = payloads.clone();
        reversed.reverse();
        net.deliver(to, &reversed);
    }

    for (n, snapshot) in converged_snapshot.iter().enumerate() {
        assert_eq!(
            &postcard::to_allocvec(net.book(n)).unwrap(),
            snapshot,
            "node {n}: replayed/duplicated/reversed delivery must not change converged state"
        );
    }
    assert_books_converged(&net, &[0, 1, 2], "after duplicate/reversed replay");
}
