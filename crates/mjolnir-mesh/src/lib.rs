// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Duke Jones and the Lightning Mesh contributors
// Lightning Mesh is dual-licensed (AGPL-3.0-or-later or commercial); see LICENSE
// and COMMERCIAL-LICENSE.md at the repository root.

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
//! - [`dns_responder`] (daemon feature) — the `.mesh` zone UDP responder
//!   (sprint-002 e21.1.x)
//!
//! See `docs/network-coordination/` in the repo root for the design specs.

pub mod alloc;
pub mod babel;
pub mod bootstrap;
pub mod claim_cooldown;
pub mod crdt;
#[cfg(feature = "daemon")]
pub mod dns_responder;
#[cfg(all(test, feature = "daemon"))]
mod dns_conformance_tests;
pub mod roster;
pub mod tun;

pub use crdt::{
    dns::DnsEntry,
    gossip::GossipMessage,
    hlc::HLC,
    lease::LeaseEntry,
    liveness::{monotonic_now_ms, LivenessTracker},
    merge::{
        merge_peer_addr, merge_service, merge_service_v2, merge_subnet_claim, merge_user,
        resolve_subnet_conflict, MergeResult, ReservedServiceName,
    },
    peer_addr::{AddrBook, PeerAddrEntry},
    service::{
        device_service_key, is_reserved_service_name, node_scope_label, normalize_device_host,
        parse_host_mac, DeviceHostError, LostName, LostNameMap, ServiceBook, ServiceBookV2,
        ServiceEntry, ServiceEntryV2, ServiceTombstone, ServiceTombstoneBook,
        RESERVED_SERVICE_NAMES,
    },
    service_apply::{
        apply_service_publish_v2, apply_service_publish_v2_tracking_loss,
        apply_service_unpublish_v2, publish_service_v2, PublishOutcome, ServicePublishError,
        UnpublishOutcome,
    },
    subnet::SubnetClaim,
    sync::{GossipError, GossipSync, GossipTransport},
    users::{UserBook, UserEntry},
};
pub use roster::{PeerEntry, PeerRoster, RosterError};
