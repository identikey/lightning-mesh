//! Forwarding table for the single overlay TUN (mjolnir-mesh-buw.4).
//!
//! Maps an IPv4 destination to the next-hop peer's overlay address
//! (`10.254.x`) by longest-prefix match. The daemon mirrors babeld's kernel
//! routes on `mjolnir0` into this table (from rtnetlink `RTM_NEWROUTE` /
//! `RTM_DELROUTE`), so the overlay data plane can demux each outbound packet to
//! the right iroh connection:
//!
//! ```text
//! dest IP  --lookup-->  next-hop 10.254.x  --conn map-->  iroh Connection
//! ```
//!
//! Pure and cross-platform so it unit-tests without rtnetlink or a TUN. At mesh
//! scale (a few hundred `/24`s) the linear longest-match scan is trivially fast;
//! a trie is a later optimization the callers don't need to know about.

use std::collections::HashMap;
use std::net::Ipv4Addr;

/// An IPv4 prefix `net/len`. `net` is always stored masked to `len`, so equal
/// prefixes compare and hash equal regardless of the address they were built
/// from (e.g. `10.42.1.7/24` and `10.42.1.0/24` normalize to the same key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Prefix {
    net: Ipv4Addr,
    len: u8,
}

/// Mask for the top `len` bits (`len` clamped to 0..=32). `len == 0` is the
/// default route and masks to `0.0.0.0`.
fn mask_of(len: u8) -> u32 {
    match len.min(32) {
        0 => 0,
        n => u32::MAX << (32 - n),
    }
}

impl Prefix {
    /// Build a prefix, normalizing `addr` to the network address for `len`.
    pub fn new(addr: Ipv4Addr, len: u8) -> Self {
        let len = len.min(32);
        Self {
            net: Ipv4Addr::from(u32::from(addr) & mask_of(len)),
            len,
        }
    }

    /// The network address (masked).
    pub fn net(&self) -> Ipv4Addr {
        self.net
    }

    /// True if `addr` falls within this prefix.
    fn contains(&self, addr: Ipv4Addr) -> bool {
        (u32::from(addr) & mask_of(self.len)) == u32::from(self.net)
    }
}

/// Longest-prefix-match forwarding table: dest prefix -> next-hop overlay addr.
#[derive(Debug, Default, Clone)]
pub struct Fib {
    routes: HashMap<Prefix, Ipv4Addr>,
}

impl Fib {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace the next hop for `addr/len`.
    pub fn upsert(&mut self, addr: Ipv4Addr, len: u8, next_hop: Ipv4Addr) {
        self.routes.insert(Prefix::new(addr, len), next_hop);
    }

    /// Remove `addr/len`. No-op if absent.
    pub fn remove(&mut self, addr: Ipv4Addr, len: u8) {
        self.routes.remove(&Prefix::new(addr, len));
    }

    /// The next hop for `dest` by longest-prefix match, or `None` if no prefix
    /// covers it. A `/32` beats a `/24` beats a `/16` beats the `/0` default.
    pub fn lookup(&self, dest: Ipv4Addr) -> Option<Ipv4Addr> {
        self.routes
            .iter()
            .filter(|(p, _)| p.contains(dest))
            .max_by_key(|(p, _)| p.len)
            .map(|(_, next_hop)| *next_hop)
    }

    /// Number of installed prefixes.
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn prefix_normalizes_host_bits() {
        // Same /24 regardless of host bits -> same key.
        assert_eq!(
            Prefix::new(ip("10.42.1.7"), 24),
            Prefix::new(ip("10.42.1.0"), 24)
        );
        assert_eq!(Prefix::new(ip("10.42.1.7"), 24).net(), ip("10.42.1.0"));
    }

    #[test]
    fn empty_fib_misses() {
        let fib = Fib::new();
        assert_eq!(fib.lookup(ip("10.42.1.5")), None);
        assert!(fib.is_empty());
    }

    #[test]
    fn most_specific_wins_24_beats_16() {
        let mut fib = Fib::new();
        fib.upsert(ip("10.42.0.0"), 16, ip("10.254.0.1"));
        fib.upsert(ip("10.42.1.0"), 24, ip("10.254.0.2"));
        // Inside the /24 -> the /24's next hop.
        assert_eq!(fib.lookup(ip("10.42.1.99")), Some(ip("10.254.0.2")));
        // Inside the /16 but outside the /24 -> the /16's next hop.
        assert_eq!(fib.lookup(ip("10.42.2.99")), Some(ip("10.254.0.1")));
    }

    #[test]
    fn host_route_32_beats_24() {
        let mut fib = Fib::new();
        fib.upsert(ip("10.42.1.0"), 24, ip("10.254.0.2"));
        fib.upsert(ip("10.42.1.7"), 32, ip("10.254.0.9"));
        assert_eq!(fib.lookup(ip("10.42.1.7")), Some(ip("10.254.0.9")));
        assert_eq!(fib.lookup(ip("10.42.1.8")), Some(ip("10.254.0.2")));
    }

    #[test]
    fn default_route_catches_everything_else() {
        let mut fib = Fib::new();
        fib.upsert(ip("0.0.0.0"), 0, ip("10.254.0.1"));
        fib.upsert(ip("10.42.1.0"), 24, ip("10.254.0.2"));
        assert_eq!(fib.lookup(ip("10.42.1.5")), Some(ip("10.254.0.2"))); // specific
        assert_eq!(fib.lookup(ip("8.8.8.8")), Some(ip("10.254.0.1"))); // default
    }

    #[test]
    fn upsert_replaces_and_remove_works() {
        let mut fib = Fib::new();
        fib.upsert(ip("10.42.1.0"), 24, ip("10.254.0.2"));
        assert_eq!(fib.len(), 1);
        // Upsert same prefix (different host bits) replaces the next hop.
        fib.upsert(ip("10.42.1.200"), 24, ip("10.254.0.5"));
        assert_eq!(fib.len(), 1);
        assert_eq!(fib.lookup(ip("10.42.1.5")), Some(ip("10.254.0.5")));
        // Remove.
        fib.remove(ip("10.42.1.0"), 24);
        assert_eq!(fib.lookup(ip("10.42.1.5")), None);
        assert!(fib.is_empty());
        // Removing an absent prefix is a no-op.
        fib.remove(ip("10.42.9.0"), 24);
    }
}
