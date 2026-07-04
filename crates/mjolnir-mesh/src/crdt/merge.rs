use crate::crdt::peer_addr::PeerAddrEntry;
use crate::crdt::service::ServiceEntry;
use crate::crdt::subnet::SubnetClaim;
use crate::crdt::users::UserEntry;
use std::cmp::Ordering;

/// Result of merging an incoming entry into a CRDT store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult<T> {
    /// Key did not exist locally; inserted.
    Inserted,
    /// Key existed and incoming was strictly newer; replaced.
    Updated,
    /// Key existed and incoming was equal or older; discarded.
    Unchanged,
    /// Conflict on a domain invariant (e.g., same CIDR claimed by two owners).
    /// Caller is responsible for taking conflict-recovery action.
    Conflict { winner: T, loser: T },
}

/// First-writer-wins: lower HLC = first claimer wins.
/// Pure, deterministic. Both routers seeing the same (a, b) pair reach the same verdict.
///
/// Tie-break order: HLC.wall_clock → HLC.counter → HLC.node_id (lexicographic).
/// Inherits Ord from `HLC`.
pub fn resolve_subnet_conflict<'a>(
    a: &'a SubnetClaim,
    b: &'a SubnetClaim,
) -> (&'a SubnetClaim, &'a SubnetClaim) {
    match a.claimed_at.cmp(&b.claimed_at) {
        Ordering::Less => (a, b),
        Ordering::Greater => (b, a),
        Ordering::Equal => {
            // Identical HLC → tie-break on owner_node_id.
            // This branch is hit only when wall_clock + counter + hlc.node_id are
            // all equal but owner_node_id differs. Extremely rare in practice.
            if a.owner_node_id <= b.owner_node_id {
                (a, b)
            } else {
                (b, a)
            }
        }
    }
}

/// Pure function: given the local entry (if any) and an incoming entry for the
/// same CIDR, return the merge result.
///
/// Note: this function does not enforce that `local.cidr == incoming.cidr` —
/// the caller must look up local by CIDR before calling.
pub fn merge_subnet_claim(
    local: Option<&SubnetClaim>,
    incoming: &SubnetClaim,
) -> MergeResult<SubnetClaim> {
    match local {
        None => MergeResult::Inserted,
        Some(existing) => {
            // Same owner means this is a refresh/update, not a conflict.
            if existing.owner_node_id == incoming.owner_node_id {
                match incoming.claimed_at.cmp(&existing.claimed_at) {
                    Ordering::Greater => MergeResult::Updated,
                    _ => MergeResult::Unchanged,
                }
            } else {
                // Different owners → conflict on the claim.
                let (winner, loser) = resolve_subnet_conflict(existing, incoming);
                MergeResult::Conflict {
                    winner: winner.clone(),
                    loser: loser.clone(),
                }
            }
        }
    }
}

/// Last-writer-wins merge for self-announced peer address entries.
///
/// Since only the subject node announces its own entry, there is no conflict
/// arm — a newer `announced_at` always wins outright.
///
/// Note: this function does not enforce that the map key matches
/// `incoming.node_id` — the caller must look up local by `node_id` before
/// calling.
pub fn merge_peer_addr(
    local: Option<&PeerAddrEntry>,
    incoming: &PeerAddrEntry,
) -> MergeResult<PeerAddrEntry> {
    match local {
        None => MergeResult::Inserted,
        Some(existing) => match incoming.announced_at.cmp(&existing.announced_at) {
            Ordering::Greater => MergeResult::Updated,
            _ => MergeResult::Unchanged,
        },
    }
}

/// Last-writer-wins merge for user identity records (bead `2xd`).
///
/// A user record has no single authoritative announcer — any node that ingests
/// an identity submission may write it — so there is no conflict arm: the entry
/// with the newer `updated_at` HLC wins outright. HLC tie-break (wall_clock →
/// counter → node_id) makes the verdict deterministic across nodes, so two
/// peers seeing the same pair converge on the same record.
///
/// Note: this function does not enforce that the map key matches
/// `incoming.username` — the caller must look up local by `username` first.
pub fn merge_user(local: Option<&UserEntry>, incoming: &UserEntry) -> MergeResult<UserEntry> {
    match local {
        None => MergeResult::Inserted,
        Some(existing) => match incoming.updated_at.cmp(&existing.updated_at) {
            Ordering::Greater => MergeResult::Updated,
            _ => MergeResult::Unchanged,
        },
    }
}

