//! mjolnir-mesh — networking substrate for the mjolnir router mesh.
//!
//! This crate provides the coordination primitives used by routers running
//! mjolnir-mesh on OpenWrt: a CRDT data model for shared mesh state (leases,
//! DNS, services, subnet claims), per-peer TUN tunnel interfaces over Iroh
//! QUIC, and a `babeld` config layer for cross-site routing.
//!
//! Modules:
//! - [`crdt`] — shared-state types and FWW merge
//! - [`alloc`] / [`claim_cooldown`] — subnet claim allocation on first boot
//! - [`tun`] — per-peer TUN lifecycle and Iroh-datagram encap/decap loops
//! - [`babel`] — babeld config rendering (babeld's process lifecycle is owned by
//!   procd via the `mjolnir-babeld` service, not this crate — mjolnir-mesh-m8t)
//!
//! See `docs/network-coordination/` in the repo root for the design specs.

pub mod alloc;
pub mod babel;
pub mod claim_cooldown;
pub mod crdt;
pub mod roster;
pub mod tun;

pub use crdt::{
    dns::DnsEntry,
    gossip::GossipMessage,
    hlc::HLC,
    lease::LeaseEntry,
    merge::{merge_peer_addr, merge_subnet_claim, merge_user, resolve_subnet_conflict, MergeResult},
    peer_addr::{AddrBook, PeerAddrEntry},
    service::ServiceEntry,
    subnet::SubnetClaim,
    sync::{GossipError, GossipSync, GossipTransport},
    users::{UserBook, UserEntry},
};
pub use roster::{PeerEntry, PeerRoster, RosterError};
