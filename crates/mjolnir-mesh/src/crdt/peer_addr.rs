use std::collections::BTreeMap;
use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::crdt::hlc::HLC;

/// A self-announced peer address-book entry, keyed by node id.
///
/// Only the subject node announces its own entry (`node_id` == announcer),
/// so merge is pure last-writer-wins on `announced_at` — no conflict arm.
///
/// Stored at `/peers/{node_id}` in the CRDT address book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAddrEntry {
    /// 64-hex iroh node id (subject == announcer).
    pub node_id: String,
    /// Sorted, deduplicated direct socket addresses for this peer.
    ///
    /// Use [`PeerAddrEntry::new`] to construct with normalization; direct
    /// field construction is allowed for deserialization paths that trust
    /// the sender already normalized.
    pub direct_addrs: Vec<SocketAddr>,
    /// iroh relay URL, if any.
    pub relay_url: Option<String>,
    pub announced_at: HLC,
}

impl PeerAddrEntry {
    /// Construct a new entry, normalizing `direct_addrs` to sorted+deduped order.
    ///
    /// Sort key is the string representation, giving a stable, deterministic
    /// order across platforms without requiring `SocketAddr: Ord`.
    pub fn new(
        node_id: String,
        mut direct_addrs: Vec<SocketAddr>,
        relay_url: Option<String>,
        announced_at: HLC,
    ) -> Self {
        direct_addrs.sort_by_key(|a| a.to_string());
        direct_addrs.dedup();
        Self {
            node_id,
            direct_addrs,
            relay_url,
            announced_at,
        }
    }
}

/// Mesh-wide address book: node_id → most recent self-announced entry.
///
/// The key must equal `entry.node_id`. Callers are responsible for enforcing
/// this invariant; `merge_peer_addr` does not re-check it.
pub type AddrBook = BTreeMap<String, PeerAddrEntry>;

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    fn hlc(wall_clock: u64, counter: u32, node_id: &str) -> HLC {
        HLC {
            wall_clock,
            counter,
            node_id: node_id.to_string(),
        }
    }

    #[test]
    fn postcard_roundtrip_with_relay() {
        let original = PeerAddrEntry {
            node_id: "abcd1234".repeat(8),
            direct_addrs: vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 7000),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), 7001),
            ],
            relay_url: Some("https://relay.example.com".to_string()),
            announced_at: hlc(1_700_000_001_000, 0, "abcd1234abcd1234"),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: PeerAddrEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn postcard_roundtrip_no_relay_no_addrs() {
        let original = PeerAddrEntry {
            node_id: "deadbeef".repeat(8),
            direct_addrs: vec![],
            relay_url: None,
            announced_at: hlc(1_700_000_002_000, 1, "deadbeef"),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: PeerAddrEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn postcard_roundtrip_ipv6_addr() {
        let original = PeerAddrEntry {
            node_id: "cafebabe".repeat(8),
            direct_addrs: vec![SocketAddr::new(
                IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
                7000,
            )],
            relay_url: None,
            announced_at: hlc(1_700_000_003_000, 0, "cafebabe"),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: PeerAddrEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn new_deduplicates_and_sorts() {
        let addr_a = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), 7001);
        let addr_b = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 7000);
        let entry = PeerAddrEntry::new(
            "node-x".to_string(),
            vec![addr_a, addr_b, addr_a],
            None,
            hlc(1_000, 0, "node-x"),
        );
        // "10.0.0.1:7000" < "10.0.0.2:7001", duplicate addr_a removed
        assert_eq!(entry.direct_addrs, vec![addr_b, addr_a]);
    }
}
