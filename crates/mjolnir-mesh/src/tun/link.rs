use std::net::Ipv4Addr;

/// The reserved link-addressing block for per-peer TUN /31s.
/// Devices on the mesh never see these addresses.
pub const LINK_BLOCK: (Ipv4Addr, u8) = (Ipv4Addr::new(10, 255, 0, 0), 16);

/// Prefix length of the shared mesh **backhaul** block (`10.254.0.0/16`).
///
/// Every node self-assigns one address in this block to its shared-segment
/// (container) interface; the host part is derived from the node id, so each
/// node's underlay address is stable and collision-free with zero coordination —
/// no DHCP, no server, works fully offline.
///
/// This is the node-to-node *underlay* that iroh/QUIC and mDNS run over (distinct
/// from the per-peer TUN /31s in [`LINK_BLOCK`], from the CRDT-assigned client
/// /24s, and from babeld's IPv6 link-local *overlay* adjacency on the TUNs).
/// Because every node shares this one `/16`, all peers are on-link to each other
/// on the shared L2 segment — direct neighbours, no routing.
///
/// IPv4 (not an IPv6 ULA) on purpose: iroh 1.0 surfaces private IPv4 addresses as
/// connection candidates and announces them over mDNS, but does *not* surface
/// IPv6 ULA addresses, and binding one directly trips IPv6 DAD. See the
/// `iroh-lan-backhaul-findings` memory for the empirical detail.
pub const BACKHAUL_PREFIX_LEN: u8 = 16;

/// First two octets of the backhaul block `10.254.0.0/16`. Chosen to avoid the
/// TUN-link block (`10.255.0.0/16`, [`LINK_BLOCK`]) and the client mesh space
/// (`10.42.0.0/16`).
const BACKHAUL_BLOCK: [u8; 2] = [10, 254];

/// Derive this node's stable IPv4 backhaul address from its node id.
///
/// Address = `10.254.<h>.<l>`, where `<h>.<l>` is the first 16 bits of
/// `blake3(node_id)`. Deterministic (same node id → same address every boot) and
/// collision-resistant over the 16-bit host space — negligible for any realistic
/// same-site mesh, and the same trade-off [`pick_link_31`] already makes for the
/// TUN /31s. The host part is clamped off the `/16` network (`.0.0`) and
/// broadcast (`.255.255`) addresses.
pub fn backhaul_addr(node_id: &str) -> Ipv4Addr {
    let hash = blake3::hash(node_id.as_bytes());
    let bytes = hash.as_bytes();
    let mut host = u16::from_be_bytes([bytes[0], bytes[1]]);
    if host == 0 {
        host = 1; // avoid the network address 10.254.0.0
    } else if host == u16::MAX {
        host = u16::MAX - 1; // avoid the broadcast address 10.254.255.255
    }
    let [h, l] = host.to_be_bytes();
    Ipv4Addr::new(BACKHAUL_BLOCK[0], BACKHAUL_BLOCK[1], h, l)
}

