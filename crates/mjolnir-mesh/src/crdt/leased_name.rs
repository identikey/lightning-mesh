//! Key-owned, leased `.mesh` names (bead mjolnir-mesh-p43, epic `coo`).
//!
//! A plain *client* (not a node) that holds an Ed25519 keypair claims a flat
//! `.mesh` name through the `hello.mesh` front desk (bead `lex`). Unlike
//! [`ServiceEntryV2`](crate::crdt::service::ServiceEntryV2), the owner here is a
//! **client pubkey**, not a node id, and ownership is a **renewable lease** —
//! deliberately reversing e21.2.2 decision D-004 (permanent tombstone) per
//! decision `e1r`, so that losing a key never locks a name forever.
//!
//! ## Two-tier fade
//!
//! - *Resolving* fade is the ephemeral liveness plane (e21.9): the name stops
//!   answering DNS ~60s after the owner's heartbeats stop. That does NOT change
//!   ownership — it's a fast UX signal.
//! - *Ownership* fade is this durable lease. The claim carries an HLC
//!   ([`renewed_at`](LeasedName::renewed_at)); the lease expires at
//!   `renewed_at.wall_clock + `[`LEASE_TTL_MS`]. A same-key renewal (a fresh
//!   HLC) extends it. Only once the lease has lapsed may a *different* key
//!   reclaim the name — and that reclaim is decided **deterministically from the
//!   two records alone** (see [`merge_leased_name`]), so every node converges
//!   without consulting a local wall clock at merge time.
//!
//! ## Trust
//!
//! The claim `sig` is carried on the record so it can be re-verified mesh-wide
//! (the name lane is trustless — a node must not be able to forge a key-owned
//! claim, unlike `/users`). NOTE: meshd-side re-verification on gossip apply is
//! deferred to its own bead — for the demo, nodes trust the ingesting node's
//! ceremony-time verification (bead `lex`). The `ip` is vouched by the ingesting
//! node (the client's lease address); the signed message binds only name+port,
//! not the IP.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::crdt::hlc::HLC;
use crate::crdt::merge::MergeResult;

/// Lease duration: a name whose owner stops renewing becomes reclaimable by a
/// different key this long after its last renewal. 1 hour for the demo
/// (decision `e1r`); fixed, with no tenure/reputation weighting yet (that lives
/// in bead `d0w`).
pub const LEASE_TTL_MS: u64 = 60 * 60 * 1000;

/// A key-owned, leased `.mesh` name. Keyed by the flat name in
/// [`LeasedNameBook`] (same key convention as
/// [`ServiceBookV2`](crate::crdt::service::ServiceBookV2)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedName {
    /// Lowercase-hex Ed25519 public key of the owning client — the authority.
    pub owner_pubkey: String,
    /// Lowercase-hex Ed25519 signature over the canonical claim message
    /// (`mjolnir-name-claim:v1\n{challenge}\n{name}\n{port}`). Carried for
    /// mesh-wide re-verification.
    pub sig: String,
    /// The challenge nonce the owner signed; retained so any node can
    /// reconstruct the signed message to verify `sig`.
    pub challenge: String,
    /// Target address (the client's lease IP), vouched by the ingesting node —
    /// NOT covered by `sig`.
    pub ip: IpAddr,
    pub port: u16,
    /// HLC of this owner's FIRST claim on the name. Preserved verbatim across
    /// renewals; it is the arbitration clock for cross-owner reclaim.
    pub first_claimed_at: HLC,
    /// HLC of the most recent renewal. The lease expires at
    /// `wall_clock + `[`LEASE_TTL_MS`]; a newer value (same owner) extends it.
    pub renewed_at: HLC,
}

/// Mesh-wide leased-name directory: flat name → current owner's lease.
pub type LeasedNameBook = std::collections::BTreeMap<String, LeasedName>;

impl LeasedName {
    /// Wall-clock ms after which this lease has lapsed (last renewal +
    /// [`LEASE_TTL_MS`]). Saturates rather than wrapping at the u64 ceiling.
    pub fn lease_expiry_ms(&self) -> u64 {
        self.renewed_at.wall_clock.saturating_add(LEASE_TTL_MS)
    }

    /// True if the lease has lapsed relative to wall-clock `now_ms`. Used by the
    /// publish gate (a challenger may only claim a name whose incumbent lease is
    /// expired); the *merge* never consults a live clock (see
    /// [`merge_leased_name`]).
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms > self.lease_expiry_ms()
    }
}

