//! Gossip bootstrap candidate selection (bead f8b).
//!
//! At boot and on every zero-neighbor rejoin, the daemon must pick a set of
//! node ids to `join_peers` against. Sprint-0yb used ONLY the static
//! roster/UCI peers, so a node whose configured peers are all down stayed a
//! gossip island even when other live, dialable nodes were known — the
//! addressing for those peers is already resolved (the daemon's `MemoryLookup`
//! is seeded from the address book, claims, and derivation), so the only thing
//! missing was *choosing to dial them*.
//!
//! [`rank_bootstrap_candidates`] fixes that: it unions the roster with the
//! persisted address book and claim owners, then ranks by recency so a rejoin
//! prefers recently-seen peers, and caps the set so a large, mostly-dead fleet
//! cannot turn each retry into a dial storm (the f8b CAUTION). Recency is the
//! e21.9 [`LivenessTracker`] when the node has heard a beacon this run, falling
//! back to the persisted `announced_at` for peers only known from disk — so it
//! works both for a mid-run partition (tracker warm) and a cold reboot (tracker
//! empty, disk recency only).
//!
//! Pure over its inputs (node-id strings), so it is unit-tested here without
//! the daemon's iroh types; the daemon parses the returned strings into
//! `EndpointId`s and dials.

use std::collections::BTreeSet;

use crate::crdt::liveness::LivenessTracker;
use crate::crdt::peer_addr::AddrBook;

