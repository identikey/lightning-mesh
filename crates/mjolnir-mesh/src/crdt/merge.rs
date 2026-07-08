use crate::crdt::peer_addr::PeerAddrEntry;
use crate::crdt::service::{ServiceEntry, ServiceEntryV2, is_reserved_service_name};
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

/// Error returned by [`merge_service_v2`] when `name` is one of
/// [`RESERVED_SERVICE_NAMES`](crate::crdt::service::RESERVED_SERVICE_NAMES).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("service name {0:?} is reserved and cannot be claimed")]
pub struct ReservedServiceName(pub String);

/// First-writer-wins tie-break for owner-bound service conflicts (bead
/// e21.2.1): lower `first_claimed_at` = original claimant wins.
///
/// Mirrors [`resolve_subnet_conflict`]'s shape (Ord on the HLC, then a
/// deterministic `owner_node_id` lexicographic tie-break when the HLCs are
/// exactly equal) so two routers merging the same pair always agree,
/// regardless of argument order.
pub fn resolve_service_conflict_v2<'a>(
    a: &'a ServiceEntryV2,
    b: &'a ServiceEntryV2,
) -> (&'a ServiceEntryV2, &'a ServiceEntryV2) {
    match a.first_claimed_at.cmp(&b.first_claimed_at) {
        Ordering::Less => (a, b),
        Ordering::Greater => (b, a),
        Ordering::Equal => {
            // Identical first-claim HLC → tie-break on owner_node_id.
            if a.owner_node_id <= b.owner_node_id {
                (a, b)
            } else {
                (b, a)
            }
        }
    }
}

