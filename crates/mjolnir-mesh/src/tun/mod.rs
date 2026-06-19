pub mod encap;
pub mod iface;
pub mod link;

pub use encap::{spawn_encap_pair, DatagramConn, EncapError, EncapHandles};
pub use iface::{spawn_tunnel, IfaceError, PeerInterface, Tunnel, TUNNEL_MTU};
pub use link::{pick_link_31, LINK_BLOCK};
