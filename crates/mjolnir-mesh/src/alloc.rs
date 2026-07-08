//! Subnet claim allocation: deterministic preference + collision walk.
//!
//! The allocator lets a router request a subnet of a chosen size from the
//! mesh address space. Sizes are expressed as IPv4 prefix lengths (e.g. /24
//! = 256 addresses, 254 usable hosts; /22 = 1024 addresses, 1022 usable;
//! /16 = 65 536 addresses, 65 534 usable).
//!
//! ## Operator UX model
//!
//! The expected interaction is a TUI/CLI where the operator scrolls a prefix
//! selector with arrow keys: /24 ↔ /23 ↔ /22 ↔ … and a label next to the
//! selector shows the resulting IP count. The library exposes:
//!
//! - [`pick_subnet`] — the allocation primitive (takes the chosen prefix).
//! - [`bump_larger_subnet`] / [`bump_smaller_subnet`] — UI arrow helpers that
//!   step the prefix while honoring sensible bounds.
//! - [`usable_hosts`] — the IP-count label for the selector.
//!
//! ## Conflict semantics
//!
//! CIDR blocks form a tree: two blocks are either disjoint or one fully
//! contains the other. The allocator walks candidate slots at the requested
//! prefix length and rejects any candidate that overlaps an existing claim
//! of any size (a /22 candidate that contains a claimed /24 is rejected, as
//! is a /24 candidate inside a claimed /16). On exhaustion, returns `None` —
//! callers (typically a daemon UI) decide whether to widen the search,
//! shrink the request, or fail loud.

use ipnet::Ipv4Net;
use std::collections::HashSet;
use std::net::Ipv4Addr;

/// The default mesh address space (10.42.0.0/16, 65 536 addresses).
pub const DEFAULT_MESH_SPACE: Ipv4Net = Ipv4Net::new_assert(Ipv4Addr::new(10, 42, 0, 0), 16);

/// Smallest prefix the allocator will hand out for a device subnet.
/// /30 leaves 2 usable hosts — anything smaller has no meaningful device
/// capacity. /31 is reserved for point-to-point tunnel link addressing.
pub const SMALLEST_DEVICE_PREFIX: u8 = 30;

/// Pick a free subnet of `target_prefix_len` within `base`, preferred by
/// hashing `node_id`. Returns `None` if no candidate slot of that size is
/// free in `base`.
///
/// `target_prefix_len` must satisfy `base.prefix_len() <= target_prefix_len <= 32`.
/// Calling with a smaller prefix than the base (i.e. requesting a subnet
/// bigger than the entire mesh address space) panics — this is a programmer
/// error, not a runtime condition.
///
/// `claimed` is the set of currently-known claimed subnets at any prefix
/// length (read from the CRDT `/subnets/` namespace by the caller). A
/// candidate is rejected if it overlaps any entry in `claimed`.
pub fn pick_subnet(
    node_id: &str,
    claimed: &HashSet<Ipv4Net>,
    base: Ipv4Net,
    target_prefix_len: u8,
) -> Option<Ipv4Net> {
    assert!(
        target_prefix_len >= base.prefix_len(),
        "pick_subnet: target_prefix_len {target_prefix_len} cannot be smaller than base prefix {}",
        base.prefix_len()
    );
    assert!(
        target_prefix_len <= 32,
        "pick_subnet: target_prefix_len {target_prefix_len} exceeds 32"
    );

    let slot_bits = target_prefix_len - base.prefix_len();
    // Number of candidate slots at the target size inside `base`.
    // slot_bits == 0 means base IS the candidate — exactly one slot.
    let num_slots: u64 = 1u64 << slot_bits;
    let slot_size_addrs: u32 = if target_prefix_len == 32 {
        1
    } else {
        1u32 << (32 - target_prefix_len)
    };
    let base_addr: u32 = u32::from(base.network());

    // Deterministic preferred slot. Hash bytes 0..8 of blake3(node_id) as
    // a u64 then mod into the slot count. For num_slots up to 2^64 this is
    // fine; for our /16 base + /N >= 16 it's at most 2^16 slots.
    let hash = blake3::hash(node_id.as_bytes());
    let hash_bytes = hash.as_bytes();
    let mut preferred_buf = [0u8; 8];
    preferred_buf.copy_from_slice(&hash_bytes[0..8]);
    let preferred: u64 = u64::from_le_bytes(preferred_buf) % num_slots;

    for offset in 0..num_slots {
        let slot_idx = (preferred + offset) % num_slots;
        let slot_addr_u32 = base_addr.wrapping_add((slot_idx as u32) * slot_size_addrs);
        let candidate = Ipv4Net::new(Ipv4Addr::from(slot_addr_u32), target_prefix_len).ok()?;

        if !overlaps_any(&candidate, claimed) {
            return Some(candidate);
        }
    }
    None
}

