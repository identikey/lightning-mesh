//! Lib-side apply layer for v2 service gossip (bead e21.2.2).
//!
//! This is the seam S2.3 (the daemon dispatch arm) calls into: given a
//! decoded `GossipMessage::ServicePublishV2` / `ServiceUnpublishV2` payload
//! plus the current [`ServiceBookV2`] and [`ServiceTombstoneBook`], compute
//! the merge/tombstone outcome and mutate both stores accordingly. Pure and
//! transport-free, same seam shape as [`crate::crdt::merge`].
//!
//! Tombstone semantics (FR31, decision D-004):
//! - A publish for a name with no tombstone and no live entry is a normal
//!   [`merge_service_v2`] insert.
//! - A publish for a name that already has a live entry is a normal
//!   [`merge_service_v2`] call against that entry (the tombstone, if any, is
//!   stale — the name was already revived or never actually vacated).
//! - A publish for a name that is tombstoned (no live entry) only succeeds
//!   if the publisher is the tombstone's own owner AND the publish's
//!   `updated_at` is newer than the tombstone's `hlc` — this is the FR31
//!   "revive" path. Any other publish against a tombstoned, vacant name is
//!   rejected: an older HLC from the same owner is a stale replay, and a
//!   different owner cannot claim the name until the tombstone is GC'd
//!   (deferred, bead 99f) — the owner-bound TOFU model extends past
//!   unpublish.
//! - An unpublish only takes effect if its `owner_node_id` matches the live
//!   entry's owner (a non-owner tombstone is ignored — conflicting owner
//!   claims go through [`merge_service_v2`], not unpublish). If there is no
//!   live entry, the message either refreshes an existing tombstone from the
//!   same owner (HLC-ordered, newer wins) or, if no tombstone exists yet,
//!   records a fresh one.

use crate::crdt::merge::{MergeResult, ReservedServiceName, merge_service_v2};
use crate::crdt::service::{
    LostName, LostNameMap, ServiceBookV2, ServiceEntryV2, ServiceTombstone, ServiceTombstoneBook,
    is_reserved_service_name,
};

/// Outcome of applying a `ServicePublishV2` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishOutcome {
    /// Applied via the normal owner-bound merge (name had a live entry, or
    /// had neither a live entry nor a tombstone).
    Merged(Box<MergeResult<ServiceEntryV2>>),
    /// The name was tombstoned and this publish revived it (same owner,
    /// newer HLC than the tombstone).
    Revived,
    /// The name is tombstoned and this publish does not qualify to revive
    /// it — either a stale replay from the tombstone's own owner, or a
    /// different owner attempting to claim a vacant-but-tombstoned name.
    RejectedByTombstone,
}

/// Outcome of applying a `ServiceUnpublishV2` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnpublishOutcome {
    /// The live entry (owned by the sender) was removed and a tombstone
    /// recorded.
    Unpublished,
    /// No live entry existed; recorded a new tombstone for the sender.
    TombstoneRecorded,
    /// No live entry existed, but a tombstone already existed from the same
    /// owner with an equal-or-newer HLC — this message is a stale replay.
    Unchanged,
    /// Ignored: the sender does not own the live entry (or, when there is
    /// no live entry, does not own the existing tombstone).
    IgnoredNotOwner,
}

/// Apply an incoming `ServicePublishV2` (`name`, `incoming`) to `book`,
/// consulting/updating `tombstones` per the tombstone-vs-publish rules
/// above. Mutates `book` in place when the merge/revival succeeds.
///
/// Reserved-name rejection (shared with [`merge_service_v2`]) is surfaced
/// as `Err` before any tombstone logic runs.
pub fn apply_service_publish_v2(
    book: &mut ServiceBookV2,
    tombstones: &ServiceTombstoneBook,
    name: &str,
    incoming: ServiceEntryV2,
) -> Result<PublishOutcome, ReservedServiceName> {
    if let Some(local) = book.get(name) {
        // Live entry present: tombstone (if any) is moot, go through the
        // normal owner-bound merge.
        let result = merge_service_v2(name, Some(local), &incoming)?;
        apply_merge_result(book, name, &incoming, &result);
        return Ok(PublishOutcome::Merged(Box::new(result)));
    }

    match tombstones.get(name) {
        None => {
            let result = merge_service_v2(name, None, &incoming)?;
            apply_merge_result(book, name, &incoming, &result);
            Ok(PublishOutcome::Merged(Box::new(result)))
        }
        Some(tombstone) => {
            if incoming.owner_node_id == tombstone.owner_node_id
                && incoming.updated_at > tombstone.hlc
            {
                book.insert(name.to_string(), incoming);
                Ok(PublishOutcome::Revived)
            } else {
                Ok(PublishOutcome::RejectedByTombstone)
            }
        }
    }
}

