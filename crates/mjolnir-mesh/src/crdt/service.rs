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

/// Well-known service names that can never be claimed in the `/services/`
/// directory (bead e21.2.1). Matched case-insensitively — names are
/// normalized to lowercase before comparison (see [`is_reserved_service_name`]).
///
/// Shared across the owner-bound merge guard (S2.1), the gossip apply path
/// (S2.2), and the publish surface (S3.1) so all three enforce the same list.
pub const RESERVED_SERVICE_NAMES: &[&str] = &["hello", "id"];

/// True if `name`, compared case-insensitively, is one of
/// [`RESERVED_SERVICE_NAMES`].
pub fn is_reserved_service_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    RESERVED_SERVICE_NAMES
        .iter()
        .any(|reserved| *reserved == lower)
}

/// Owner-bound service entry (v2, bead e21.2.1) — the upgrade over
/// [`ServiceEntry`] (v1, bead 7jb).
///
/// v1 is pure LWW with no single authoritative announcer. v2 introduces an
/// owning node per service name: the *same* owner may freely refresh its
/// entry (LWW on `updated_at`), but a *different* owner claiming the same
/// name is a conflict resolved first-writer-wins on `first_claimed_at` — the
/// HLC of the *original* claim, which a refresh never changes. This is a
/// deliberate semantics change from v1's cross-owner LWW (PRD FR20 / ADR):
/// a service name is claimed on first sight (owner-bound TOFU), not
/// re-claimable by whoever gossips last.
///
/// The service name itself is not stored on the entry; it is the map key in
/// [`ServiceBookV2`], matching v1's [`ServiceBook`] convention.
///
/// See [`merge_service_v2`](crate::crdt::merge::merge_service_v2) for the
/// merge semantics and [`RESERVED_SERVICE_NAMES`] for names that are always
/// rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEntryV2 {
    /// iroh node id of the claiming/owning node (same encoding as
    /// [`PeerAddrEntry::node_id`](crate::crdt::peer_addr::PeerAddrEntry::node_id)).
    pub owner_node_id: String,
    /// HLC of the original claim. Never updated on refresh; this is what
    /// arbitrates cross-owner conflicts (first-writer-wins).
    pub first_claimed_at: HLC,
    /// HLC of the most recent refresh by the owner. Newer wins the
    /// same-owner LWW comparison.
    pub updated_at: HLC,
    pub ip: IpAddr,
    pub port: u16,
    pub protocol: String,
    pub txt: BTreeMap<String, String>,
    pub host_mac: Option<[u8; 6]>,
}

/// Mesh-wide v2 service directory: service name → most recent owner-bound
/// record. Same key convention as [`ServiceBook`].
pub type ServiceBookV2 = BTreeMap<String, ServiceEntryV2>;

/// Tombstone recording that `owner_node_id` unpublished a v2 service name at
/// `hlc` (bead e21.2.2, decision D-004).
///
/// Tombstones are retained indefinitely once written — GC is deferred to
/// bead 99f, so unbounded retention is accepted for now — and gate future
/// publishes to the same name via
/// [`apply_service_publish_v2`](crate::crdt::service_apply::apply_service_publish_v2):
/// a publish older than the tombstone's `hlc` loses (FR31), and only the
/// SAME `owner_node_id` publishing with a newer `hlc` than the tombstone may
/// revive the name. A different owner cannot claim a tombstoned name until
/// the tombstone is GC'd — the owner-bound TOFU model from v2's merge
/// semantics extends past unpublish, not just past publish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTombstone {
    pub owner_node_id: String,
    pub hlc: HLC,
}

/// Tombstone store keyed by service name, same convention as
/// [`ServiceBookV2`].
pub type ServiceTombstoneBook = BTreeMap<String, ServiceTombstone>;

/// Local-only bookkeeping (bead e21.2.4, FR32/FR34): recorded whenever a
/// merge [`Conflict`](crate::crdt::merge::MergeResult::Conflict) makes THIS
/// node the loser for a service name — i.e. some other node's claim on the
/// name is first-writer-wins-earlier than ours. Never gossiped (it is derived
/// purely from local merge outcomes, and every node reaches the same verdict
/// independently from the same CRDT data); persisted alongside the v2
/// book/tombstones purely so a restart doesn't forget a name is lost and
/// briefly allow a doomed republish.
///
/// Gates future local publish attempts to the same name
/// ([`publish_service_v2`](crate::crdt::service_apply::publish_service_v2))
/// so they fail synchronously naming the winner (FR34) instead of silently
/// losing another conflict round-trip, and is kept accessible for a future
/// status/API surface (FR32; not wired to `status` output by this story).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostName {
    pub winner_node_id: String,
    /// The winner's `first_claimed_at` HLC — the arbitration clock, not
    /// necessarily its latest refresh.
    pub hlc: HLC,
}

/// Lost-name map keyed by service name, same convention as [`ServiceBookV2`].
pub type LostNameMap = BTreeMap<String, LostName>;

