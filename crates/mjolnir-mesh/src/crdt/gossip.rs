use serde::{Deserialize, Serialize};

use crate::crdt::{
    dns::DnsEntry,
    hlc::HLC,
    lease::LeaseEntry,
    peer_addr::PeerAddrEntry,
    service::ServiceEntry,
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
        assert_eq!(postcard::to_allocvec(&msg).unwrap(), postcard::to_allocvec(&decoded).unwrap());
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
                direct_addrs: vec![
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 7000),
                ],
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
}