/// Decide which of two DIFFERENT-owner claims on the same name holds, purely
/// from the two records. First-writer among live leases wins; a later claim
/// wins ONLY if it was made after the earlier owner's lease had already expired
/// (legitimate reclaim after lapse). Symmetric in `a`/`b` — the incumbent is
/// picked by HLC order, not argument order — so the merge is deterministic
/// regardless of gossip arrival order.
fn cross_owner_winner<'a>(a: &'a LeasedName, b: &'a LeasedName) -> (&'a LeasedName, &'a LeasedName) {
    let (incumbent, challenger) = if a.first_claimed_at <= b.first_claimed_at {
        (a, b)
    } else {
        (b, a)
    };
    // Reclaim is valid iff the challenger's first claim happened strictly after
    // the incumbent's lease expiry. Both are durable fields, so every node
    // computes the same verdict without a local clock.
    if challenger.first_claimed_at.wall_clock > incumbent.lease_expiry_ms() {
        (challenger, incumbent)
    } else {
        (incumbent, challenger)
    }
}

/// Pure merge of an `incoming` leased-name claim against the `existing` record
/// (if any) for the same name. Mirrors
/// [`merge_service_v2`](crate::crdt::merge::merge_service_v2)'s shape:
///
/// - no existing → [`MergeResult::Inserted`]
/// - same owner, strictly-newer `renewed_at` → [`MergeResult::Updated`] (renewal)
/// - same owner, older/equal `renewed_at` → [`MergeResult::Unchanged`] (stale replay)
/// - different owner → [`MergeResult::Conflict`] resolved by [`cross_owner_winner`]
///
/// The caller applies the result to the book (see [`apply_leased_name`]).
pub fn merge_leased_name(
    existing: Option<&LeasedName>,
    incoming: &LeasedName,
) -> MergeResult<LeasedName> {
    let Some(existing) = existing else {
        return MergeResult::Inserted;
    };
    if existing.owner_pubkey == incoming.owner_pubkey {
        return if incoming.renewed_at > existing.renewed_at {
            MergeResult::Updated
        } else {
            MergeResult::Unchanged
        };
    }
    let (winner, loser) = cross_owner_winner(existing, incoming);
    MergeResult::Conflict {
        winner: winner.clone(),
        loser: loser.clone(),
    }
}

/// Merge `incoming` into `book` under `name` and report the outcome. On
/// `Inserted`/`Updated`, and on a `Conflict` the incoming record wins, the book
/// ends up holding the winner; otherwise it is left unchanged.
pub fn apply_leased_name(
    book: &mut LeasedNameBook,
    name: &str,
    incoming: LeasedName,
) -> MergeResult<LeasedName> {
    let result = merge_leased_name(book.get(name), &incoming);
    match &result {
        MergeResult::Inserted | MergeResult::Updated => {
            book.insert(name.to_string(), incoming);
        }
        MergeResult::Conflict { winner, .. } => {
            // Book must always hold the winner, even if `existing` lost.
            if winner.owner_pubkey == incoming.owner_pubkey && *winner == incoming {
                book.insert(name.to_string(), incoming);
            } else {
                book.insert(name.to_string(), winner.clone());
            }
        }
        MergeResult::Unchanged => {}
    }
    result
}