// --- Stationary device names (bead e21.3) ---
//
// A stationary device (NAS, printer, always-on box) is published as a *scoped*
// service under `<host>.<scope>.mesh`, mechanically a device-published entry in
// the same [`ServiceBookV2`] lane (with `host_mac` populated). The scope segment
// is derived from the publishing node's id, so the name is authority-free and
// Sybil-bounded — a node can only publish under its own scope — and, being two
// labels, structurally cannot collide with (shadow / be shadowed by) a bare
// one-label service or well-known name (`wiki.mesh`, `hello.mesh`). See
// `docs/network-coordination/mesh-naming.md` "Device names: identity-gated".

/// Lowercase RFC 4648 base32 alphabet (no padding). Used only for the short
/// node-scope label; DNS labels are case-insensitive and this alphabet is all
/// `[a-z2-7]`, so scoped device names are safe to type.
const SCOPE_BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Length, in base32 characters, of a derived node-scope label. Four chars
/// encode the top 20 bits of `blake3(node_id)` — human-typable, matching the
/// `nas.n7x3.mesh` shape in the naming doc. Cross-node scope collision is
/// possible but bounded by first-writer-wins on HLC, exactly like the flat
/// service tier (squatting accepted until web-of-trust identity binding).
const SCOPE_LABEL_LEN: usize = 4;

/// Derive the short, human-typable DNS label that scopes a device name to its
/// publishing node (bead e21.3): the first [`SCOPE_LABEL_LEN`] base32 chars of
/// `blake3(node_id)`. Deterministic and dependency-free (no `data-encoding`, so
/// it is available in every build, not just `--features daemon`).
pub fn node_scope_label(node_id: &str) -> String {
    let hash = blake3::hash(node_id.as_bytes());
    let b = hash.as_bytes();
    // Top 24 bits of the digest; we consume the top 20 (4 × 5-bit groups).
    let bits = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
    let mut label = String::with_capacity(SCOPE_LABEL_LEN);
    for i in 0..SCOPE_LABEL_LEN {
        let shift = 19 - 5 * i; // 19, 14, 9, 4 → most-significant group first
        let idx = ((bits >> shift) & 0x1f) as usize;
        label.push(SCOPE_BASE32_ALPHABET[idx] as char);
    }
    label
}

/// Why a proposed device host label is unusable (bead e21.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceHostError {
    /// Empty after trimming.
    Empty,
    /// Longer than 63 chars (the DNS single-label limit).
    TooLong,
    /// Contains a character outside `[a-z0-9-]` (case-folded before checking),
    /// including a `.` — the operator names only the host; the daemon appends
    /// the scope, so a dotted name is a mistake, not a multi-label request.
    InvalidChar,
    /// Begins or ends with `-` (not a valid DNS label).
    HyphenBoundary,
}

/// Validate and normalize a device host label to a single lowercase DNS label
/// (bead e21.3). The operator supplies only the `<host>` part (`nas`,
/// `printer`); the daemon derives and appends `<scope>`.
pub fn normalize_device_host(host: &str) -> Result<String, DeviceHostError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(DeviceHostError::Empty);
    }
    if host.len() > 63 {
        return Err(DeviceHostError::TooLong);
    }
    let lower = host.to_ascii_lowercase();
    if !lower
        .bytes()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err(DeviceHostError::InvalidChar);
    }
    if lower.starts_with('-') || lower.ends_with('-') {
        return Err(DeviceHostError::HyphenBoundary);
    }
    Ok(lower)
}

/// Compose the scoped [`ServiceBookV2`] key for a stationary device published by
/// `node_id`: `<host>.<scope>` (bead e21.3). The resolver keys the book on the
/// full pre-`.mesh` string, so this key resolves at `<host>.<scope>.mesh` with
/// no responder changes.
pub fn device_service_key(host: &str, node_id: &str) -> Result<String, DeviceHostError> {
    let host = normalize_device_host(host)?;
    Ok(format!("{host}.{}", node_scope_label(node_id)))
}