/// Derive a /31 for a peer-pair, symmetrically.
///
/// Returns `(self_addr, peer_addr)` from the perspective of `self_id`.
/// Calling `pick_link_31(B, A)` on the other side returns the swapped pair, so both
/// endpoints agree on which /31 the link uses without coordination.
///
/// The /31 is selected by hashing the sorted (lower, higher) node-id pair into the
/// `10.255.0.0/16` address space. Collisions across distinct pairs are bounded by
/// the size of the /16 (32,768 distinct /31s) — sufficient for any realistic mesh.
pub fn pick_link_31(self_id: &str, peer_id: &str) -> (Ipv4Addr, Ipv4Addr) {
    // 1. Sort the pair lexicographically (lower, higher).
    let (lower, higher) = if self_id <= peer_id {
        (self_id, peer_id)
    } else {
        (peer_id, self_id)
    };

    // 2. Hash with blake3 over the concatenation (lower || "\0" || higher).
    let mut input = String::with_capacity(lower.len() + 1 + higher.len());
    input.push_str(lower);
    input.push('\0');
    input.push_str(higher);
    let hash = blake3::hash(input.as_bytes());
    let bytes = hash.as_bytes();

    // 3. Take 15 bits → offset within 10.255.0.0/16 chunked into /31s (15-bit space).
    //    Use first two bytes of hash for the 15-bit offset.
    let raw = u16::from_be_bytes([bytes[0], bytes[1]]);
    let offset = raw & 0x7FFF; // 15 bits

    // 4. Lower IP of the /31:
    //    10.255.{(offset >> 7) & 0xff}.{(offset & 0x7f) << 1}
    //    Upper IP = lower + 1.
    let third_octet = ((offset >> 7) & 0xFF) as u8;
    let fourth_octet_lower = ((offset & 0x7F) << 1) as u8;
    let lower_ip = Ipv4Addr::new(10, 255, third_octet, fourth_octet_lower);
    let upper_ip = Ipv4Addr::new(10, 255, third_octet, fourth_octet_lower + 1);

    // 5. Lower node-id gets the lower IP; higher gets the higher IP.
    // 6. From self_id's perspective: if self_id is the lower of the pair, return
    //    (lower_ip, upper_ip); else (upper_ip, lower_ip).
    if self_id <= peer_id {
        (lower_ip, upper_ip)
    } else {
        (upper_ip, lower_ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_same_pair() {
        let first = pick_link_31("alpha", "beta");
        let second = pick_link_31("alpha", "beta");
        assert_eq!(first, second);
    }

    #[test]
    fn symmetric_across_endpoints() {
        let (self_a, peer_a) = pick_link_31("alpha", "beta");
        let (self_b, peer_b) = pick_link_31("beta", "alpha");
        assert_eq!(self_a, peer_b);
        assert_eq!(peer_a, self_b);
        // Both ends agree on the link addresses.
    }

    #[test]
    fn addresses_form_valid_31() {
        let (self_addr, peer_addr) = pick_link_31("node-aabbccdd", "node-eeff0011");

        // Both within 10.255.0.0/16
        let octets_self = self_addr.octets();
        let octets_peer = peer_addr.octets();
        assert_eq!(octets_self[0], 10);
        assert_eq!(octets_self[1], 255);
        assert_eq!(octets_peer[0], 10);
        assert_eq!(octets_peer[1], 255);

        // self_addr and peer_addr differ by exactly 1 in the last octet,
        // with the lower being even (start of /31).
        let self_u32 = u32::from(self_addr);
        let peer_u32 = u32::from(peer_addr);
        assert_eq!(self_u32.abs_diff(peer_u32), 1);

        // The lower address of the pair must be even (bit 0 clear).
        let lower_u32 = self_u32.min(peer_u32);
        assert_eq!(lower_u32 & 1, 0);
    }

    #[test]
    fn backhaul_addr_is_deterministic() {
        let id = "fd7691128f2bb615d56cf2f0e202fa01472890dd8af89f9132d34d566776ed45";
        assert_eq!(backhaul_addr(id), backhaul_addr(id));
    }

    #[test]
    fn backhaul_addr_in_shared_block() {
        // Every node lands in the same 10.254.0.0/16, so all peers are on-link.
        let a = backhaul_addr("alpha");
        let b = backhaul_addr("beta");
        assert_eq!(&a.octets()[..2], &[10, 254]);
        assert_eq!(&b.octets()[..2], &[10, 254]);
        // Distinct node ids → distinct addresses.
        assert_ne!(a, b);
        // Never the /16 network or broadcast address.
        assert_ne!(a, Ipv4Addr::new(10, 254, 0, 0));
        assert_ne!(a, Ipv4Addr::new(10, 254, 255, 255));
    }

    #[test]
    fn backhaul_addrs_distinct_across_many_nodes() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for i in 0u32..100 {
            seen.insert(backhaul_addr(&format!("node-{i:08x}")));
        }
        // 100 nodes in a 16-bit host space: collisions are statistically negligible.
        assert_eq!(seen.len(), 100, "all 100 node backhaul addrs must be unique");
    }

    #[test]
    fn distinct_pairs_get_distinct_links_usually() {
        use std::collections::HashSet;

        let pairs: Vec<(String, String)> = (0u32..100)
            .map(|i| (format!("node-{i:08x}"), format!("node-{:08x}", i + 1000)))
            .collect();

        let mut seen = HashSet::new();
        for (a, b) in &pairs {
            let (self_addr, _) = pick_link_31(a, b);
            // Use the lower IP of the /31 as the key (both ends share the same /31)
            let lower = u32::from(self_addr).min(u32::from({
                let (_, p) = pick_link_31(a, b);
                p
            }));
            seen.insert(lower);
        }

        // >95% should be distinct
        assert!(
            seen.len() >= 95,
            "only {} distinct /31s out of 100 pairs",
            seen.len()
        );
    }
}