fn apply_merge_result(
    book: &mut ServiceBookV2,
    name: &str,
    incoming: &ServiceEntryV2,
    result: &MergeResult<ServiceEntryV2>,
) {
    match result {
        MergeResult::Inserted | MergeResult::Updated => {
            book.insert(name.to_string(), incoming.clone());
        }
        MergeResult::Conflict { winner, .. } => {
            book.insert(name.to_string(), winner.clone());
        }
        MergeResult::Unchanged => {}
    }
}

/// Error returned by [`publish_service_v2`] — the typed result S3.1's control
/// API surfaces to a caller attempting to claim a service name (bead
/// e21.2.4, FR34).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServicePublishError {
    /// `name` is one of [`RESERVED_SERVICE_NAMES`](crate::crdt::service::RESERVED_SERVICE_NAMES).
    #[error("service name {0:?} is reserved and cannot be claimed")]
    Reserved(String),
    /// `name` was already lost to a conflicting claim by `winner_node_id`
    /// (see [`LostName`]) — a different owner cannot reclaim it until the
    /// tombstone/loss record is GC'd (deferred, bead 99f).
    #[error("service name lost to a conflicting claim by node {winner_node_id:?}")]
    LostToPeer { winner_node_id: String },
}

/// If `outcome` is a `Conflict` where `self_id` is the loser, record the win
/// in `lost_names` (bead e21.2.4, FR32/FR33). The book itself already carries
/// the winner (see [`apply_merge_result`]) — this only updates the
/// local-only bookkeeping that gates future publish attempts and will back a
/// future status/API surface.
fn track_conflict_loss(
    lost_names: &mut LostNameMap,
    self_id: &str,
    name: &str,
    outcome: &PublishOutcome,
) {
    let PublishOutcome::Merged(boxed) = outcome else {
        return;
    };
    let MergeResult::Conflict { winner, loser } = boxed.as_ref() else {
        return;
    };
    if loser.owner_node_id == self_id {
        lost_names.insert(
            name.to_string(),
            LostName {
                winner_node_id: winner.owner_node_id.clone(),
                hlc: winner.first_claimed_at.clone(),
            },
        );
    }
}

/// Like [`apply_service_publish_v2`], but also updates `lost_names` when the
/// merge conflicts and `self_id` is the loser (bead e21.2.4). Used by the
/// gossip dispatch arm, which must always apply what a peer announces
/// (gossip is authoritative merge input) — unlike [`publish_service_v2`],
/// this does NOT gate on an existing `lost_names` entry, since a peer's
/// gossiped claim is not a local publish attempt.
pub fn apply_service_publish_v2_tracking_loss(
    book: &mut ServiceBookV2,
    tombstones: &ServiceTombstoneBook,
    lost_names: &mut LostNameMap,
    self_id: &str,
    name: &str,
    incoming: ServiceEntryV2,
) -> Result<PublishOutcome, ReservedServiceName> {
    let outcome = apply_service_publish_v2(book, tombstones, name, incoming)?;
    track_conflict_loss(lost_names, self_id, name, &outcome);
    Ok(outcome)
}