/// Compute the ordered, deduplicated, capped set of gossip bootstrap node ids.
///
/// Priority tiers, highest first:
/// 1. **Roster** — the configured/intended peers, in the given order. Always
///    tried first; these are what the operator meant to peer with.
/// 2. **Recently seen** — address-book/claim nodes the [`LivenessTracker`] has a
///    beacon time for (and that are not hard-expired), most-recent first. This
///    is the mid-run partition case: prefer the peer we heard from a minute ago.
/// 3. **Disk-known** — address-book nodes never heard from this run, by
///    persisted `announced_at` descending (the cold-reboot recency proxy).
/// 4. **Claim owners** — nodes known only from a subnet claim (least address
///    info; the daemon derives their address), recency-ordered where known.
///
/// `self_id` is always excluded. Nodes the tracker has marked hard-expired
/// (silent past the e21.9 hard-expiry horizon) are dropped as long dead — but a
/// node merely *stale*, or never heard from at all, is kept: at rejoin we have
/// no neighbors, so "stale" is the expected state of every good candidate.
///
/// `cap` bounds the total returned (dial-storm guard); `cap == 0` yields an
/// empty set.
pub fn rank_bootstrap_candidates(
    roster: &[String],
    addr_book: &AddrBook,
    claim_owner_ids: &[String],
    liveness: &LivenessTracker,
    self_id: &str,
    now_ms: u64,
    cap: usize,
) -> Vec<String> {
    if cap == 0 {
        return Vec::new();
    }

    // Tiers 2 & 3: split the address book by whether we've heard a beacon this
    // run, dropping long-dead (hard-expired) nodes entirely.
    let mut recently_seen: Vec<(&str, u64)> = Vec::new();
    let mut disk_known: Vec<(&str, u64)> = Vec::new();
    for (id, entry) in addr_book.iter() {
        if liveness.is_hard_expired(id, now_ms) {
            continue;
        }
        match liveness.last_seen_ms(id) {
            Some(seen_ms) => recently_seen.push((id.as_str(), seen_ms)),
            None => disk_known.push((id.as_str(), entry.announced_at.wall_clock)),
        }
    }
    // Most-recent first, id as a deterministic tiebreak.
    recently_seen.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    disk_known.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    // Tier 4: claim owners, recency-ordered where the tracker knows them.
    let mut claim_owners: Vec<(&str, u64)> = claim_owner_ids
        .iter()
        .map(|s| s.as_str())
        .filter(|id| !liveness.is_hard_expired(id, now_ms))
        .map(|id| (id, liveness.last_seen_ms(id).unwrap_or(0)))
        .collect();
    claim_owners.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let mut ordered: Vec<String> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let tiers = roster
        .iter()
        .map(|s| s.as_str())
        .chain(recently_seen.into_iter().map(|(id, _)| id))
        .chain(disk_known.into_iter().map(|(id, _)| id))
        .chain(claim_owners.into_iter().map(|(id, _)| id));
    for id in tiers {
        if id == self_id || !seen.insert(id) {
            continue;
        }
        ordered.push(id.to_string());
        if ordered.len() >= cap {
            break;
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::*;
    use crate::crdt::hlc::HLC;
    use crate::crdt::peer_addr::PeerAddrEntry;

    const STALE: u64 = 60_000;
    const HARD: u64 = 3_600_000;

    fn tracker() -> LivenessTracker {
        LivenessTracker::new(STALE, HARD)
    }

    /// An address-book entry for `id` whose self-announced HLC wall clock is
    /// `announced` (the persisted recency proxy).
    fn entry(id: &str, announced: u64) -> PeerAddrEntry {
        PeerAddrEntry {
            node_id: id.to_string(),
            direct_addrs: vec![SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(10, 254, 0, 1)),
                7000,
            )],
            relay_url: None,
            announced_at: HLC {
                wall_clock: announced,
                counter: 0,
                node_id: id.to_string(),
            },
        }
    }

    fn book(entries: &[(&str, u64)]) -> AddrBook {
        entries
            .iter()
            .map(|(id, a)| (id.to_string(), entry(id, *a)))
            .collect()
    }

    fn s(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn roster_only_when_nothing_else_known() {
        let got = rank_bootstrap_candidates(
            &s(&["a", "b"]),
            &AddrBook::new(),
            &[],
            &tracker(),
            "self",
            1_000,
            16,
        );
        assert_eq!(got, s(&["a", "b"]));
    }

    #[test]
    fn self_is_always_excluded() {
        let got = rank_bootstrap_candidates(
            &s(&["self", "a"]),
            &book(&[("self", 5), ("b", 5)]),
            &s(&["self"]),
            &tracker(),
            "self",
            1_000,
            16,
        );
        assert!(!got.contains(&"self".to_string()));
        assert!(got.contains(&"a".to_string()));
        assert!(got.contains(&"b".to_string()));
    }

    #[test]
    fn union_includes_addrbook_and_claim_owners_beyond_roster() {
        // The island bug: roster peer "r" is down, but "ab" (addrbook) and "co"
        // (claim owner) are live and dialable — the union must surface them.
        let got = rank_bootstrap_candidates(
            &s(&["r"]),
            &book(&[("ab", 10)]),
            &s(&["co"]),
            &tracker(),
            "self",
            1_000,
            16,
        );
        assert_eq!(got, s(&["r", "ab", "co"]));
    }

    #[test]
    fn roster_ranks_ahead_of_learned_peers() {
        let mut t = tracker();
        t.observe("ab", 1, 1, 900); // recently seen, but still after roster
        let got =
            rank_bootstrap_candidates(&s(&["r"]), &book(&[("ab", 10)]), &[], &t, "self", 1_000, 16);
        assert_eq!(got[0], "r");
    }

    #[test]
    fn recently_seen_outrank_disk_only_and_sort_by_recency() {
        let mut t = tracker();
        t.observe("recent", 1, 1, 950); // seen just now
        t.observe("older", 1, 1, 400); // seen a while ago
        // "diskonly" has no beacon this run — ranks below both seen peers even
        // though its announced_at is the highest number, because live-seen
        // (tier 2) always beats disk-only (tier 3).
        let addr = book(&[("recent", 1), ("older", 1), ("diskonly", 9_999)]);
        let got = rank_bootstrap_candidates(&[], &addr, &[], &t, "self", 1_000, 16);
        assert_eq!(got, s(&["recent", "older", "diskonly"]));
    }

    #[test]
    fn disk_known_sort_by_announced_at_desc() {
        let addr = book(&[("old", 100), ("new", 900), ("mid", 500)]);
        let got = rank_bootstrap_candidates(&[], &addr, &[], &tracker(), "self", 1_000, 16);
        assert_eq!(got, s(&["new", "mid", "old"]));
    }

    #[test]
    fn hard_expired_peers_are_dropped() {
        let mut t = tracker();
        t.observe("dead", 1, 1, 1_000); // seen at t=1s...
        // ...but it is now way past the hard-expiry horizon -> long dead, drop.
        let now = 1_000 + HARD + 1;
        let addr = book(&[("dead", 5), ("alive", 5)]);
        let got = rank_bootstrap_candidates(&[], &addr, &[], &t, "self", now, 16);
        assert_eq!(got, s(&["alive"])); // "alive" never seen -> disk-known, kept
    }

    #[test]
    fn stale_but_not_hard_expired_is_kept() {
        let mut t = tracker();
        t.observe("quiet", 1, 1, 1_000);
        let now = 1_000 + STALE + 5_000; // stale, but nowhere near hard-expiry
        let addr = book(&[("quiet", 5)]);
        let got = rank_bootstrap_candidates(&[], &addr, &[], &t, "self", now, 16);
        assert_eq!(got, s(&["quiet"]));
    }

    #[test]
    fn cap_bounds_the_set_and_keeps_highest_priority() {
        let addr = book(&[("d1", 900), ("d2", 800), ("d3", 700)]);
        let got =
            rank_bootstrap_candidates(&s(&["r1", "r2"]), &addr, &[], &tracker(), "self", 1_000, 3);
        // Roster first (2), then the single highest-announced disk peer.
        assert_eq!(got, s(&["r1", "r2", "d1"]));
    }

    #[test]
    fn cap_zero_is_empty() {
        let got = rank_bootstrap_candidates(
            &s(&["a"]),
            &book(&[("b", 1)]),
            &[],
            &tracker(),
            "self",
            1_000,
            0,
        );
        assert!(got.is_empty());
    }

    #[test]
    fn duplicates_across_sources_are_deduped_keeping_first() {
        // "x" is in roster, addrbook, AND a claim owner — appears once, in the
        // roster (highest) position.
        let mut t = tracker();
        t.observe("x", 1, 1, 950);
        let got = rank_bootstrap_candidates(
            &s(&["x"]),
            &book(&[("x", 10), ("y", 10)]),
            &s(&["x"]),
            &t,
            "self",
            1_000,
            16,
        );
        assert_eq!(got.iter().filter(|id| *id == "x").count(), 1);
        assert_eq!(got[0], "x");
    }
}