/// Pick a free subnet of `target_prefix_len`, automatically falling back to
/// progressively smaller subnets (toward [`SMALLEST_DEVICE_PREFIX`]) when the
/// requested size cannot be placed.
///
/// This is the graceful-degradation wrapper around [`pick_subnet`]. Because the
/// address space fragments, a request can fail even with plenty of free
/// addresses (no contiguous, aligned slot of that size remains). Rather than
/// hard-failing, this hands back the largest subnet that *does* fit at or below
/// the request. Inspect the returned [`Ipv4Net`]'s `prefix_len()` to see the
/// size actually granted: it is always `>= target_prefix_len`, i.e. never
/// larger than requested.
///
/// Returns `None` only when nothing fits all the way down to the `/30` floor.
///
/// `target_prefix_len` must satisfy `base.prefix_len() <= target_prefix_len <= 32`
/// (same contract as [`pick_subnet`]).
pub fn pick_subnet_or_smaller(
    node_id: &str,
    claimed: &HashSet<Ipv4Net>,
    base: Ipv4Net,
    target_prefix_len: u8,
) -> Option<Ipv4Net> {
    let mut prefix = target_prefix_len;
    loop {
        if let Some(net) = pick_subnet(node_id, claimed, base, prefix) {
            return Some(net);
        }
        // Step toward a smaller subnet; `None` means we've hit the /30 floor
        // and nothing fits.
        match bump_smaller_subnet(prefix) {
            Some(next) => prefix = next,
            None => return None,
        }
    }
}

/// True if `candidate` overlaps any subnet in `claimed`. Because CIDR blocks
/// nest cleanly, "overlap" reduces to "one contains the other".
fn overlaps_any(candidate: &Ipv4Net, claimed: &HashSet<Ipv4Net>) -> bool {
    claimed
        .iter()
        .any(|c| c.contains(candidate) || candidate.contains(c))
}

/// Usable host addresses in a CIDR of the given prefix length.
///
/// Convention: total addresses minus network and broadcast for /N ≤ 30.
/// /31 returns 2 (RFC 3021 point-to-point links).
/// /32 returns 1 (single host route).
pub const fn usable_hosts(prefix_len: u8) -> u32 {
    match prefix_len {
        32 => 1,
        31 => 2,
        n if n <= 30 => (1u32 << (32 - n)) - 2,
        _ => 0, // invalid prefix
    }
}

/// Total addresses in a CIDR of the given prefix length, including network
/// and broadcast. Useful as a "this many addresses" label alongside
/// [`usable_hosts`].
pub const fn total_addresses(prefix_len: u8) -> u32 {
    if prefix_len >= 32 {
        1
    } else {
        1u32 << (32 - prefix_len)
    }
}

/// UI helper: step toward a *larger* subnet (more IPs, smaller prefix number).
///
/// `/24 → /23 → /22 → …`. Clamped at `min_prefix` (typically the mesh base
/// prefix — you can't claim a subnet bigger than the entire address space).
/// Returns `None` if already at the bound.
pub fn bump_larger_subnet(prefix_len: u8, min_prefix: u8) -> Option<u8> {
    if prefix_len <= min_prefix {
        None
    } else {
        Some(prefix_len - 1)
    }
}

