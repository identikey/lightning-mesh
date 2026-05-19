pub mod alloc;
pub mod babel;
pub mod claim_cooldown;
pub mod crdt;
pub mod tun;

pub use crdt::{
    dns::DnsEntry,
    gossip::GossipMessage,
    hlc::HLC,
    lease::LeaseEntry,
    merge::{merge_subnet_claim, resolve_subnet_conflict, MergeResult},
    service::ServiceEntry,
    subnet::SubnetClaim,
};