/// Daemon-facing local publish (bead e21.2.3 FR25, e21.2.4 FR34): the seam
/// S3.1's control API calls to claim/refresh a service name on behalf of
/// THIS node. Rejects reserved names and names already known lost to a
/// different owner's conflicting claim before touching the store; otherwise
/// behaves like [`apply_service_publish_v2_tracking_loss`]. Callers are
/// responsible for the gossip broadcast + persistence side effects (this
/// function only mutates the in-memory stores it's given).
pub fn publish_service_v2(
    book: &mut ServiceBookV2,
    tombstones: &ServiceTombstoneBook,
    lost_names: &mut LostNameMap,
    self_id: &str,
    name: &str,
    incoming: ServiceEntryV2,
) -> Result<PublishOutcome, ServicePublishError> {
    if is_reserved_service_name(name) {
        return Err(ServicePublishError::Reserved(name.to_string()));
    }
    if let Some(lost) = lost_names.get(name) {
        return Err(ServicePublishError::LostToPeer {
            winner_node_id: lost.winner_node_id.clone(),
        });
    }
    apply_service_publish_v2_tracking_loss(book, tombstones, lost_names, self_id, name, incoming)
        .map_err(|ReservedServiceName(n)| ServicePublishError::Reserved(n))
}

/// Apply an incoming `ServiceUnpublishV2` (`name`, `owner_node_id`, `hlc`) to
/// `book` and `tombstones`.
pub fn apply_service_unpublish_v2(
    book: &mut ServiceBookV2,
    tombstones: &mut ServiceTombstoneBook,
    name: &str,
    owner_node_id: &str,
    hlc: crate::crdt::hlc::HLC,
) -> UnpublishOutcome {
    if let Some(local) = book.get(name) {
        if local.owner_node_id != owner_node_id {
            return UnpublishOutcome::IgnoredNotOwner;
        }
        book.remove(name);
        tombstones.insert(
            name.to_string(),
            ServiceTombstone {
                owner_node_id: owner_node_id.to_string(),
                hlc,
            },
        );
        return UnpublishOutcome::Unpublished;
    }

    match tombstones.get(name) {
        None => {
            tombstones.insert(
                name.to_string(),
                ServiceTombstone {
                    owner_node_id: owner_node_id.to_string(),
                    hlc,
                },
            );
            UnpublishOutcome::TombstoneRecorded
        }
        Some(existing) => {
            if existing.owner_node_id != owner_node_id {
                return UnpublishOutcome::IgnoredNotOwner;
            }
            if hlc > existing.hlc {
                tombstones.insert(
                    name.to_string(),
                    ServiceTombstone {
                        owner_node_id: owner_node_id.to_string(),
                        hlc,
                    },
                );
                UnpublishOutcome::TombstoneRecorded
            } else {
                UnpublishOutcome::Unchanged
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;
    use crate::crdt::hlc::HLC;

    fn hlc(wall_clock: u64, counter: u32, node_id: &str) -> HLC {
        HLC {
            wall_clock,
            counter,
            node_id: node_id.to_string(),
        }
    }

    fn entry(owner: &str, wall_clock: u64, counter: u32, node_id: &str) -> ServiceEntryV2 {
        ServiceEntryV2 {
            owner_node_id: owner.to_string(),
            first_claimed_at: hlc(wall_clock, counter, node_id),
            updated_at: hlc(wall_clock, counter, node_id),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            port: 631,
            protocol: "_ipp._tcp".to_string(),
            txt: BTreeMap::new(),
            host_mac: None,
        }
    }

    // --- publish: no tombstone, no local entry ---

    #[test]
    fn publish_inserted_when_no_local_no_tombstone() {
        let mut book = ServiceBookV2::new();
        let tombstones = ServiceTombstoneBook::new();
        let incoming = entry("router-a", 1_000, 0, "router-a");
        let result =
            apply_service_publish_v2(&mut book, &tombstones, "printer", incoming.clone()).unwrap();
        assert_eq!(
            result,
            PublishOutcome::Merged(Box::new(MergeResult::Inserted))
        );
        assert_eq!(book.get("printer"), Some(&incoming));
    }

    // --- publish: live entry present (tombstone, if any, is moot) ---

    #[test]
    fn publish_merges_normally_when_live_entry_present() {
        let mut book = ServiceBookV2::new();
        let local = entry("router-a", 1_000, 0, "router-a");
        book.insert("printer".to_string(), local);
        let tombstones = ServiceTombstoneBook::new();

        let incoming = entry("router-a", 2_000, 0, "router-a");
        let result =
            apply_service_publish_v2(&mut book, &tombstones, "printer", incoming.clone()).unwrap();
        assert_eq!(
            result,
            PublishOutcome::Merged(Box::new(MergeResult::Updated))
        );
        assert_eq!(book.get("printer"), Some(&incoming));
    }

    #[test]
    fn publish_unchanged_ignored_when_live_entry_present_and_older() {
        let mut book = ServiceBookV2::new();
        let local = entry("router-a", 2_000, 0, "router-a");
        book.insert("printer".to_string(), local.clone());
        let tombstones = ServiceTombstoneBook::new();

        let incoming = entry("router-a", 1_000, 0, "router-a");
        let result = apply_service_publish_v2(&mut book, &tombstones, "printer", incoming).unwrap();
        assert_eq!(
            result,
            PublishOutcome::Merged(Box::new(MergeResult::Unchanged))
        );
        assert_eq!(book.get("printer"), Some(&local));
    }

    // --- publish: tombstoned name, vacant (no live entry) ---

    #[test]
    fn publish_older_than_tombstone_rejected_same_owner() {
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();
        tombstones.insert(
            "printer".to_string(),
            ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(2_000, 0, "router-a"),
            },
        );

        // Same owner, but stale HLC (older than the tombstone) — replay, rejected.
        let incoming = entry("router-a", 1_000, 0, "router-a");
        let result = apply_service_publish_v2(&mut book, &tombstones, "printer", incoming).unwrap();
        assert_eq!(result, PublishOutcome::RejectedByTombstone);
        assert!(!book.contains_key("printer"));
    }

    #[test]
    fn publish_equal_to_tombstone_hlc_rejected() {
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();
        tombstones.insert(
            "printer".to_string(),
            ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(2_000, 0, "router-a"),
            },
        );

        let incoming = entry("router-a", 2_000, 0, "router-a");
        let result = apply_service_publish_v2(&mut book, &tombstones, "printer", incoming).unwrap();
        assert_eq!(result, PublishOutcome::RejectedByTombstone);
    }

    #[test]
    fn publish_newer_than_tombstone_same_owner_revives() {
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();
        tombstones.insert(
            "printer".to_string(),
            ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(2_000, 0, "router-a"),
            },
        );

        let incoming = entry("router-a", 3_000, 0, "router-a");
        let result =
            apply_service_publish_v2(&mut book, &tombstones, "printer", incoming.clone()).unwrap();
        assert_eq!(result, PublishOutcome::Revived);
        assert_eq!(book.get("printer"), Some(&incoming));
    }

    #[test]
    fn publish_newer_than_tombstone_different_owner_rejected() {
        // A different owner cannot claim a tombstoned name, even with a
        // newer HLC than the tombstone — only the tombstone's own owner may
        // revive; GC (deferred, 99f) is what eventually reopens the name.
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();
        tombstones.insert(
            "printer".to_string(),
            ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(2_000, 0, "router-a"),
            },
        );

        let incoming = entry("router-b", 9_000, 0, "router-b");
        let result = apply_service_publish_v2(&mut book, &tombstones, "printer", incoming).unwrap();
        assert_eq!(result, PublishOutcome::RejectedByTombstone);
        assert!(!book.contains_key("printer"));
    }

    #[test]
    fn publish_reserved_name_rejected_even_with_tombstone() {
        let mut book = ServiceBookV2::new();
        let tombstones = ServiceTombstoneBook::new();
        let incoming = entry("router-a", 1_000, 0, "router-a");
        let err = apply_service_publish_v2(&mut book, &tombstones, "hello", incoming).unwrap_err();
        assert_eq!(err, ReservedServiceName("hello".to_string()));
    }

    // --- publish: conflict path (different owner, no tombstone, no local -> handled above as Inserted;
    // different owner WITH local entry goes through merge_service_v2's Conflict arm) ---

    #[test]
    fn publish_conflict_installs_the_merge_winner() {
        let mut book = ServiceBookV2::new();
        let local = entry("router-a", 1_000, 0, "router-a"); // earlier first_claimed_at
        book.insert("printer".to_string(), local.clone());
        let tombstones = ServiceTombstoneBook::new();

        let incoming = entry("router-b", 2_000, 0, "router-b"); // later first_claimed_at, loses
        let result =
            apply_service_publish_v2(&mut book, &tombstones, "printer", incoming.clone()).unwrap();
        match result {
            PublishOutcome::Merged(ref boxed)
                if matches!(**boxed, MergeResult::Conflict { .. }) =>
            {
                let MergeResult::Conflict {
                    ref winner,
                    ref loser,
                } = **boxed
                else {
                    unreachable!()
                };
                assert_eq!(winner.owner_node_id, "router-a");
                assert_eq!(loser.owner_node_id, "router-b");
            }
            other => panic!("expected Merged(Conflict), got {:?}", other),
        }
        // The book keeps the winner (the original owner), not the incoming loser.
        assert_eq!(book.get("printer"), Some(&local));
    }

    // --- unpublish: owner matches live entry ---

    #[test]
    fn unpublish_by_owner_removes_entry_and_tombstones() {
        let mut book = ServiceBookV2::new();
        book.insert(
            "printer".to_string(),
            entry("router-a", 1_000, 0, "router-a"),
        );
        let mut tombstones = ServiceTombstoneBook::new();

        let result = apply_service_unpublish_v2(
            &mut book,
            &mut tombstones,
            "printer",
            "router-a",
            hlc(2_000, 0, "router-a"),
        );
        assert_eq!(result, UnpublishOutcome::Unpublished);
        assert!(!book.contains_key("printer"));
        assert_eq!(
            tombstones.get("printer"),
            Some(&ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(2_000, 0, "router-a")
            })
        );
    }

    #[test]
    fn unpublish_by_non_owner_of_live_entry_ignored() {
        let mut book = ServiceBookV2::new();
        book.insert(
            "printer".to_string(),
            entry("router-a", 1_000, 0, "router-a"),
        );
        let mut tombstones = ServiceTombstoneBook::new();

        let result = apply_service_unpublish_v2(
            &mut book,
            &mut tombstones,
            "printer",
            "router-b",
            hlc(2_000, 0, "router-b"),
        );
        assert_eq!(result, UnpublishOutcome::IgnoredNotOwner);
        // Neither the book nor the tombstone store is touched.
        assert!(book.contains_key("printer"));
        assert!(!tombstones.contains_key("printer"));
    }

    // --- unpublish: no live entry ---

    #[test]
    fn unpublish_with_no_live_entry_and_no_tombstone_records_one() {
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();

        let result = apply_service_unpublish_v2(
            &mut book,
            &mut tombstones,
            "printer",
            "router-a",
            hlc(1_000, 0, "router-a"),
        );
        assert_eq!(result, UnpublishOutcome::TombstoneRecorded);
        assert_eq!(
            tombstones.get("printer"),
            Some(&ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(1_000, 0, "router-a")
            })
        );
    }

    #[test]
    fn unpublish_refresh_from_same_owner_updates_tombstone_hlc() {
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();
        tombstones.insert(
            "printer".to_string(),
            ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(1_000, 0, "router-a"),
            },
        );

        let result = apply_service_unpublish_v2(
            &mut book,
            &mut tombstones,
            "printer",
            "router-a",
            hlc(2_000, 0, "router-a"),
        );
        assert_eq!(result, UnpublishOutcome::TombstoneRecorded);
        assert_eq!(
            tombstones.get("printer").unwrap().hlc,
            hlc(2_000, 0, "router-a")
        );
    }

    #[test]
    fn unpublish_stale_replay_from_same_owner_is_unchanged() {
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();
        tombstones.insert(
            "printer".to_string(),
            ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(2_000, 0, "router-a"),
            },
        );

        let result = apply_service_unpublish_v2(
            &mut book,
            &mut tombstones,
            "printer",
            "router-a",
            hlc(1_000, 0, "router-a"),
        );
        assert_eq!(result, UnpublishOutcome::Unchanged);
        assert_eq!(
            tombstones.get("printer").unwrap().hlc,
            hlc(2_000, 0, "router-a")
        );
    }

    #[test]
    fn unpublish_from_different_owner_than_existing_tombstone_ignored() {
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();
        tombstones.insert(
            "printer".to_string(),
            ServiceTombstone {
                owner_node_id: "router-a".to_string(),
                hlc: hlc(1_000, 0, "router-a"),
            },
        );

        let result = apply_service_unpublish_v2(
            &mut book,
            &mut tombstones,
            "printer",
            "router-b",
            hlc(9_000, 0, "router-b"),
        );
        assert_eq!(result, UnpublishOutcome::IgnoredNotOwner);
        assert_eq!(tombstones.get("printer").unwrap().owner_node_id, "router-a");
    }

    // --- full lifecycle: publish -> unpublish -> revive ---

    #[test]
    fn full_lifecycle_publish_unpublish_revive() {
        let mut book = ServiceBookV2::new();
        let mut tombstones = ServiceTombstoneBook::new();

        // 1. First publish.
        let published = entry("router-a", 1_000, 0, "router-a");
        let r1 = apply_service_publish_v2(&mut book, &tombstones, "printer", published).unwrap();
        assert_eq!(r1, PublishOutcome::Merged(Box::new(MergeResult::Inserted)));

        // 2. Owner unpublishes.
        let r2 = apply_service_unpublish_v2(
            &mut book,
            &mut tombstones,
            "printer",
            "router-a",
            hlc(2_000, 0, "router-a"),
        );
        assert_eq!(r2, UnpublishOutcome::Unpublished);
        assert!(!book.contains_key("printer"));

        // 3. A stale, pre-unpublish republish (older HLC) must not resurrect it.
        let stale = entry("router-a", 1_500, 0, "router-a");
        let r3 = apply_service_publish_v2(&mut book, &tombstones, "printer", stale).unwrap();
        assert_eq!(r3, PublishOutcome::RejectedByTombstone);
        assert!(!book.contains_key("printer"));

        // 4. A different owner cannot claim the vacated name.
        let intruder = entry("router-b", 5_000, 0, "router-b");
        let r4 = apply_service_publish_v2(&mut book, &tombstones, "printer", intruder).unwrap();
        assert_eq!(r4, PublishOutcome::RejectedByTombstone);
        assert!(!book.contains_key("printer"));

        // 5. The original owner republishes with a newer HLC than the tombstone: revives.
        let revived = entry("router-a", 3_000, 0, "router-a");
        let r5 =
            apply_service_publish_v2(&mut book, &tombstones, "printer", revived.clone()).unwrap();
        assert_eq!(r5, PublishOutcome::Revived);
        assert_eq!(book.get("printer"), Some(&revived));
    }

    // --- conflict-loss tracking (bead e21.2.4) ---

    #[test]
    fn tracking_loss_records_lost_name_when_self_is_loser() {
        let mut book = ServiceBookV2::new();
        let local = entry("self-node", 2_000, 0, "self-node"); // later first_claimed_at -> loses
        book.insert("printer".to_string(), local);
        let tombstones = ServiceTombstoneBook::new();
        let mut lost_names = LostNameMap::new();

        let incoming = entry("peer-node", 1_000, 0, "peer-node"); // earlier -> wins
        let outcome = apply_service_publish_v2_tracking_loss(
            &mut book,
            &tombstones,
            &mut lost_names,
            "self-node",
            "printer",
            incoming,
        )
        .unwrap();
        assert!(
            matches!(outcome, PublishOutcome::Merged(ref boxed) if matches!(**boxed, MergeResult::Conflict { .. }))
        );
        let lost = lost_names
            .get("printer")
            .expect("self should be recorded as loser");
        assert_eq!(lost.winner_node_id, "peer-node");
        assert_eq!(lost.hlc, hlc(1_000, 0, "peer-node"));
    }

    #[test]
    fn tracking_loss_does_not_record_when_self_is_winner() {
        let mut book = ServiceBookV2::new();
        let local = entry("self-node", 1_000, 0, "self-node"); // earlier -> wins
        book.insert("printer".to_string(), local);
        let tombstones = ServiceTombstoneBook::new();
        let mut lost_names = LostNameMap::new();

        let incoming = entry("peer-node", 2_000, 0, "peer-node"); // later -> loses
        apply_service_publish_v2_tracking_loss(
            &mut book,
            &tombstones,
            &mut lost_names,
            "self-node",
            "printer",
            incoming,
        )
        .unwrap();
        assert!(!lost_names.contains_key("printer"));
    }

    #[test]
    fn tracking_loss_does_not_record_for_non_conflict_outcomes() {
        let mut book = ServiceBookV2::new();
        let tombstones = ServiceTombstoneBook::new();
        let mut lost_names = LostNameMap::new();

        let incoming = entry("self-node", 1_000, 0, "self-node");
        apply_service_publish_v2_tracking_loss(
            &mut book,
            &tombstones,
            &mut lost_names,
            "self-node",
            "printer",
            incoming,
        )
        .unwrap();
        assert!(!lost_names.contains_key("printer"));
    }

    // --- publish_service_v2 (bead e21.2.3/e21.2.4 daemon-facing seam) ---

    #[test]
    fn publish_service_v2_rejects_reserved_name() {
        let mut book = ServiceBookV2::new();
        let tombstones = ServiceTombstoneBook::new();
        let mut lost_names = LostNameMap::new();
        let incoming = entry("self-node", 1_000, 0, "self-node");

        let err = publish_service_v2(
            &mut book,
            &tombstones,
            &mut lost_names,
            "self-node",
            "hello",
            incoming,
        )
        .unwrap_err();
        assert_eq!(err, ServicePublishError::Reserved("hello".to_string()));
    }

    #[test]
    fn publish_service_v2_rejects_name_already_lost() {
        let mut book = ServiceBookV2::new();
        let tombstones = ServiceTombstoneBook::new();
        let mut lost_names = LostNameMap::new();
        lost_names.insert(
            "printer".to_string(),
            LostName {
                winner_node_id: "peer-node".to_string(),
                hlc: hlc(1_000, 0, "peer-node"),
            },
        );

        let incoming = entry("self-node", 5_000, 0, "self-node");
        let err = publish_service_v2(
            &mut book,
            &tombstones,
            &mut lost_names,
            "self-node",
            "printer",
            incoming,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ServicePublishError::LostToPeer {
                winner_node_id: "peer-node".to_string()
            }
        );
        // The gated attempt never touched the book.
        assert!(!book.contains_key("printer"));
    }

    #[test]
    fn publish_service_v2_conflict_records_loss_and_returns_outcome() {
        let mut book = ServiceBookV2::new();
        let local = entry("peer-node", 1_000, 0, "peer-node"); // earlier -> wins
        book.insert("printer".to_string(), local.clone());
        let tombstones = ServiceTombstoneBook::new();
        let mut lost_names = LostNameMap::new();

        let incoming = entry("self-node", 2_000, 0, "self-node"); // later -> self loses
        let outcome = publish_service_v2(
            &mut book,
            &tombstones,
            &mut lost_names,
            "self-node",
            "printer",
            incoming,
        )
        .unwrap();
        assert!(
            matches!(outcome, PublishOutcome::Merged(ref boxed) if matches!(**boxed, MergeResult::Conflict { .. }))
        );
        assert_eq!(
            lost_names.get("printer").unwrap().winner_node_id,
            "peer-node"
        );
        // Book keeps the winner's entry, not self's losing claim.
        assert_eq!(book.get("printer"), Some(&local));
    }

    #[test]
    fn publish_service_v2_succeeds_for_a_fresh_name() {
        let mut book = ServiceBookV2::new();
        let tombstones = ServiceTombstoneBook::new();
        let mut lost_names = LostNameMap::new();

        let incoming = entry("self-node", 1_000, 0, "self-node");
        let outcome = publish_service_v2(
            &mut book,
            &tombstones,
            &mut lost_names,
            "self-node",
            "printer",
            incoming.clone(),
        )
        .unwrap();
        assert_eq!(
            outcome,
            PublishOutcome::Merged(Box::new(MergeResult::Inserted))
        );
        assert_eq!(book.get("printer"), Some(&incoming));
        assert!(lost_names.is_empty());
    }
}