/// UI helper: step toward a *smaller* subnet (fewer IPs, larger prefix number).
///
/// `/24 → /25 → /26 → …`. Clamped at [`SMALLEST_DEVICE_PREFIX`] (/30 — two
/// usable hosts; smaller is not meaningful for a device subnet). Returns
/// `None` if already at the bound.
pub fn bump_smaller_subnet(prefix_len: u8) -> Option<u8> {
    if prefix_len >= SMALLEST_DEVICE_PREFIX {
        None
    } else {
        Some(prefix_len + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> HashSet<Ipv4Net> {
        HashSet::new()
    }

    // --- pick_subnet: /24 (existing behavior, preserved) ---

    #[test]
    fn deterministic_preference() {
        let a = pick_subnet("node-alpha", &empty(), DEFAULT_MESH_SPACE, 24);
        let b = pick_subnet("node-alpha", &empty(), DEFAULT_MESH_SPACE, 24);
        assert_eq!(a, b);
        assert!(a.is_some());
        assert_eq!(a.unwrap().prefix_len(), 24);
    }

    #[test]
    fn distinct_nodes_get_distinct_preferences_usually() {
        let mut results = HashSet::new();
        for i in 0..50u32 {
            let node_id = format!("node-{i}");
            let subnet = pick_subnet(&node_id, &empty(), DEFAULT_MESH_SPACE, 24).unwrap();
            results.insert(subnet);
        }
        assert!(
            results.len() >= 40,
            "expected at least 40 distinct /24s, got {}",
            results.len()
        );
    }

    #[test]
    fn skips_claimed_subnets() {
        let preferred = pick_subnet("node-x", &empty(), DEFAULT_MESH_SPACE, 24).unwrap();
        let mut claimed = HashSet::new();
        claimed.insert(preferred);
        let next = pick_subnet("node-x", &claimed, DEFAULT_MESH_SPACE, 24).unwrap();
        assert_ne!(next, preferred);
    }

    #[test]
    fn three_routers_pick_non_overlapping() {
        let mut claimed = HashSet::new();
        let r1 = pick_subnet("router-1", &claimed, DEFAULT_MESH_SPACE, 24).unwrap();
        claimed.insert(r1);
        let r2 = pick_subnet("router-2", &claimed, DEFAULT_MESH_SPACE, 24).unwrap();
        claimed.insert(r2);
        let r3 = pick_subnet("router-3", &claimed, DEFAULT_MESH_SPACE, 24).unwrap();
        assert_ne!(r1, r2);
        assert_ne!(r1, r3);
        assert_ne!(r2, r3);
    }

    #[test]
    fn exhaustion_returns_none() {
        let mut claimed = HashSet::new();
        let base_octets = DEFAULT_MESH_SPACE.network().octets();
        for idx in 0u8..=255 {
            let net =
                Ipv4Net::new(Ipv4Addr::new(base_octets[0], base_octets[1], idx, 0), 24).unwrap();
            claimed.insert(net);
        }
        assert_eq!(
            pick_subnet("any-node", &claimed, DEFAULT_MESH_SPACE, 24),
            None
        );
    }

    // --- variable prefix length ---

    #[test]
    fn pick_22_returns_22() {
        let n = pick_subnet("big-site", &empty(), DEFAULT_MESH_SPACE, 22).unwrap();
        assert_eq!(n.prefix_len(), 22);
        // Must align to /22 boundary inside 10.42.0.0/16.
        let octets = n.network().octets();
        assert_eq!(octets[0], 10);
        assert_eq!(octets[1], 42);
        assert_eq!(octets[2] & 0b11, 0, "octet 2 must be /22-aligned");
        assert_eq!(octets[3], 0);
    }

    #[test]
    fn pick_16_returns_the_base_when_free() {
        let n = pick_subnet("only-site", &empty(), DEFAULT_MESH_SPACE, 16).unwrap();
        assert_eq!(n, DEFAULT_MESH_SPACE);
    }

    #[test]
    fn pick_16_returns_none_when_anything_already_claimed() {
        let mut claimed = HashSet::new();
        // One /24 inside the /16 — overlap with any /16 candidate.
        claimed.insert(Ipv4Net::new(Ipv4Addr::new(10, 42, 7, 0), 24).unwrap());
        let n = pick_subnet("only-site", &claimed, DEFAULT_MESH_SPACE, 16);
        assert_eq!(n, None);
    }

    #[test]
    fn pick_24_avoids_overlapping_22() {
        // Pre-claim 10.42.0.0/22 (covers 10.42.0.0–10.42.3.255).
        let mut claimed = HashSet::new();
        claimed.insert(Ipv4Net::new(Ipv4Addr::new(10, 42, 0, 0), 22).unwrap());

        // Any /24 that hashes into 10.42.{0,1,2,3}.0 must be skipped.
        for i in 0..50u32 {
            let node_id = format!("node-{i}");
            let n = pick_subnet(&node_id, &claimed, DEFAULT_MESH_SPACE, 24).unwrap();
            let o2 = n.network().octets()[2];
            assert!(
                !(0..=3).contains(&o2),
                "node-{i} picked {n} which overlaps the pre-claimed /22"
            );
        }
    }

    #[test]
    fn pick_22_avoids_overlapping_24() {
        // Pre-claim a /24 — the /22 that contains it must be skipped.
        let mut claimed = HashSet::new();
        claimed.insert(Ipv4Net::new(Ipv4Addr::new(10, 42, 2, 0), 24).unwrap());
        let n = pick_subnet("router-x", &claimed, DEFAULT_MESH_SPACE, 22).unwrap();
        // 10.42.0.0/22 contains 10.42.2.0/24 → must NOT be returned.
        let bad = Ipv4Net::new(Ipv4Addr::new(10, 42, 0, 0), 22).unwrap();
        assert_ne!(n, bad);
    }

    #[test]
    #[should_panic(expected = "cannot be smaller than base prefix")]
    fn target_smaller_than_base_panics() {
        let _ = pick_subnet("node", &empty(), DEFAULT_MESH_SPACE, 15);
    }

    #[test]
    fn allocation_at_smallest_device_prefix() {
        let n = pick_subnet("tiny-site", &empty(), DEFAULT_MESH_SPACE, 30).unwrap();
        assert_eq!(n.prefix_len(), 30);
        assert_eq!(usable_hosts(30), 2);
    }

    // --- pick_subnet_or_smaller (auto-downgrade) ---

    #[test]
    fn or_smaller_returns_requested_when_it_fits() {
        let n = pick_subnet_or_smaller("node-a", &empty(), DEFAULT_MESH_SPACE, 24).unwrap();
        assert_eq!(
            n.prefix_len(),
            24,
            "should grant the requested size when free"
        );
    }

    #[test]
    fn or_smaller_downgrades_to_largest_that_fits() {
        // One /24 claimed makes a /16 impossible, but the other /17 half is
        // free — so the request for /16 should degrade to a /17, the largest
        // size that still fits.
        let mut claimed = HashSet::new();
        claimed.insert(Ipv4Net::new(Ipv4Addr::new(10, 42, 7, 0), 24).unwrap());
        let n = pick_subnet_or_smaller("only-site", &claimed, DEFAULT_MESH_SPACE, 16).unwrap();
        assert_eq!(n.prefix_len(), 17, "should degrade /16 -> /17");
        assert!(
            !overlaps_any(&n, &claimed),
            "downgraded subnet must not overlap"
        );
    }

    #[test]
    fn or_smaller_never_returns_larger_than_requested() {
        // Empty space could fit a /16, but a /28 request must stay /28.
        let n = pick_subnet_or_smaller("node-b", &empty(), DEFAULT_MESH_SPACE, 28).unwrap();
        assert_eq!(n.prefix_len(), 28);
        assert!(
            n.prefix_len() >= 28,
            "never larger (smaller prefix number) than requested"
        );
    }

    #[test]
    fn or_smaller_returns_none_when_nothing_fits() {
        // Claim every /24 in the /16: no slot of any size down to /30 is free.
        let mut claimed = HashSet::new();
        let base_octets = DEFAULT_MESH_SPACE.network().octets();
        for idx in 0u8..=255 {
            claimed.insert(
                Ipv4Net::new(Ipv4Addr::new(base_octets[0], base_octets[1], idx, 0), 24).unwrap(),
            );
        }
        assert_eq!(
            pick_subnet_or_smaller("any-node", &claimed, DEFAULT_MESH_SPACE, 24),
            None
        );
    }

    // --- usable_hosts / total_addresses ---

    #[test]
    fn usable_hosts_table() {
        assert_eq!(usable_hosts(16), 65_534);
        assert_eq!(usable_hosts(20), 4_094);
        assert_eq!(usable_hosts(22), 1_022);
        assert_eq!(usable_hosts(23), 510);
        assert_eq!(usable_hosts(24), 254);
        assert_eq!(usable_hosts(28), 14);
        assert_eq!(usable_hosts(30), 2);
        assert_eq!(usable_hosts(31), 2); // RFC 3021 PtP
        assert_eq!(usable_hosts(32), 1); // host route
    }

    #[test]
    fn total_addresses_table() {
        assert_eq!(total_addresses(16), 65_536);
        assert_eq!(total_addresses(24), 256);
        assert_eq!(total_addresses(30), 4);
        assert_eq!(total_addresses(32), 1);
    }

    // --- bump helpers ---

    #[test]
    fn bump_larger_walks_toward_base() {
        assert_eq!(bump_larger_subnet(24, 16), Some(23));
        assert_eq!(bump_larger_subnet(17, 16), Some(16));
        assert_eq!(bump_larger_subnet(16, 16), None); // clamped at base
    }

    #[test]
    fn bump_smaller_walks_toward_floor() {
        assert_eq!(bump_smaller_subnet(24), Some(25));
        assert_eq!(bump_smaller_subnet(29), Some(30));
        assert_eq!(bump_smaller_subnet(30), None); // clamped at /30
    }

    #[test]
    fn bump_round_trip() {
        let prefix = 24u8;
        let up = bump_larger_subnet(prefix, 16).unwrap();
        let back = bump_smaller_subnet(up).unwrap();
        assert_eq!(back, prefix);
    }
}