/// Owner-bound merge for v2 service records (bead e21.2.1) — the upgrade over
/// [`merge_service`]'s pure LWW, per PRD FR18-FR20.
///
/// - Same `owner_node_id`, newer `updated_at` → `Updated` (the owner is
///   refreshing its own entry; `first_claimed_at` is never touched by a
///   refresh).
/// - Same owner, older-or-equal `updated_at` → `Unchanged`.
/// - Different owner → `Conflict`, resolved first-writer-wins on the
///   *original* `first_claimed_at` (not `updated_at`), so that an owner
///   refreshing its entry can neither weaken nor strengthen its claim.
///   Deterministic `owner_node_id` tie-break when `first_claimed_at` is
///   exactly equal. The result is the same regardless of which side is
///   `local` vs `incoming` — see [`resolve_service_conflict_v2`].
///
/// `name` is the service's map key (not stored on the entry, same convention
/// as [`merge_service`]); entries for
/// [reserved names](crate::crdt::service::RESERVED_SERVICE_NAMES) are
/// rejected outright, before any comparison against `local`.
///
/// Note: as with the other merge fns, this does not enforce that `local` was
/// looked up by `name` — the caller must do that lookup first.
pub fn merge_service_v2(
    name: &str,
    local: Option<&ServiceEntryV2>,
    incoming: &ServiceEntryV2,
) -> Result<MergeResult<ServiceEntryV2>, ReservedServiceName> {
    if is_reserved_service_name(name) {
        return Err(ReservedServiceName(name.to_string()));
    }
    Ok(match local {
        None => MergeResult::Inserted,
        Some(existing) => {
            if existing.owner_node_id == incoming.owner_node_id {
                match incoming.updated_at.cmp(&existing.updated_at) {
                    Ordering::Greater => MergeResult::Updated,
                    _ => MergeResult::Unchanged,
                }
            } else {
                let (winner, loser) = resolve_service_conflict_v2(existing, incoming);
                MergeResult::Conflict {
                    winner: winner.clone(),
                    loser: loser.clone(),
                }
            }
        }
    })
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
        assert!(matches!(
            merge_subnet_claim(None, &incoming),
            MergeResult::Inserted
        ));
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
        assert!(matches!(
            merge_peer_addr(None, &incoming),
            MergeResult::Inserted
        ));
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
            announced_at: HLC {
                wall_clock: 1_000,
                counter: 1,
                node_id: "node-a".to_string(),
            },
            ..peer("node-a", 1_000, 0)
        };
        assert!(matches!(
            merge_peer_addr(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    // --- merge_user tests (bead 2xd) ---

    fn user(
        username: &str,
        display: &str,
        wall_clock: u64,
        counter: u32,
        node_id: &str,
    ) -> UserEntry {
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
        assert!(matches!(
            merge_user(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    #[test]
    fn user_unchanged_on_older() {
        let local = user("ada", "Ada Lovelace", 2_000, 0, "router-b");
        let incoming = user("ada", "Ada", 1_000, 0, "router-a");
        assert!(matches!(
            merge_user(Some(&local), &incoming),
            MergeResult::Unchanged
        ));
    }

    #[test]
    fn user_unchanged_on_duplicate() {
        let entry = user("ada", "Ada", 5_000, 2, "router-a");
        assert!(matches!(
            merge_user(Some(&entry), &entry),
            MergeResult::Unchanged
        ));
    }

    #[test]
    fn user_counter_breaks_wall_clock_tie() {
        // Equal wall_clock, higher counter → newer.
        let local = user("ada", "Ada", 1_000, 0, "router-a");
        let incoming = user("ada", "Ada2", 1_000, 1, "router-a");
        assert!(matches!(
            merge_user(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    // --- merge_service tests (bead 7jb) ---

    fn service(
        hostname: &str,
        port: u16,
        wall_clock: u64,
        counter: u32,
        node_id: &str,
    ) -> ServiceEntry {
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
        assert!(matches!(
            merge_service(None, &incoming),
            MergeResult::Inserted
        ));
    }

    #[test]
    fn service_updated_on_newer() {
        let local = service("printer", 631, 1_000, 0, "router-a");
        let incoming = service("printer", 9100, 2_000, 0, "router-b");
        assert!(matches!(
            merge_service(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    #[test]
    fn service_unchanged_on_older() {
        let local = service("printer", 9100, 2_000, 0, "router-b");
        let incoming = service("printer", 631, 1_000, 0, "router-a");
        assert!(matches!(
            merge_service(Some(&local), &incoming),
            MergeResult::Unchanged
        ));
    }

    #[test]
    fn service_unchanged_on_duplicate() {
        let entry = service("printer", 631, 5_000, 2, "router-a");
        assert!(matches!(
            merge_service(Some(&entry), &entry),
            MergeResult::Unchanged
        ));
    }

    #[test]
    fn service_counter_breaks_wall_clock_tie() {
        // Equal wall_clock, higher counter → newer.
        let local = service("printer", 631, 1_000, 0, "router-a");
        let incoming = service("printer", 9100, 1_000, 1, "router-a");
        assert!(matches!(
            merge_service(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    #[test]
    fn service_node_id_breaks_hlc_tie() {
        // Equal wall_clock and counter, higher node_id → newer (deterministic).
        let local = service("printer", 631, 1_000, 0, "aaa");
        let incoming = service("printer", 9100, 1_000, 0, "zzz");
        assert!(matches!(
            merge_service(Some(&local), &incoming),
            MergeResult::Updated
        ));
    }

    // --- merge_service_v2 tests (bead e21.2.1) ---

    fn service_v2(
        owner: &str,
        first_claimed: (u64, u32, &str),
        updated: (u64, u32, &str),
    ) -> ServiceEntryV2 {
        ServiceEntryV2 {
            owner_node_id: owner.to_string(),
            first_claimed_at: HLC {
                wall_clock: first_claimed.0,
                counter: first_claimed.1,
                node_id: first_claimed.2.to_string(),
            },
            updated_at: HLC {
                wall_clock: updated.0,
                counter: updated.1,
                node_id: updated.2.to_string(),
            },
            ip: std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 50)),
            port: 631,
            protocol: "_ipp._tcp".to_string(),
            txt: BTreeMap::new(),
            host_mac: Some([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]),
        }
    }

    #[test]
    fn v2_inserted_when_no_local() {
        let incoming = service_v2("router-a", (1_000, 0, "router-a"), (1_000, 0, "router-a"));
        assert!(matches!(
            merge_service_v2("printer", None, &incoming),
            Ok(MergeResult::Inserted)
        ));
    }

    // -- same owner, older/newer/equal updated_at --

    #[test]
    fn v2_same_owner_newer_updated_at_is_updated() {
        let local = service_v2("router-a", (1_000, 0, "router-a"), (1_000, 0, "router-a"));
        let incoming = service_v2("router-a", (1_000, 0, "router-a"), (2_000, 0, "router-a"));
        assert!(matches!(
            merge_service_v2("printer", Some(&local), &incoming),
            Ok(MergeResult::Updated)
        ));
    }

    #[test]
    fn v2_same_owner_older_updated_at_is_unchanged() {
        let local = service_v2("router-a", (1_000, 0, "router-a"), (2_000, 0, "router-a"));
        let incoming = service_v2("router-a", (1_000, 0, "router-a"), (1_000, 0, "router-a"));
        assert!(matches!(
            merge_service_v2("printer", Some(&local), &incoming),
            Ok(MergeResult::Unchanged)
        ));
    }

    #[test]
    fn v2_same_owner_equal_updated_at_is_unchanged() {
        let entry = service_v2("router-a", (1_000, 0, "router-a"), (1_000, 0, "router-a"));
        assert!(matches!(
            merge_service_v2("printer", Some(&entry), &entry),
            Ok(MergeResult::Unchanged)
        ));
    }

    #[test]
    fn v2_same_owner_refresh_does_not_change_first_claimed_at() {
        // Refreshing bumps updated_at but must leave first_claimed_at alone;
        // the merge fn doesn't mutate first_claimed_at itself, but a refresh
        // performed by the same owner should still merge as Updated even
        // when the incoming refresh's own first_claimed_at field matches the
        // original (the caller is responsible for carrying it forward
        // unchanged — this test documents that expectation).
        let local = service_v2("router-a", (500, 0, "router-a"), (1_000, 0, "router-a"));
        let incoming = service_v2("router-a", (500, 0, "router-a"), (2_000, 0, "router-a"));
        assert_eq!(local.first_claimed_at, incoming.first_claimed_at);
        assert!(matches!(
            merge_service_v2("printer", Some(&local), &incoming),
            Ok(MergeResult::Updated)
        ));
    }

    // -- different owner: conflict resolved on first_claimed_at --

    #[test]
    fn v2_different_owner_lower_first_claimed_wins() {
        let local = service_v2("router-a", (1_000, 0, "router-a"), (5_000, 0, "router-a"));
        let incoming = service_v2("router-b", (2_000, 0, "router-b"), (2_000, 0, "router-b"));
        match merge_service_v2("printer", Some(&local), &incoming) {
            Ok(MergeResult::Conflict { winner, loser }) => {
                assert_eq!(winner.owner_node_id, "router-a");
                assert_eq!(loser.owner_node_id, "router-b");
            }
            other => panic!("expected Conflict, got {:?}", other),
        }
    }

    #[test]
    fn v2_different_owner_ignores_updated_at_uses_first_claimed_at() {
        // router-b has a much newer updated_at (a later refresh) but its
        // first_claimed_at is still later than router-a's original claim, so
        // router-a (the earlier claimant) must still win.
        let local = service_v2("router-a", (1_000, 0, "router-a"), (1_000, 0, "router-a"));
        let incoming = service_v2("router-b", (9_000, 0, "router-b"), (100_000, 0, "router-b"));
        match merge_service_v2("printer", Some(&local), &incoming) {
            Ok(MergeResult::Conflict { winner, loser }) => {
                assert_eq!(winner.owner_node_id, "router-a");
                assert_eq!(loser.owner_node_id, "router-b");
            }
            other => panic!("expected Conflict, got {:?}", other),
        }
    }

    #[test]
    fn v2_different_owner_equal_first_claimed_tiebreaks_on_owner_node_id() {
        let a = service_v2(
            "aaa-owner",
            (1_000, 0, "shared-node"),
            (1_000, 0, "shared-node"),
        );
        let b = service_v2(
            "zzz-owner",
            (1_000, 0, "shared-node"),
            (1_000, 0, "shared-node"),
        );
        match merge_service_v2("printer", Some(&a), &b) {
            Ok(MergeResult::Conflict { winner, loser }) => {
                assert_eq!(winner.owner_node_id, "aaa-owner");
                assert_eq!(loser.owner_node_id, "zzz-owner");
            }
            other => panic!("expected Conflict, got {:?}", other),
        }
    }

    // -- reserved names --

    #[test]
    fn v2_reserved_name_rejected_case_insensitively() {
        let incoming = service_v2("router-a", (1_000, 0, "router-a"), (1_000, 0, "router-a"));
        assert_eq!(
            merge_service_v2("hello", None, &incoming),
            Err(ReservedServiceName("hello".to_string()))
        );
        assert_eq!(
            merge_service_v2("HELLO", None, &incoming),
            Err(ReservedServiceName("HELLO".to_string()))
        );
        assert_eq!(
            merge_service_v2("Id", None, &incoming),
            Err(ReservedServiceName("Id".to_string()))
        );
        assert!(merge_service_v2("printer", None, &incoming).is_ok());
    }

    #[test]
    fn v2_reserved_name_rejected_even_with_local_present() {
        // Reserved-name rejection happens before any local/incoming
        // comparison, regardless of whether a local entry exists.
        let local = service_v2("router-a", (1_000, 0, "router-a"), (1_000, 0, "router-a"));
        let incoming = service_v2("router-b", (2_000, 0, "router-b"), (2_000, 0, "router-b"));
        assert_eq!(
            merge_service_v2("hello", Some(&local), &incoming),
            Err(ReservedServiceName("hello".to_string()))
        );
    }

    // -- argument-order symmetry property --
    //
    // For every (a, b) pair, merging a-as-local/b-as-incoming and
    // b-as-local/a-as-incoming must select the same surviving entry (NFR7:
    // every node computes the identical winner regardless of gossip order).

    fn assert_symmetric_conflict(a: &ServiceEntryV2, b: &ServiceEntryV2) {
        let a_local = merge_service_v2("printer", Some(a), b).unwrap();
        let b_local = merge_service_v2("printer", Some(b), a).unwrap();
        let winner_from_a_local = match a_local {
            MergeResult::Conflict { winner, .. } => winner.owner_node_id,
            other => panic!("expected Conflict, got {:?}", other),
        };
        let winner_from_b_local = match b_local {
            MergeResult::Conflict { winner, .. } => winner.owner_node_id,
            other => panic!("expected Conflict, got {:?}", other),
        };
        assert_eq!(winner_from_a_local, winner_from_b_local);
    }

    #[test]
    fn v2_conflict_resolution_is_symmetric_distinct_first_claimed() {
        let a = service_v2("router-a", (1_000, 0, "router-a"), (5_000, 0, "router-a"));
        let b = service_v2("router-b", (2_000, 0, "router-b"), (2_000, 0, "router-b"));
        assert_symmetric_conflict(&a, &b);
    }

    #[test]
    fn v2_conflict_resolution_is_symmetric_equal_first_claimed() {
        let a = service_v2(
            "aaa-owner",
            (1_000, 0, "shared-node"),
            (1_000, 0, "shared-node"),
        );
        let b = service_v2(
            "zzz-owner",
            (1_000, 0, "shared-node"),
            (9_000, 0, "shared-node"),
        );
        assert_symmetric_conflict(&a, &b);
    }

    #[test]
    fn v2_resolve_service_conflict_v2_symmetric_helper() {
        let a = service_v2("router-a", (1_000, 0, "router-a"), (1_000, 0, "router-a"));
        let b = service_v2("router-b", (2_000, 0, "router-b"), (2_000, 0, "router-b"));
        let (w1, l1) = resolve_service_conflict_v2(&a, &b);
        let (w2, l2) = resolve_service_conflict_v2(&b, &a);
        assert_eq!(w1.owner_node_id, w2.owner_node_id);
        assert_eq!(l1.owner_node_id, l2.owner_node_id);
    }
}
