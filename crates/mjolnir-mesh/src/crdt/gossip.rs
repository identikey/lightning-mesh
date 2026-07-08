use serde::{Deserialize, Serialize};

use crate::crdt::{
    dns::DnsEntry,
    hlc::HLC,
    lease::LeaseEntry,
    peer_addr::PeerAddrEntry,
    service::{ServiceEntry, ServiceEntryV2},
    subnet::SubnetClaim,
    users::UserEntry,
};

/// Wire message enum for CRDT gossip replication.
///
/// All variants are serialized with postcard; the enum discriminant is a
/// single byte prefix. Gossip is best-effort; the CRDT merge function
/// handles duplicates, reordering, and lost messages correctly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipMessage {
    LeaseUpdate(LeaseEntry),
    LeaseRelease {
        mac: [u8; 6],
        hlc: HLC,
    },
    DnsUpdate {
        hostname: String,
        entry: DnsEntry,
    },
    ServiceUpdate {
        name: String,
        entry: ServiceEntry,
    },
    SubnetClaimUpdate {
        cidr: String,
        entry: SubnetClaim,
    },
    SubnetClaimRelease {
        cidr: String,
        hlc: HLC,
    },
    /// Self-announced peer address update, keyed by node_id.
    ///
    /// Appended last so existing discriminants are not disturbed; old nodes
    /// that do not recognise this variant will decode-skip it.
    PeerAddrUpdate {
        node_id: String,
        entry: PeerAddrEntry,
    },
    /// User identity record update, keyed by username (hello.mesh front desk,
    /// bead `2xd`). Appended last so existing discriminants are undisturbed;
    /// nodes that predate this variant decode-skip it.
    UserUpdate {
        username: String,
        entry: UserEntry,
    },
    /// Owner-bound service publish (v2, bead e21.2.2). Supersedes
    /// `ServiceUpdate` for new publishers; `ServiceUpdate` itself is frozen
    /// (stop emitting, keep decoding) per D-004. Appended last so existing
    /// discriminants are undisturbed; nodes that predate this variant
    /// decode-error (not decode-skip) on it — see the `mixed_fleet` tests in
    /// this module for the verified safety mechanism (the `GossipSync` recv
    /// loop treats a decode error as log-and-skip, not the enum itself).
    ServicePublishV2 {
        name: String,
        entry: ServiceEntryV2,
    },
    /// Owner-bound service tombstone (v2, bead e21.2.2): records that
    /// `owner_node_id` unpublished `name` at `hlc`. See
    /// `crate::crdt::service_apply` for tombstone-vs-publish ordering
    /// semantics (FR31). Appended last; same mixed-fleet caveat as
    /// `ServicePublishV2`.
    ServiceUnpublishV2 {
        name: String,
        owner_node_id: String,
        hlc: HLC,
    },
    /// Ephemeral per-node liveness beacon (bead e21.9). Carries no CRDT state:
    /// it is authored fresh by the living origin about ITSELF once per
    /// anti-entropy tick, and receivers use it only to refresh an in-memory
    /// [`LivenessTracker`](crate::crdt::liveness::LivenessTracker) — never
    /// merged into a book, never persisted, never relayed. `incarnation` is the
    /// origin's boot wall-clock time (ms); `counter` is a per-boot tick
    /// sequence. See `docs/network-coordination/lane-staleness.md`.
    ///
    /// Appended last so existing discriminants are undisturbed; same mixed-fleet
    /// caveat as `ServicePublishV2` — a node that predates this variant
    /// decode-errors on it, and the `GossipSync` recv loop log-and-skips (a
    /// dropped beacon just means that peer looks stale to the old node, which
    /// has no staleness logic anyway).
    LivenessBeacon {
        node_id: String,
        incarnation: u64,
        counter: u64,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::str::FromStr;

    use ipnet::IpNet;

    use super::*;
    use crate::crdt::peer_addr::PeerAddrEntry;

    fn make_hlc(wall_clock: u64, counter: u32, node_id: &str) -> HLC {
        HLC {
            wall_clock,
            counter,
            node_id: node_id.to_string(),
        }
    }

    #[test]
    fn postcard_roundtrip_lease_update() {
        let msg = GossipMessage::LeaseUpdate(LeaseEntry {
            mac: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42)),
            hostname: Some("laptop".to_string()),
            router_id: "router-a".to_string(),
            expiry: 1_700_000_000,
            hlc: make_hlc(1_700_000_000_000, 0, "router-a"),
        });
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        // Compare via serialized bytes — LeaseEntry fields don't all impl Eq
        assert_eq!(
            postcard::to_allocvec(&msg).unwrap(),
            postcard::to_allocvec(&decoded).unwrap()
        );
    }

    #[test]
    fn postcard_roundtrip_lease_release() {
        let msg = GossipMessage::LeaseRelease {
            mac: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
            hlc: make_hlc(1_700_000_000_000, 1, "router-b"),
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_dns_update() {
        let msg = GossipMessage::DnsUpdate {
            hostname: "laptop".to_string(),
            entry: DnsEntry {
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42)),
                mac: [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
            },
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_service_update() {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/ipp/print".to_string());

        let msg = GossipMessage::ServiceUpdate {
            name: "printer._ipp._tcp".to_string(),
            entry: ServiceEntry {
                hostname: "printer".to_string(),
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
                port: 631,
                protocol: "_ipp._tcp".to_string(),
                txt,
                host_mac: [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
                updated_at: make_hlc(1_700_000_007_000, 0, "router-a"),
            },
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_subnet_claim_update() {
        let msg = GossipMessage::SubnetClaimUpdate {
            cidr: "10.42.1.0_24".to_string(),
            entry: SubnetClaim {
                cidr: IpNet::from_str("10.42.1.0/24").unwrap(),
                owner_node_id: "router-c".to_string(),
                site_name: None,
                claimed_at: make_hlc(1_700_000_002_000, 0, "router-c"),
            },
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_subnet_claim_release() {
        let msg = GossipMessage::SubnetClaimRelease {
            cidr: "10.42.1.0_24".to_string(),
            hlc: make_hlc(1_700_000_003_000, 0, "router-c"),
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_peer_addr_update() {
        let node_id = "abcd1234".repeat(8);
        let msg = GossipMessage::PeerAddrUpdate {
            node_id: node_id.clone(),
            entry: PeerAddrEntry {
                node_id,
                direct_addrs: vec![SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
                    7000,
                )],
                relay_url: Some("https://relay.example.com".to_string()),
                announced_at: make_hlc(1_700_000_004_000, 0, "abcd1234abcd1234"),
            },
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_user_update() {
        use crate::crdt::users::UserEntry;
        let mut attrs = BTreeMap::new();
        attrs.insert("role".to_string(), "guest".to_string());
        let msg = GossipMessage::UserUpdate {
            username: "ada".to_string(),
            entry: UserEntry {
                username: "ada".to_string(),
                display_name: "Ada Lovelace".to_string(),
                registered_by: "router-a".to_string(),
                attrs,
                updated_at: make_hlc(1_700_000_006_000, 0, "router-a"),
            },
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_peer_addr_update_no_relay() {
        let node_id = "deadbeef".repeat(8);
        let msg = GossipMessage::PeerAddrUpdate {
            node_id: node_id.clone(),
            entry: PeerAddrEntry {
                node_id,
                direct_addrs: vec![],
                relay_url: None,
                announced_at: make_hlc(1_700_000_005_000, 1, "deadbeef"),
            },
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    fn v2_entry(
        owner: &str,
        wall_clock: u64,
        counter: u32,
        node_id: &str,
    ) -> crate::crdt::service::ServiceEntryV2 {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/ipp/print".to_string());
        crate::crdt::service::ServiceEntryV2 {
            owner_node_id: owner.to_string(),
            first_claimed_at: make_hlc(wall_clock, counter, node_id),
            updated_at: make_hlc(wall_clock, counter, node_id),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            port: 631,
            protocol: "_ipp._tcp".to_string(),
            txt,
            host_mac: Some([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]),
        }
    }

    #[test]
    fn postcard_roundtrip_service_publish_v2() {
        let msg = GossipMessage::ServicePublishV2 {
            name: "printer._ipp._tcp".to_string(),
            entry: v2_entry("router-a-node-id", 1_700_000_008_000, 0, "router-a-node-id"),
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_service_unpublish_v2() {
        let msg = GossipMessage::ServiceUnpublishV2 {
            name: "printer._ipp._tcp".to_string(),
            owner_node_id: "router-a-node-id".to_string(),
            hlc: make_hlc(1_700_000_009_000, 0, "router-a-node-id"),
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }

    #[test]
    fn postcard_roundtrip_liveness_beacon() {
        let msg = GossipMessage::LivenessBeacon {
            node_id: "abcd1234".repeat(8),
            incarnation: 1_700_000_020_000,
            counter: 42,
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let decoded: GossipMessage = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&decoded).unwrap());
    }
}

/// Mixed-fleet wire-compatibility tests (bead e21.2.2, FLAGGED thorough).
///
/// The one unrecoverable failure class here is an old node's decode of a
/// message containing one of the two new variants. This module pins a copy
/// of the CURRENT fleet's `GossipMessage` enum (through `UserUpdate` —
/// exactly the decoder shipped in bead 0yb / commit 55a24c7, before this
/// story's two variants existed) and verifies, empirically, what postcard
/// actually does when that old decoder is fed bytes encoded with the new
/// variants.
///
/// FINDING: postcard's enum discriminant is a varint index into the
/// variant list. An old decoder that has never heard of discriminant 8 or 9
/// (`ServicePublishV2` / `ServiceUnpublishV2`) does NOT silently skip the
/// unknown variant's payload — it returns `Err(postcard::Error::...)` on the
/// unrecognized discriminant. Postcard itself provides no forward-compat
/// skip. The safety net is entirely at the `GossipSync::run` recv-loop
/// layer (`crdt/sync.rs`), which already treats any decode error as
/// log-and-skip and keeps looping (see `sync::tests::malformed_payload_is_skipped_not_fatal`).
/// This module proves both halves: (1) old-enum decode of new-variant bytes
/// errors, and (2) that error does not propagate past the recv loop.
#[cfg(test)]
mod mixed_fleet {
    use std::collections::BTreeMap;
    use std::net::{IpAddr, Ipv4Addr};

    use bytes::Bytes;
    use serde::{Deserialize, Serialize};

    use super::GossipMessage;
    use crate::crdt::dns::DnsEntry;
    use crate::crdt::hlc::HLC;
    use crate::crdt::lease::LeaseEntry;
    use crate::crdt::peer_addr::PeerAddrEntry;
    use crate::crdt::service::ServiceEntry;
    use crate::crdt::subnet::SubnetClaim;
    use crate::crdt::sync::{GossipError, GossipTransport};
    use crate::crdt::users::UserEntry;

    /// Pinned copy of the fleet enum as it exists TODAY (through
    /// `UserUpdate`, gossip.rs:52 at the time this story started) — i.e. the
    /// decoder an un-upgraded router in the field is actually running.
    /// Deliberately duplicated rather than `#[allow(dead_code)]`-truncating
    /// the real enum, so this pin cannot silently drift when the real enum
    /// changes again later.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    enum OldGossipMessage {
        LeaseUpdate(LeaseEntry),
        LeaseRelease {
            mac: [u8; 6],
            hlc: HLC,
        },
        DnsUpdate {
            hostname: String,
            entry: DnsEntry,
        },
        ServiceUpdate {
            name: String,
            entry: ServiceEntry,
        },
        SubnetClaimUpdate {
            cidr: String,
            entry: SubnetClaim,
        },
        SubnetClaimRelease {
            cidr: String,
            hlc: HLC,
        },
        PeerAddrUpdate {
            node_id: String,
            entry: PeerAddrEntry,
        },
        UserUpdate {
            username: String,
            entry: UserEntry,
        },
    }

    fn hlc(wall_clock: u64, counter: u32, node_id: &str) -> HLC {
        HLC {
            wall_clock,
            counter,
            node_id: node_id.to_string(),
        }
    }

    fn service_publish_v2_bytes() -> Vec<u8> {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/ipp/print".to_string());
        let msg = GossipMessage::ServicePublishV2 {
            name: "printer._ipp._tcp".to_string(),
            entry: crate::crdt::service::ServiceEntryV2 {
                owner_node_id: "router-a".to_string(),
                first_claimed_at: hlc(1_700_000_010_000, 0, "router-a"),
                updated_at: hlc(1_700_000_010_000, 0, "router-a"),
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
                port: 631,
                protocol: "_ipp._tcp".to_string(),
                txt,
                host_mac: Some([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]),
            },
        };
        postcard::to_allocvec(&msg).unwrap()
    }

    fn service_unpublish_v2_bytes() -> Vec<u8> {
        let msg = GossipMessage::ServiceUnpublishV2 {
            name: "printer._ipp._tcp".to_string(),
            owner_node_id: "router-a".to_string(),
            hlc: hlc(1_700_000_011_000, 0, "router-a"),
        };
        postcard::to_allocvec(&msg).unwrap()
    }

    #[test]
    fn old_enum_decode_of_service_publish_v2_bytes_errors() {
        let bytes = service_publish_v2_bytes();
        let result: Result<OldGossipMessage, _> = postcard::from_bytes(&bytes);
        assert!(
            result.is_err(),
            "expected the old (pre-e21.2.2) enum to hard-error on the new \
             ServicePublishV2 discriminant, not silently skip it — postcard \
             provides no forward-compat skip for unknown enum variants"
        );
    }

    #[test]
    fn old_enum_decode_of_service_unpublish_v2_bytes_errors() {
        let bytes = service_unpublish_v2_bytes();
        let result: Result<OldGossipMessage, _> = postcard::from_bytes(&bytes);
        assert!(
            result.is_err(),
            "expected the old (pre-e21.2.2) enum to hard-error on the new \
             ServiceUnpublishV2 discriminant"
        );
    }

    #[test]
    fn old_enum_still_decodes_pre_existing_variants_fine() {
        // Control: the old enum's own variants must still decode cleanly —
        // proves the errors above are specifically about the unknown
        // discriminant, not some unrelated bytes mismatch.
        let msg = GossipMessage::UserUpdate {
            username: "ada".to_string(),
            entry: UserEntry {
                username: "ada".to_string(),
                display_name: "Ada".to_string(),
                registered_by: "router-a".to_string(),
                attrs: BTreeMap::new(),
                updated_at: hlc(1_700_000_012_000, 0, "router-a"),
            },
        };
        let bytes = postcard::to_allocvec(&msg).unwrap();
        let result: Result<OldGossipMessage, _> = postcard::from_bytes(&bytes);
        assert!(result.is_ok());
    }

    /// mpsc-backed test double, mirroring `sync::tests::MockTransport`.
    #[derive(Clone)]
    struct MockTransport {
        tx: std::sync::Arc<tokio::sync::mpsc::Sender<Bytes>>,
        rx: std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Bytes>>>,
    }

    impl MockTransport {
        fn pair() -> (MockTransport, MockTransport) {
            let (a_tx, b_rx) = tokio::sync::mpsc::channel::<Bytes>(256);
            let (b_tx, a_rx) = tokio::sync::mpsc::channel::<Bytes>(256);
            (
                MockTransport {
                    tx: std::sync::Arc::new(a_tx),
                    rx: std::sync::Arc::new(tokio::sync::Mutex::new(a_rx)),
                },
                MockTransport {
                    tx: std::sync::Arc::new(b_tx),
                    rx: std::sync::Arc::new(tokio::sync::Mutex::new(b_rx)),
                },
            )
        }

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

    /// End-to-end proof of the SECOND half of the finding, driven by the
    /// pinned `OldGossipMessage` decoder itself (not the current, upgraded
    /// one — using the real `GossipSync`/`decode()` here would trivially
    /// succeed, since the current decoder DOES know the new variants, and
    /// would prove nothing about an old node).
    ///
    /// This replicates `GossipSync::run`'s exact loop shape
    /// (`recv → decode → on Err, warn+continue; on Ok, dispatch`) but wired
    /// to `OldGossipMessage::decode`, standing in for an un-upgraded router.
    /// It shows the old node survives receiving bytes for the two new
    /// variants (interleaved with a message it does understand) precisely
    /// because the RECV LOOP swallows the decode error — the same
    /// log-and-skip discipline `crdt::sync::GossipSync::run` implements
    /// today, unchanged by this story. That loop shape is the actual
    /// mixed-fleet safety mechanism; postcard's enum decoding itself is not.
    #[tokio::test]
    async fn old_decoder_recv_loop_survives_new_variant_bytes() {
        let (a, b) = MockTransport::pair();

        // Old node's own understood message, interleaved with two payloads
        // it cannot decode (the new variants), then another understood one.
        let old_good_1 = OldGossipMessage::SubnetClaimRelease {
            cidr: "10.42.1.0_24".to_string(),
            hlc: hlc(1_700_000_013_000, 0, "router-c"),
        };
        let old_good_1_bytes = postcard::to_allocvec(&old_good_1).unwrap();
        a.broadcast(Bytes::from(old_good_1_bytes.clone()))
            .await
            .unwrap();
        a.inject_raw(Bytes::from(service_publish_v2_bytes())).await;
        a.inject_raw(Bytes::from(service_unpublish_v2_bytes()))
            .await;
        let old_good_2 = OldGossipMessage::UserUpdate {
            username: "ada".to_string(),
            entry: UserEntry {
                username: "ada".to_string(),
                display_name: "Ada".to_string(),
                registered_by: "router-a".to_string(),
                attrs: BTreeMap::new(),
                updated_at: hlc(1_700_000_014_000, 0, "router-a"),
            },
        };
        let old_good_2_bytes = postcard::to_allocvec(&old_good_2).unwrap();
        a.broadcast(Bytes::from(old_good_2_bytes.clone()))
            .await
            .unwrap();
        drop(a);

        // Old node's recv loop: same shape as GossipSync::run, decoding
        // into OldGossipMessage instead of the current GossipMessage.
        let mut seen: Vec<Vec<u8>> = Vec::new();
        loop {
            let payload = match b.recv().await {
                Ok(p) => p,
                Err(GossipError::Closed) => break,
                Err(e) => panic!("unexpected transport error: {e}"),
            };
            match postcard::from_bytes::<OldGossipMessage>(&payload) {
                Ok(msg) => seen.push(postcard::to_allocvec(&msg).unwrap()),
                Err(_) => continue, // log-and-skip, exactly like GossipSync::run
            }
        }

        assert_eq!(
            seen,
            vec![old_good_1_bytes, old_good_2_bytes],
            "old node must see both messages it understands, in order, with \
             the two new-variant payloads silently skipped rather than \
             killing the loop"
        );
    }
}