/// Parse a `aa:bb:cc:dd:ee:ff` MAC (case-insensitive, colon-separated) into six
/// bytes for [`ServiceEntryV2::host_mac`] (bead e21.3). Returns `None` on any
/// malformed input.
pub fn parse_host_mac(s: &str) -> Option<[u8; 6]> {
    let mut octets = [0u8; 6];
    let mut parts = s.split(':');
    for slot in &mut octets {
        let part = parts.next()?;
        if part.len() != 2 {
            return None;
        }
        *slot = u8::from_str_radix(part, 16).ok()?;
    }
    if parts.next().is_some() {
        return None; // more than six octets
    }
    Some(octets)
}

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

    // --- ServiceEntryV2 (bead e21.2.1) ---

    fn v2_entry(owner: &str, wall_clock: u64, counter: u32, node_id: &str) -> ServiceEntryV2 {
        let mut txt = BTreeMap::new();
        txt.insert("path".to_string(), "/ipp/print".to_string());
        txt.insert("version".to_string(), "2.0".to_string());
        ServiceEntryV2 {
            owner_node_id: owner.to_string(),
            first_claimed_at: hlc(wall_clock, counter, node_id),
            updated_at: hlc(wall_clock, counter, node_id),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            port: 631,
            protocol: "_ipp._tcp".to_string(),
            txt,
            host_mac: Some([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]),
        }
    }

    #[test]
    fn v2_postcard_roundtrip() {
        let original = v2_entry("router-a-node-id", 1_700_000_001_000, 0, "router-a-node-id");
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: ServiceEntryV2 = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn v2_postcard_roundtrip_no_txt_no_mac() {
        let original = ServiceEntryV2 {
            owner_node_id: "router-b-node-id".to_string(),
            first_claimed_at: hlc(1_700_000_000_000, 0, "router-b-node-id"),
            updated_at: hlc(1_700_000_002_000, 3, "router-b-node-id"),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 60)),
            port: 445,
            protocol: "_smb._tcp".to_string(),
            txt: BTreeMap::new(),
            host_mac: None,
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: ServiceEntryV2 = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    // --- ServiceTombstone (bead e21.2.2) ---

    #[test]
    fn tombstone_postcard_roundtrip() {
        let original = ServiceTombstone {
            owner_node_id: "router-a-node-id".to_string(),
            hlc: hlc(1_700_000_020_000, 0, "router-a-node-id"),
        };
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: ServiceTombstone = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn reserved_names_are_case_insensitive() {
        assert!(is_reserved_service_name("hello"));
        assert!(is_reserved_service_name("Hello"));
        assert!(is_reserved_service_name("HELLO"));
        assert!(is_reserved_service_name("id"));
        assert!(is_reserved_service_name("ID"));
        assert!(!is_reserved_service_name("printer"));
        assert!(!is_reserved_service_name("hello2"));
    }

    // --- Stationary device names (bead e21.3) ---

    #[test]
    fn node_scope_label_is_stable_and_typable() {
        let a = node_scope_label("router-a-node-id");
        // Deterministic across calls.
        assert_eq!(a, node_scope_label("router-a-node-id"));
        // Four chars, all from the lowercase base32 alphabet.
        assert_eq!(a.len(), SCOPE_LABEL_LEN);
        assert!(a.bytes().all(|c| SCOPE_BASE32_ALPHABET.contains(&c)));
    }

    #[test]
    fn node_scope_label_differs_between_nodes() {
        // Different node ids overwhelmingly derive different scopes.
        assert_ne!(
            node_scope_label("router-a-node-id"),
            node_scope_label("router-b-node-id")
        );
    }

    #[test]
    fn device_service_key_is_two_labels_scoped_to_node() {
        let key = device_service_key("nas", "router-a-node-id").unwrap();
        let scope = node_scope_label("router-a-node-id");
        assert_eq!(key, format!("nas.{scope}"));
        // Two labels ⇒ resolves at `nas.<scope>.mesh`, never colliding with a
        // bare one-label service or a reserved well-known name.
        assert_eq!(key.split('.').count(), 2);
        assert!(!is_reserved_service_name(&key));
    }

    #[test]
    fn device_host_is_normalized_and_validated() {
        assert_eq!(normalize_device_host("NAS").unwrap(), "nas");
        assert_eq!(normalize_device_host("  printer ").unwrap(), "printer");
        assert_eq!(normalize_device_host("host-01").unwrap(), "host-01");
        assert_eq!(normalize_device_host(""), Err(DeviceHostError::Empty));
        assert_eq!(normalize_device_host("   "), Err(DeviceHostError::Empty));
        // A dotted name is a mistake — the daemon appends the scope, not the user.
        assert_eq!(
            normalize_device_host("nas.n7x3"),
            Err(DeviceHostError::InvalidChar)
        );
        assert_eq!(
            normalize_device_host("a_b"),
            Err(DeviceHostError::InvalidChar)
        );
        assert_eq!(
            normalize_device_host("-nas"),
            Err(DeviceHostError::HyphenBoundary)
        );
        assert_eq!(
            normalize_device_host("nas-"),
            Err(DeviceHostError::HyphenBoundary)
        );
        assert_eq!(
            normalize_device_host(&"a".repeat(64)),
            Err(DeviceHostError::TooLong)
        );
    }

    #[test]
    fn parse_host_mac_roundtrips_and_rejects_junk() {
        assert_eq!(
            parse_host_mac("de:ad:be:ef:00:01"),
            Some([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01])
        );
        assert_eq!(
            parse_host_mac("DE:AD:BE:EF:00:01"),
            Some([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01])
        );
        assert_eq!(parse_host_mac("de:ad:be:ef:00"), None); // too few
        assert_eq!(parse_host_mac("de:ad:be:ef:00:01:02"), None); // too many
        assert_eq!(parse_host_mac("de-ad-be-ef-00-01"), None); // wrong separator
        assert_eq!(parse_host_mac("dead:be:ef:00:01:02"), None); // bad octet width
        assert_eq!(parse_host_mac("gg:ad:be:ef:00:01"), None); // non-hex
    }
}