/// Last-writer-wins merge for service records (bead `7jb`, the focused `e21`
/// slice the hello.mesh directory needs).
///
/// Like [`merge_user`], a service record has no single authoritative announcer —
/// any node that ingests a service advertisement may write it — so there is no
/// conflict arm: the entry with the newer `updated_at` HLC wins outright. HLC
/// tie-break (wall_clock → counter → node_id) makes the verdict deterministic
/// across nodes, so two peers seeing the same pair converge on the same record.
///
/// Note: this function does not enforce that the map key matches the service
/// name — the caller must look up local by name before calling.
pub fn merge_service(
    local: Option<&ServiceEntry>,
    incoming: &ServiceEntry,
) -> MergeResult<ServiceEntry> {
    match local {
        None => MergeResult::Inserted,
        Some(existing) => match incoming.updated_at.cmp(&existing.updated_at) {
            Ordering::Greater => MergeResult::Updated,
            _ => MergeResult::Unchanged,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use ipnet::IpNet;

    use super::*;
    use crate::crdt::hlc::HLC;

    fn cidr() -> IpNet {
        IpNet::from_str("10.42.1.0/24").unwrap()
    }

    fn claim(owner: &str, wall_clock: u64, counter: u32, hlc_node: &str) -> SubnetClaim {
        SubnetClaim {
            cidr: cidr(),
            owner_node_id: owner.to_string(),
            site_name: None,
            claimed_at: HLC {
                wall_clock,
                counter,
                node_id: hlc_node.to_string(),
            },
        }
    }

    #[test]
    fn inserted_when_no_local() {
        let incoming = claim("router-a", 1_000, 0, "router-a");
        assert!(matches!(merge_subnet_claim(None, &incoming), MergeResult::Inserted));
    }

    #[test]
    fn unchanged_on_duplicate() {
        let entry = claim("router-a", 1_000, 0, "router-a");
        assert!(matches!(
            merge_subnet_claim(Some(&entry), &entry),
            MergeResult::Unchanged
        ));
    }

    #[test]
    fn updated_on_newer_from_same_owner() {
        let local = claim("router-a", 1_000, 0, "router-a");
        let incoming = claim("router-a", 2_000, 0, "router-a");
        assert!(matches!(
            merge_subnet_claim(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    #[test]
    fn unchanged_on_older_from_same_owner() {
        let local = claim("router-a", 2_000, 0, "router-a");
        let incoming = claim("router-a", 1_000, 0, "router-a");
        assert!(matches!(
            merge_subnet_claim(Some(&local), &incoming),
            MergeResult::Unchanged
        ));
    }

    #[test]
    fn conflict_resolves_lower_hlc_wins() {
        let a = claim("router-a", 1_000, 0, "router-a");
        let b = claim("router-b", 2_000, 0, "router-b");
        let result = merge_subnet_claim(Some(&a), &b);
        match result {
            MergeResult::Conflict { winner, loser } => {
                assert_eq!(winner.owner_node_id, "router-a");
                assert_eq!(loser.owner_node_id, "router-b");
            }
            other => panic!("expected Conflict, got {:?}", other),
        }
    }

    #[test]
    fn conflict_tie_break_by_counter() {
        // Equal wall_clock, lower counter wins.
        let a = claim("router-a", 1_000, 0, "router-a");
        let b = claim("router-b", 1_000, 1, "router-b");
        let result = merge_subnet_claim(Some(&a), &b);
        match result {
            MergeResult::Conflict { winner, loser } => {
                assert_eq!(winner.owner_node_id, "router-a");
                assert_eq!(loser.owner_node_id, "router-b");
            }
            other => panic!("expected Conflict, got {:?}", other),
        }
    }

    #[test]
    fn conflict_tie_break_by_hlc_node_id() {
        // Equal wall_clock and counter, lower hlc.node_id wins.
        let a = claim("router-a", 1_000, 0, "aaa");
        let b = claim("router-b", 1_000, 0, "zzz");
        let result = merge_subnet_claim(Some(&a), &b);
        match result {
            MergeResult::Conflict { winner, loser } => {
                assert_eq!(winner.owner_node_id, "router-a");
                assert_eq!(loser.owner_node_id, "router-b");
            }
            other => panic!("expected Conflict, got {:?}", other),
        }
    }

    #[test]
    fn conflict_tie_break_by_owner_node_id_when_hlc_equal() {
        // HLC fully equal (same wall_clock, counter, node_id) → tie-break on owner_node_id.
        let a = claim("aaa-owner", 1_000, 0, "shared-node");
        let b = claim("zzz-owner", 1_000, 0, "shared-node");
        let result = merge_subnet_claim(Some(&a), &b);
        match result {
            MergeResult::Conflict { winner, loser } => {
                assert_eq!(winner.owner_node_id, "aaa-owner");
                assert_eq!(loser.owner_node_id, "zzz-owner");
            }
            other => panic!("expected Conflict, got {:?}", other),
        }
    }

    #[test]
    fn deterministic_across_arg_order() {
        let a = claim("router-a", 1_000, 0, "router-a");
        let b = claim("router-b", 2_000, 0, "router-b");
        let (w1, l1) = resolve_subnet_conflict(&a, &b);
        let (w2, l2) = resolve_subnet_conflict(&b, &a);
        assert_eq!(w1.owner_node_id, w2.owner_node_id);
        assert_eq!(l1.owner_node_id, l2.owner_node_id);
    }

    // --- merge_peer_addr tests ---

    fn peer(node_id: &str, wall_clock: u64, counter: u32) -> PeerAddrEntry {
        PeerAddrEntry {
            node_id: node_id.to_string(),
            direct_addrs: vec![],
            relay_url: None,
            announced_at: HLC {
                wall_clock,
                counter,
                node_id: node_id.to_string(),
            },
        }
    }

    #[test]
    fn peer_addr_inserted_when_no_local() {
        let incoming = peer("node-a", 1_000, 0);
        assert!(matches!(merge_peer_addr(None, &incoming), MergeResult::Inserted));
    }

    #[test]
    fn peer_addr_updated_on_newer_announced_at() {
        let local = peer("node-a", 1_000, 0);
        let incoming = peer("node-a", 2_000, 0);
        assert!(matches!(
            merge_peer_addr(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    #[test]
    fn peer_addr_unchanged_on_equal_announced_at() {
        let entry = peer("node-a", 1_000, 0);
        assert!(matches!(
            merge_peer_addr(Some(&entry), &entry),
            MergeResult::Unchanged
        ));
    }

    #[test]
    fn peer_addr_unchanged_on_older_announced_at() {
        let local = peer("node-a", 2_000, 0);
        let incoming = peer("node-a", 1_000, 0);
        assert!(matches!(
            merge_peer_addr(Some(&local), &incoming),
            MergeResult::Unchanged
        ));
    }

    #[test]
    fn peer_addr_idempotent_same_message_twice() {
        // Receiving the same announcement a second time is Unchanged, not Updated.
        let entry = peer("node-a", 5_000, 3);
        let result1 = merge_peer_addr(None, &entry);
        assert!(matches!(result1, MergeResult::Inserted));
        let result2 = merge_peer_addr(Some(&entry), &entry);
        assert!(matches!(result2, MergeResult::Unchanged));
    }

    #[test]
    fn peer_addr_hlc_counter_breaks_wall_clock_tie() {
        // Same wall_clock, higher counter → newer.
        let local = peer("node-a", 1_000, 0);
        let incoming = PeerAddrEntry {
            announced_at: HLC { wall_clock: 1_000, counter: 1, node_id: "node-a".to_string() },
            ..peer("node-a", 1_000, 0)
        };
        assert!(matches!(
            merge_peer_addr(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    // --- merge_user tests (bead 2xd) ---

    fn user(username: &str, display: &str, wall_clock: u64, counter: u32, node_id: &str) -> UserEntry {
        UserEntry {
            username: username.to_string(),
            display_name: display.to_string(),
            registered_by: node_id.to_string(),
            attrs: BTreeMap::new(),
            updated_at: HLC {
                wall_clock,
                counter,
                node_id: node_id.to_string(),
            },
        }
    }

    #[test]
    fn user_inserted_when_no_local() {
        let incoming = user("ada", "Ada", 1_000, 0, "router-a");
        assert!(matches!(merge_user(None, &incoming), MergeResult::Inserted));
    }

    #[test]
    fn user_updated_on_newer() {
        let local = user("ada", "Ada", 1_000, 0, "router-a");
        let incoming = user("ada", "Ada Lovelace", 2_000, 0, "router-b");
        assert!(matches!(merge_user(Some(&local), &incoming), MergeResult::Updated));
    }

    #[test]
    fn user_unchanged_on_older() {
        let local = user("ada", "Ada Lovelace", 2_000, 0, "router-b");
        let incoming = user("ada", "Ada", 1_000, 0, "router-a");
        assert!(matches!(merge_user(Some(&local), &incoming), MergeResult::Unchanged));
    }

    #[test]
    fn user_unchanged_on_duplicate() {
        let entry = user("ada", "Ada", 5_000, 2, "router-a");
        assert!(matches!(merge_user(Some(&entry), &entry), MergeResult::Unchanged));
    }

    #[test]
    fn user_counter_breaks_wall_clock_tie() {
        // Equal wall_clock, higher counter → newer.
        let local = user("ada", "Ada", 1_000, 0, "router-a");
        let incoming = user("ada", "Ada2", 1_000, 1, "router-a");
        assert!(matches!(merge_user(Some(&local), &incoming), MergeResult::Updated));
    }

    // --- merge_service tests (bead 7jb) ---

    fn service(hostname: &str, port: u16, wall_clock: u64, counter: u32, node_id: &str) -> ServiceEntry {
        use std::net::{IpAddr, Ipv4Addr};
        ServiceEntry {
            hostname: hostname.to_string(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            port,
            protocol: "_ipp._tcp".to_string(),
            txt: BTreeMap::new(),
            host_mac: [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
            updated_at: HLC {
                wall_clock,
                counter,
                node_id: node_id.to_string(),
            },
        }
    }

    #[test]
    fn service_inserted_when_no_local() {
        let incoming = service("printer", 631, 1_000, 0, "router-a");
        assert!(matches!(merge_service(None, &incoming), MergeResult::Inserted));
    }

    #[test]
    fn service_updated_on_newer() {
        let local = service("printer", 631, 1_000, 0, "router-a");
        let incoming = service("printer", 9100, 2_000, 0, "router-b");
        assert!(matches!(merge_service(Some(&local), &incoming), MergeResult::Updated));
    }

    #[test]
    fn service_unchanged_on_older() {
        let local = service("printer", 9100, 2_000, 0, "router-b");
        let incoming = service("printer", 631, 1_000, 0, "router-a");
        assert!(matches!(merge_service(Some(&local), &incoming), MergeResult::Unchanged));
    }

    #[test]
    fn service_unchanged_on_duplicate() {
        let entry = service("printer", 631, 5_000, 2, "router-a");
        assert!(matches!(merge_service(Some(&entry), &entry), MergeResult::Unchanged));
    }

    #[test]
    fn service_counter_breaks_wall_clock_tie() {
        // Equal wall_clock, higher counter → newer.
        let local = service("printer", 631, 1_000, 0, "router-a");
        let incoming = service("printer", 9100, 1_000, 1, "router-a");
        assert!(matches!(merge_service(Some(&local), &incoming), MergeResult::Updated));
    }

    #[test]
    fn service_node_id_breaks_hlc_tie() {
        // Equal wall_clock and counter, higher node_id → newer (deterministic).
        let local = service("printer", 631, 1_000, 0, "aaa");
        let incoming = service("printer", 9100, 1_000, 0, "zzz");
        assert!(matches!(merge_service(Some(&local), &incoming), MergeResult::Updated));
    }
}
