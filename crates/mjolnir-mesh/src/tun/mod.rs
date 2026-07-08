pub mod encap;
pub mod fib;
pub mod iface;
pub mod link;
pub mod mcast;
pub mod overlay;

pub use encap::{DatagramConn, EncapError, EncapHandles, spawn_encap_pair};
pub use fib::Fib;
#[cfg(target_os = "linux")]
pub use iface::spawn_overlay_tun;
pub use iface::{
    IfaceError, OVERLAY_IFACE, OverlayLink, PeerInterface, TUNNEL_MTU, Tunnel, overlay_link_local,
    spawn_tunnel,
};
pub use link::{
    BACKHAUL_PREFIX_LEN, LINK_BLOCK, backhaul_addr, backhaul_addr_salted, in_backhaul_block,
    pick_link_31,
};
pub use mcast::{BABEL_MCAST, BABEL_PORT, OverlayDest, classify, is_babel_multicast};
pub use overlay::{OverlayHandles, UnicastRouter, spawn_overlay, spawn_overlay_routed};
