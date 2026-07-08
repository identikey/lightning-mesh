//! Integration: rendered babeld config lands on disk correctly and round-trips
//! through the atomic-write path.
//!
//! This test exercises the public API surface of `mjolnir-mesh::babel` end-to-end
//! without spawning a real babeld process. The supervisor's process-lifecycle
//! behavior is unit-tested inside the crate; this test verifies the config
//! pipeline (the only piece a downstream consumer wires up themselves).
//!
//! Full netns-based two-site reachability (the original US-009 scope) is in
//! `tests/two_site_netns.rs`, gated `#[ignore]` until the unified daemon entry
//! point exists to bind gossip ↔ CRDT ↔ supervisor together.

use ipnet::Ipv4Net;
use mjolnir_mesh::babel::{BabelConfigInputs, render_babeld_conf, write_atomic_if_changed};
use std::str::FromStr;

#[test]
fn render_and_write_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("babeld.conf");

    let subnet = Ipv4Net::from_str("10.42.1.0/24").unwrap();
    let peers = ["mj-peer-aabbccdd", "mj-peer-eeff0011"];
    let inputs = BabelConfigInputs::new(Some(subnet), &peers);

    let body = render_babeld_conf(&inputs);
    let wrote = write_atomic_if_changed(&path, &body).unwrap();
    assert!(wrote, "first write should create the file");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, body);

    // Required structural elements present.
    // No ge/le qualifiers (kf7): they silently defeat the match in babeld 1.13.
    assert!(on_disk.contains("redistribute ip 10.42.1.0/24 allow"));
    assert!(on_disk.contains("interface mj-peer-aabbccdd type tunnel"));
    assert!(on_disk.contains("interface mj-peer-eeff0011 type tunnel"));
    assert!(on_disk.contains("in ip 10.255.0.0/16 deny"));
    assert!(on_disk.contains("out ip 10.255.0.0/16 deny"));
}

#[test]
fn rewrite_with_added_peer_changes_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("babeld.conf");
    let subnet = Ipv4Net::from_str("10.42.7.0/24").unwrap();

    // First write: one peer.
    let peers_v1 = ["mj-peer-aabbccdd"];
    let body_v1 = render_babeld_conf(&BabelConfigInputs::new(Some(subnet), &peers_v1));
    assert!(write_atomic_if_changed(&path, &body_v1).unwrap());

    // Idempotent re-render.
    assert!(!write_atomic_if_changed(&path, &body_v1).unwrap());

    // Add a peer → content changes → file rewritten.
    let peers_v2 = ["mj-peer-aabbccdd", "mj-peer-99887766"];
    let body_v2 = render_babeld_conf(&BabelConfigInputs::new(Some(subnet), &peers_v2));
    assert_ne!(body_v1, body_v2);
    assert!(write_atomic_if_changed(&path, &body_v2).unwrap());

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("mj-peer-99887766"));
}

#[test]
fn peer_disconnect_drops_interface_line() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("babeld.conf");
    let subnet = Ipv4Net::from_str("10.42.2.0/24").unwrap();

    let with_peer = ["mj-peer-aabbccdd"];
    let without = [];

    let body_connected = render_babeld_conf(&BabelConfigInputs::new(Some(subnet), &with_peer));
    write_atomic_if_changed(&path, &body_connected).unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("mj-peer-aabbccdd")
    );

    let body_disconnected = render_babeld_conf(&BabelConfigInputs::new(Some(subnet), &without));
    write_atomic_if_changed(&path, &body_disconnected).unwrap();
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("mj-peer-aabbccdd")
    );
    // redistribute line still present — this router still owns its subnet.
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("redistribute ip 10.42.2.0/24")
    );
}