/// The name currently owned by `pubkey`, if any. Backs the one-name-per-key
/// rule: before publishing a fresh claim, a caller checks the key does not
/// already own a *different*, still-live name.
pub fn name_owned_by<'a>(book: &'a LeasedNameBook, pubkey: &str, now_ms: u64) -> Option<&'a str> {
    book.iter()
        .find(|(_, e)| e.owner_pubkey == pubkey && !e.is_expired(now_ms))
        .map(|(name, _)| name.as_str())
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn hlc(wall_clock: u64) -> HLC {
        HLC {
            wall_clock,
            counter: 0,
            node_id: "n".to_string(),
        }
    }

    fn entry(owner: &str, first_ms: u64, renewed_ms: u64) -> LeasedName {
        LeasedName {
            owner_pubkey: owner.to_string(),
            sig: "00".repeat(64),
            challenge: "ab".repeat(32),
            ip: IpAddr::V4(Ipv4Addr::new(10, 42, 5, 23)),
            port: 3000,
            first_claimed_at: hlc(first_ms),
            renewed_at: hlc(renewed_ms),
        }
    }

    #[test]
    fn postcard_roundtrip() {
        let e = entry("aa", 1_000, 1_000);
        let bytes = postcard::to_allocvec(&e).unwrap();
        assert_eq!(postcard::from_bytes::<LeasedName>(&bytes).unwrap(), e);
    }

    #[test]
    fn first_claim_inserts() {
        let mut book = LeasedNameBook::new();
        let r = apply_leased_name(&mut book, "walkie-talkie", entry("aa", 1_000, 1_000));
        assert!(matches!(r, MergeResult::Inserted));
        assert_eq!(book["walkie-talkie"].owner_pubkey, "aa");
    }

    #[test]
    fn same_owner_newer_renewal_updates_and_extends_lease() {
        let mut book = LeasedNameBook::new();
        apply_leased_name(&mut book, "wt", entry("aa", 1_000, 1_000));
        let r = apply_leased_name(&mut book, "wt", entry("aa", 1_000, 2_000));
        assert!(matches!(r, MergeResult::Updated));
        assert_eq!(book["wt"].renewed_at.wall_clock, 2_000);
        assert_eq!(book["wt"].lease_expiry_ms(), 2_000 + LEASE_TTL_MS);
    }

    #[test]
    fn same_owner_stale_renewal_is_unchanged() {
        let mut book = LeasedNameBook::new();
        apply_leased_name(&mut book, "wt", entry("aa", 1_000, 5_000));
        let r = apply_leased_name(&mut book, "wt", entry("aa", 1_000, 3_000));
        assert!(matches!(r, MergeResult::Unchanged));
        assert_eq!(book["wt"].renewed_at.wall_clock, 5_000);
    }

    #[test]
    fn different_owner_cannot_steal_a_live_lease() {
        let mut book = LeasedNameBook::new();
        apply_leased_name(&mut book, "wt", entry("aa", 1_000, 1_000)); // expires 1_000+1h
        // Challenger claims well within the lease window.
        let r = apply_leased_name(&mut book, "wt", entry("bb", 1_000 + 5_000, 1_000 + 5_000));
        match r {
            MergeResult::Conflict { winner, .. } => assert_eq!(winner.owner_pubkey, "aa"),
            other => panic!("expected Conflict, got {other:?}"),
        }
        assert_eq!(book["wt"].owner_pubkey, "aa", "incumbent keeps the name");
    }

    #[test]
    fn different_owner_reclaims_after_lease_lapses() {
        let mut book = LeasedNameBook::new();
        apply_leased_name(&mut book, "wt", entry("aa", 1_000, 1_000)); // expires 1_000+1h
        // Challenger claims AFTER the incumbent's lease has expired.
        let after = 1_000 + LEASE_TTL_MS + 1;
        let r = apply_leased_name(&mut book, "wt", entry("bb", after, after));
        match r {
            MergeResult::Conflict { winner, .. } => assert_eq!(winner.owner_pubkey, "bb"),
            other => panic!("expected Conflict, got {other:?}"),
        }
        assert_eq!(book["wt"].owner_pubkey, "bb", "lapsed name is reclaimed");
    }

    #[test]
    fn reclaim_verdict_is_order_independent() {
        // Whether the challenger arrives as `existing` or as `incoming`, the same
        // key must win — otherwise nodes diverge by gossip arrival order.
        let incumbent = entry("aa", 1_000, 1_000);
        let after = 1_000 + LEASE_TTL_MS + 1;
        let challenger = entry("bb", after, after);

        let a = merge_leased_name(Some(&incumbent), &challenger);
        let b = merge_leased_name(Some(&challenger), &incumbent);
        let win = |r: MergeResult<LeasedName>| match r {
            MergeResult::Conflict { winner, .. } => winner.owner_pubkey,
            other => panic!("expected Conflict, got {other:?}"),
        };
        assert_eq!(win(a), "bb");
        assert_eq!(win(b), "bb");
    }

    #[test]
    fn one_name_per_key_sees_only_live_names() {
        let mut book = LeasedNameBook::new();
        apply_leased_name(&mut book, "wt", entry("aa", 1_000, 1_000));
        // Within the lease, the key is seen to own "wt".
        assert_eq!(name_owned_by(&book, "aa", 2_000), Some("wt"));
        // After expiry, it no longer counts against the one-name rule.
        assert_eq!(name_owned_by(&book, "aa", 1_000 + LEASE_TTL_MS + 1), None);
    }
}
