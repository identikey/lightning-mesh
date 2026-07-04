use std::collections::BTreeMap;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::crdt::hlc::HLC;

/// A mesh-wide service announcement (mDNS-style).
///
/// Keyed by service name at `/services/{name}`. Service expires when the
/// associated device lease (identified by `host_mac`) expires.
///
/// Merge is last-writer-wins on `updated_at` (see [`merge_service`]). Like
/// [`UserEntry`], a service record has no single authoritative announcer — any
/// node that ingests a service advertisement can write it — so LWW with an HLC
/// tie-break (wall_clock → counter → node_id) is what keeps two nodes
/// convergent without a conflict arm.
///
/// Uses `BTreeMap` instead of `HashMap` for deterministic serialization order,
/// which makes postcard round-trip equality straightforward.
///
/// [`UserEntry`]: crate::crdt::users::UserEntry
/// [`merge_service`]: crate::crdt::merge::merge_service
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub hostname: String,
    pub ip: IpAddr,
    pub port: u16,
    pub protocol: String,
    pub txt: BTreeMap<String, String>,
    pub host_mac: [u8; 6],
    /// Hybrid logical clock stamp; newest wins on merge.
    pub updated_at: HLC,
}

/// Mesh-wide service directory: service name → most recent record.
///
/// The key is the fully-qualified service name (e.g. `printer._ipp._tcp`);
/// callers enforce that the map key matches the record they insert (as with
/// [`UserBook`](crate::crdt::users::UserBook)).
pub type ServiceBook = BTreeMap<String, ServiceEntry>;

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::net::Ipv4Addr;

    use super::*;

    fn hlc(wall_clock: u64, counter: u32, node_id: &str) -> HLC {
        HLC {
            wall_clock,
            counter,
            node_id: node_id.to_string(),
        }
    }

    #[test]
    fn postcard_roundtrip() {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/ipp/print".to_string());
        txt.insert("version".to_string(), "2.0".to_string());

        let original = ServiceEntry {
            hostname: "printer".to_string(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            port: 631,
            protocol: "_ipp._tcp".to_string(),
            txt,
            host_mac: [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
            updated_at: hlc(1_700_000_001_000, 0, "router-a"),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: ServiceEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn postcard_roundtrip_no_txt() {
        let original = ServiceEntry {
            hostname: "nas".to_string(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 60)),
            port: 445,
            protocol: "_smb._tcp".to_string(),
            txt: BTreeMap::new(),
            host_mac: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55],
            updated_at: hlc(1_700_000_002_000, 3, "router-b"),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: ServiceEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }
}
