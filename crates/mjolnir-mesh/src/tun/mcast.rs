//! Multicast classification for the single overlay TUN (mjolnir-mesh-buw.1 spike).
//!
//! babeld discovers neighbours by sending Hello/IHU packets to the link-local
//! multicast group `ff02::1:6` (UDP 6696, RFC 8966 §4). On the per-peer /31 TUNs
//! the current design uses, that works because each TUN is a point-to-point link
//! with exactly one neighbour. The `buw` fork collapses those into ONE overlay
//! TUN (`mjolnir0`) carrying many neighbours — but a TUN has no L2, so a packet
//! sent to `ff02::1:6` on `mjolnir0` reaches nobody: there is no multicast
//! fan-out in the kernel for a point-to-multipoint TUN.
//!
//! The fix is to EMULATE multicast in the daemon: when a packet read off the
//! overlay TUN is destined for a link-local multicast group, replicate it to
//! every peer connection; unicast packets are instead routed to the single peer
//! that owns the destination address (LPM — see mjolnir-mesh-buw.4). This module
//! provides the classification the encap layer keys that decision on. It is pure
//! and cross-platform so it unit-tests without a TUN or root.

use std::net::Ipv6Addr;

/// The Babel protocol's link-local multicast group (RFC 8966 §4): `ff02::1:6`.
pub const BABEL_MCAST: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 1, 6);

/// Babel's well-known UDP port (RFC 8966): 6696.
pub const BABEL_PORT: u16 = 6696;

/// How the overlay encap layer must forward a packet read off `mjolnir0`,
/// decided by its destination address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayDest {
    /// Multicast/broadcast (e.g. a babel Hello to `ff02::1:6`). The daemon must
    /// REPLICATE the datagram to every peer connection — this is the multicast
    /// emulation that lets babel neighbour discovery work over a single TUN.
    Multicast,
    /// Ordinary unicast. buw.4 routes these by `LPM(dest)->peer`; during the
    /// buw.1 spike (a single peer) they are simply flooded like multicast, which
    /// is safe because a peer whose interface does not own the destination
    /// address drops the packet in the kernel.
    Unicast,
}

/// Classify an IP packet (as read off the overlay TUN, i.e. starting at the IP
/// header) by its destination address. Returns `None` if the buffer is too
/// short to contain the relevant header fields or the IP version is unknown.
///
/// Recognises IPv6 multicast (`ff00::/8`) and IPv4 multicast (`224.0.0.0/4`) /
/// limited broadcast (`255.255.255.255`); everything else is unicast.
pub fn classify(pkt: &[u8]) -> Option<OverlayDest> {
    match pkt.first()? >> 4 {
        6 => {
            // IPv6 fixed header: destination address is octets 24..40.
            let dst = pkt.get(24..40)?;
            // ff00::/8 is the entire IPv6 multicast space.
            if dst[0] == 0xff {
                Some(OverlayDest::Multicast)
            } else {
                Some(OverlayDest::Unicast)
            }
        }
        4 => {
            // IPv4 header: destination address is octets 16..20.
            let dst = pkt.get(16..20)?;
            let is_mcast = (dst[0] & 0xf0) == 0xe0; // 224.0.0.0/4
            let is_bcast = dst == [255, 255, 255, 255];
            if is_mcast || is_bcast {
                Some(OverlayDest::Multicast)
            } else {
                Some(OverlayDest::Unicast)
            }
        }
        _ => None,
    }
}

/// True iff `pkt` is IPv6 traffic destined for the Babel multicast group
/// `ff02::1:6` — i.e. a babel Hello/IHU that must be flooded to all peers for
/// neighbour discovery to bootstrap over the overlay TUN.
pub fn is_babel_multicast(pkt: &[u8]) -> bool {
    if pkt.first().map(|b| b >> 4) != Some(6) {
        return false;
    }
    match pkt.get(24..40).and_then(|s| <[u8; 16]>::try_from(s).ok()) {
        Some(dst) => Ipv6Addr::from(dst) == BABEL_MCAST,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    /// Build a minimal IPv6 packet header (40 bytes) with the given dest address.
    fn ipv6_to(dst: Ipv6Addr) -> Vec<u8> {
        let mut pkt = vec![0u8; 40];
        pkt[0] = 0x60; // version 6
        pkt[24..40].copy_from_slice(&dst.octets());
        pkt
    }

    /// Build a minimal IPv4 packet header (20 bytes) with the given dest address.
    fn ipv4_to(dst: Ipv4Addr) -> Vec<u8> {
        let mut pkt = vec![0u8; 20];
        pkt[0] = 0x45; // version 4, IHL 5
        pkt[16..20].copy_from_slice(&dst.octets());
        pkt
    }

    #[test]
    fn babel_hello_is_multicast() {
        let pkt = ipv6_to(BABEL_MCAST);
        assert_eq!(classify(&pkt), Some(OverlayDest::Multicast));
        assert!(is_babel_multicast(&pkt));
    }

    #[test]
    fn ipv6_linklocal_unicast_is_unicast() {
        // A peer's fe80:: link-local (where babel unicasts IHUs) is NOT multicast.
        let ll: Ipv6Addr = "fe80::42".parse().unwrap();
        let pkt = ipv6_to(ll);
        assert_eq!(classify(&pkt), Some(OverlayDest::Unicast));
        assert!(!is_babel_multicast(&pkt));
    }

    #[test]
    fn ipv6_all_nodes_multicast_recognised_but_not_babel() {
        let pkt = ipv6_to("ff02::1".parse().unwrap());
        assert_eq!(classify(&pkt), Some(OverlayDest::Multicast));
        // Multicast, but not the babel group specifically.
        assert!(!is_babel_multicast(&pkt));
    }

    #[test]
    fn ipv4_unicast_and_multicast_and_broadcast() {
        assert_eq!(
            classify(&ipv4_to(Ipv4Addr::new(10, 254, 1, 2))),
            Some(OverlayDest::Unicast)
        );
        assert_eq!(
            classify(&ipv4_to(Ipv4Addr::new(224, 0, 0, 251))), // mDNS
            Some(OverlayDest::Multicast)
        );
        assert_eq!(
            classify(&ipv4_to(Ipv4Addr::new(255, 255, 255, 255))),
            Some(OverlayDest::Multicast)
        );
    }

    #[test]
    fn short_or_unknown_is_none() {
        assert_eq!(classify(&[]), None);
        assert_eq!(classify(&[0x60]), None); // v6 claimed but no dest
        assert_eq!(classify(&[0x45, 0, 0]), None); // v4 claimed but no dest
        assert_eq!(classify(&[0x00; 40]), None); // version 0 — unknown
        assert!(!is_babel_multicast(&[0x60])); // too short, not a panic
    }
}
