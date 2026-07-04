use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::crdt::hlc::HLC;

/// A mesh-wide user identity record (hello.mesh front desk — bead `2xd`/`bc7`).
///
/// Keyed by `username` at `/users/{username}`. This is the first record type
/// added to the CRDT after subnet claims and the peer address book; the spike
/// exists to prove a brand-new record type propagates end-to-end over the
/// existing gossip layer before the rest of the front-desk slice is built.
///
/// Merge is last-writer-wins on `updated_at` (see [`merge_user`]). Unlike
/// [`PeerAddrEntry`], a user record has no single authoritative announcer — any
/// node that ingests an identity submission can write it — so LWW with HLC
/// tie-break is what keeps two nodes convergent without a conflict arm.
///
/// Uses `BTreeMap` for `attrs` so postcard serialization is deterministic,
/// mirroring [`ServiceEntry`](crate::crdt::service::ServiceEntry).
///
/// [`PeerAddrEntry`]: crate::crdt::peer_addr::PeerAddrEntry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserEntry {
    /// Stable handle used as the map key (`/users/{username}`).
    pub username: String,
    /// Human-facing name shown at the front desk.
    pub display_name: String,
    /// node_id of the mesh node that ingested this identity submission.
    pub registered_by: String,
    /// Free-form extension attributes (e.g. `role`, `email`), sorted for
    /// deterministic serialization.
    pub attrs: BTreeMap<String, String>,
    /// Hybrid logical clock stamp; newest wins on merge.
    pub updated_at: HLC,
}

/// Mesh-wide user directory: username → most recent record.
///
/// The key must equal `entry.username`; callers enforce that invariant (as with
/// [`AddrBook`](crate::crdt::peer_addr::AddrBook)).
pub type UserBook = BTreeMap<String, UserEntry>;

#[cfg(test)]
mod tests {
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
        let mut attrs = BTreeMap::new();
        attrs.insert("role".to_string(), "guest".to_string());
        attrs.insert("email".to_string(), "ada@example.com".to_string());

        let original = UserEntry {
            username: "ada".to_string(),
            display_name: "Ada Lovelace".to_string(),
            registered_by: "router-a".to_string(),
            attrs,
            updated_at: hlc(1_700_000_001_000, 0, "router-a"),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: UserEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn postcard_roundtrip_no_attrs() {
        let original = UserEntry {
            username: "grace".to_string(),
            display_name: "Grace Hopper".to_string(),
            registered_by: "router-b".to_string(),
            attrs: BTreeMap::new(),
            updated_at: hlc(1_700_000_002_000, 3, "router-b"),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: UserEntry = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }
}
