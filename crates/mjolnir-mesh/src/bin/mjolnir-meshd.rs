//! `mjolnir-meshd` — headless iroh-transport router daemon (P0: connectivity MVP).
//!
//! Phase 0 proves the core value prop on real hardware: a persistent iroh
//! identity plus QUIC connectivity (with NAT traversal via relays) between two
//! nodes. There is deliberately **no TUN** yet — that is P1 — so this binary
//! can validate iroh-in-a-RouterOS-container *before* the unverified
//! TUN-in-container question. See beads mjolnir-mesh-tr6 / mjolnir-mesh-02g.
//!
//! Subcommands:
//!   id                 print this node's EndpointId and a shareable address blob
//!   listen             accept inbound connections, echo ping datagrams
//!   connect <addr>     dial a peer by address blob, measure a datagram round-trip

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bytes::Bytes;
use clap::{Parser, Subcommand};
use ipnet::{IpNet, Ipv4Net};
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::presets;
use iroh::endpoint::Connection;
use iroh_mdns_address_lookup::MdnsAddressLookup;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey};
use iroh_gossip::api::{Event, GossipReceiver, GossipSender};
use iroh_gossip::{Gossip, TopicId};
use mjolnir_mesh::tun::{
    classify, spawn_overlay_tun, spawn_tunnel, DatagramConn, EncapError, Fib, OverlayDest, Tunnel,
    UnicastRouter, OVERLAY_IFACE, TUNNEL_MTU,
};
use mjolnir_mesh::babel::{
    render_babeld_conf, render_overlay_babeld_conf, write_atomic_if_changed, BabelConfigInputs,
    OverlayRtt,
};
use mjolnir_mesh::{
    alloc, apply_service_publish_v2_tracking_loss, apply_service_unpublish_v2, merge_peer_addr,
    merge_service, merge_subnet_claim, merge_user, publish_service_v2, AddrBook, GossipError,
    GossipSync, GossipTransport, LostNameMap, MergeResult, PeerAddrEntry, PeerEntry, PeerRoster,
    PublishOutcome, ServiceBook, ServiceBookV2, ServiceEntry, ServiceEntryV2, ServicePublishError,
    ServiceTombstone, ServiceTombstoneBook, SubnetClaim, UnpublishOutcome, UserBook, UserEntry,
    HLC,
};
use mjolnir_mesh::GossipMessage;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

/// ALPN for the P0 mesh connectivity probe. Bumped per protocol revision.
const MESH_ALPN: &[u8] = b"mjolnir/mesh/v0";

/// ALPN for the P1 L3 tunnel (TUN packets over iroh datagrams).
const TUN_ALPN: &[u8] = b"mjolnir/mesh/tun/v0";

/// UDP port the tunnel reachability probe echoes on (bound to the TUN /31 addr).
const TUN_PROBE_PORT: u16 = 9999;

/// Datagram payload used to prove an end-to-end round-trip.
const PING: &[u8] = b"mjolnir-ping";

/// Connection-wide QUIC idle timeout for tunnel endpoints (mjolnir-mesh-auu).
///
/// iroh's default connection idle timeout is ~30s, which is what killed the
/// direct tunnel ~36s after the DIRECT path was selected: once iroh fails to
/// prune the redundant candidate path (`MultipathNotNegotiated`), the selected
/// path stops carrying traffic and the connection idles out. Raising the
/// ceiling to 60s gives iroh's holepunch/path-recovery a second window to
/// re-select a live path before the connection is declared dead. It is NOT a
/// root-cause fix (if multipath never negotiates, death is merely deferred to
/// ~66s) — but that deferral is itself a clean discriminator on hardware:
/// death tracking this value confirms the idle-timeout/prune hypothesis.
/// Per-path idle is separately clamped to 15s by iroh; this is the connection
/// envelope, held open by the 5s keep-alive while any path lives.
const TUNNEL_MAX_IDLE: Duration = Duration::from_secs(60);

#[derive(Parser)]
#[command(
    name = "mjolnir-meshd",
    about = "Headless iroh-transport mesh daemon (P0 connectivity)"
)]
struct Cli {
    /// Path to the persisted node secret key (hex). Generated on first run if
    /// absent. If omitted, falls back to the IROH_SECRET env var, then to an
    /// ephemeral key (logged as a warning — identity won't survive restart).
    #[arg(long, global = true)]
    secret_file: Option<PathBuf>,

    /// Disable iroh relays (direct/LAN only). Useful for offline/LAN meshes and
    /// for same-host testing without depending on public relay servers.
    #[arg(long, global = true)]
    no_relay: bool,

    /// Bind to a specific socket address (e.g. 127.0.0.1:0 for a loopback-only
    /// test). Default is iroh's wildcard bind.
    #[arg(long, global = true)]
    bind: Option<SocketAddr>,

    /// LAN-direct mode: discover peers via mDNS on the local network, no relay,
    /// no pkarr/DNS, no internet. Connect by bare node id; addresses are found
    /// over the LAN. Implies --no-relay. For same-switch swarms.
    #[arg(long, global = true)]
    lan: bool,

    /// Relay server URL(s) to use (repeatable), e.g. a self-hosted relay. If
    /// omitted, uses n0's staging relays. NOTE: iroh 0.96's "Default" points at
    /// the flaky canary network, so we never use it. Implies internet mode.
    #[arg(long, global = true)]
    relay: Vec<String>,

    /// Opt into internet mode (n0 relays + pkarr/DNS discovery) for the `mesh`
    /// daemon. By default `mesh` runs in `--lan` mode (offline, mDNS, no relay),
    /// since the deployed same-site mesh has no internet. Use this only when the
    /// mesh must span the internet across separate sites.
    #[arg(long, global = true)]
    internet: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print this node's EndpointId and a shareable address blob.
    Id,
    /// One-shot ground-truth diagnostic: node identity + build stamp, the
    /// derived backhaul address, every interface's IPv4 addresses, and the
    /// installed kernel routes in the mesh space — read straight from the
    /// system, no running daemon required. The fast way to answer "is the
    /// backhaul addr assigned, is the interface dual-addressed, did babel
    /// install routes and via what next-hop" without grepping logs.
    Status,
    /// Listen for inbound mesh connections and echo ping datagrams. Runs until Ctrl-C.
    Listen,
    /// Dial a peer (address blob from `id`/`listen`) and measure a round-trip.
    Connect {
        /// Address blob printed by the peer's `id` or `listen`.
        addr: String,
    },
    /// Probe whether a TUN device can be created in this environment (e.g.
    /// inside a RouterOS container). Creates a throwaway /31 link and tears it
    /// down. This is the gating check for the L3 data plane (P1).
    TunTest,
    /// P1: listen for a peer and bring up a per-peer /31 TUN tunnel over iroh.
    /// Runs until Ctrl-C; echoes UDP probes on its tunnel address.
    TunListen,
    /// P1: dial a peer (address blob), bring up the /31 TUN tunnel, and probe
    /// reachability across it (UDP round-trip to the peer's link address).
    TunConnect {
        /// Address blob printed by the peer's `tun-listen`.
        addr: String,
    },
    /// P2: run the full multi-peer mesh. Reads a roster of peers, accepts inbound
    /// tunnels, and dials every peer for which this node is the initiator (lower
    /// id), maintaining one /31 TUN per peer with redial-on-drop. Runs until
    /// Ctrl-C. This is the daemon mode router deploys use.
    Mesh {
        /// Path to the peer roster file: one peer address blob or 64-hex node id
        /// per line; `#` comments and blank lines ignored. See `PeerRoster`.
        /// Optional — peers may instead (or also) be given via `--peer`.
        #[arg(long)]
        roster: Option<PathBuf>,
        /// A peer address blob or 64-hex node id to mesh with (repeatable).
        /// Merged with any `--roster` entries; avoids needing a file in a
        /// scratch container. e.g. `--peer <id> --peer <id>`.
        #[arg(long)]
        peer: Vec<String>,
        /// Where to write the generated babeld config. Its parent dir is created
        /// if missing. babeld is started once there's a live tunnel to route over.
        #[arg(long, default_value = "/etc/mjolnir/babeld.conf")]
        babel_config: PathBuf,
        /// The local client-facing interface (bridge) serving this node's devices.
        /// On claiming a /24, meshd assigns `<net>.1/24` here as a connected route so
        /// babeld redistributes a real route and inbound mesh traffic for the /24 is
        /// delivered on-link (mjolnir-mesh-e4r). Native OpenWrt has no container/veth
        /// gateway — the router sits directly on the client L2.
        #[arg(long, default_value = "br-lan")]
        client_iface: String,
        /// The container interface on the shared L2 segment (the veth facing the
        /// other mesh nodes). meshd self-assigns this node's derived IPv4 backhaul
        /// address here so peers discover + connect directly over the LAN, no DHCP.
        #[arg(long, default_value = "eth0")]
        backhaul_iface: String,
        /// Re-enable per-peer iroh data-plane tunnels in LAN mode (default off —
        /// LAN data rides the shared-L2 backhaul, babel-routed). Opt-in for the
        /// mjolnir-mesh-auu retest: native OpenWrt has no duplicate-IP container
        /// artifact and iroh is pinned to one derived addr+port, so the
        /// MultipathNotNegotiated churn should be gone — flip this on to verify a
        /// same-site tunnel stays up. Dials peers by derived address (0yb.1).
        #[arg(long)]
        lan_tunnels: bool,
        /// Where to persist the subnet-claim CRDT store (postcard-encoded). Loaded
        /// on startup so a rebooting node serves DHCP/DNS immediately without
        /// waiting to relearn claims over gossip, and rewritten on every
        /// anti-entropy cycle (mjolnir-mesh-s9v). Its parent dir is created if
        /// missing.
        #[arg(long, default_value = "/etc/mjolnir/claims.state")]
        claims_file: PathBuf,
        /// Where to write the read-only `directory.json` projection (bead avs):
        /// a snapshot of this node's identity, neighbors (AddrBook + subnet
        /// claims), identities (/users), and services, for `mjolnir-hello` to
        /// read directly — it does NOT re-derive state. Rewritten on the
        /// anti-entropy cadence via tmp+rename (same discipline as
        /// `claims_file`). Default MUST match `mjolnir-hello --directory-file`'s
        /// default and the deploy UCI config — do not diverge.
        #[arg(long, default_value = "/var/run/mjolnir/directory.json")]
        directory_file: PathBuf,
        /// Identity-submission spool dir (p6u): `mjolnir-hello` writes one
        /// `{pubkey}.json` file per accepted (Ed25519-verified) identity
        /// submission here; meshd sweeps it on the anti-entropy cadence, turns
        /// each into a `/users` record, gossips it mesh-wide, and deletes the
        /// file. Default MUST match `mjolnir-hello --spool-dir`'s default and
        /// the deploy UCI config — do not diverge.
        #[arg(long, default_value = "/var/run/mjolnir/pending")]
        spool_dir: PathBuf,
        /// buw single-overlay-TUN data plane (mjolnir-mesh-buw): bring up ONE
        /// `mjolnir0` multiplexing every peer, so babeld sees one static
        /// interface instead of N churning per-peer tunnels. Off by default —
        /// the deployed path is per-peer tunnels; this is opt-in until validated
        /// on the fleet.
        #[arg(long)]
        overlay: bool,
        /// Internet gateway role (mjolnir-mesh-a8o): redistribute this node's
        /// WAN default route into the mesh (`0.0.0.0/0`, metric 128) so other
        /// nodes — and their clients — egress through it. Announcement follows
        /// the kernel FIB: uplink lost => route withdrawn, nothing to unstick.
        /// NAT needs no extra config: the mesh sits in the `lan` firewall zone,
        /// and OpenWrt's `wan` zone already masquerades lan->wan forwards.
        #[arg(long)]
        gateway: bool,
    },
}

/// Well-known UDP port every mesh node binds its iroh socket to in LAN mode, so
/// a peer is reachable at a *fully derived* address — `backhaul_addr(node_id)` +
/// this port — with no mDNS/discovery lookup (mjolnir-mesh-0yb.1). The pinned IP
/// is unique per node, so the shared port is fine even with several nodes on one
/// host. If the port is already taken, `build_endpoint` falls back to an
/// ephemeral bind (losing derivability but staying up).
const MESH_IROH_PORT: u16 = 49737;

#[tokio::main]
async fn main() -> Result<()> {
    // Colorize only when stderr is an interactive terminal. Under procd the logs
    // go to syslog/logread, where the tracing-subscriber ANSI escapes are literal
    // bytes that sit between a field name and its `=` — silently breaking naive
    // `grep 'cidr='` fleet checks (a pt9 convergence check false-negatived to 0
    // this way, mjolnir-mesh-3xb). Interactive runs keep colors.
    use std::io::IsTerminal;
    tracing_subscriber::fmt()
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Log panics via tracing. A task panic otherwise only reaches stderr (easily
    // lost) yet can silently poison a std::Mutex — cascading every
    // `.lock().expect("poisoned")` into more silent panics until the whole runtime
    // parks with no live tasks. That is the exact "hang" signature under
    // lan_tunnels=1 (mjolnir-mesh-qz9); make the origin (file:line + message)
    // visible in logread.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(target: "mjolnir_meshd", "PANIC: {info}");
        default_hook(info);
    }));

    // Build-identity banner. mjolnir-mesh-auu traced a deterministic ~36s tunnel
    // death to iroh's `MultipathNotNegotiated` — which, on two nodes running the
    // SAME 1.0.0 binary, should not happen (multipath is on by default). The
    // cheapest explanation is a binary/version skew between the two routers, so
    // log our identity loudly: compare this line across nodes before suspecting
    // an iroh bug. (Pair with a binary sha256 check at deploy time.)
    info!(
        version = env!("CARGO_PKG_VERSION"),
        build = env!("MJOLNIR_BUILD"),
        idle_timeout_secs = TUNNEL_MAX_IDLE.as_secs(),
        "mjolnir-meshd starting",
    );

    let cli = Cli::parse();

    // tun-test needs no iroh endpoint — handle it before binding one.
    if let Command::TunTest = cli.command {
        return run_tun_test().await;
    }

    // status is a read-only system inspection — no endpoint, no daemon needed.
    if let Command::Status = cli.command {
        return run_status(cli.secret_file.as_deref()).await;
    }

    // The deployed `mesh` daemon defaults to LAN mode (offline, mDNS, no relay),
    // since the same-site mesh has no internet. Opt into internet/relay mode with
    // `--internet` or by passing `--relay`. The lower-level test commands
    // (listen/connect/id) keep their explicit-`--lan` behaviour unchanged.
    let mesh_mode = matches!(cli.command, Command::Mesh { .. });
    // In overlay mode the node's backhaul address (10.254.x) belongs to the
    // single overlay TUN mjolnir0, NOT the underlay iface — so we skip assigning
    // it to the backhaul iface and skip pinning the iroh socket to it (the iroh
    // underlay uses its own transport / relays). Assigning it to both would
    // collide (mjolnir0 vs backhaul iface with the same address).
    let overlay_mode = matches!(cli.command, Command::Mesh { overlay: true, .. });
    let internet = cli.internet || !cli.relay.is_empty();
    let lan = cli.lan || (mesh_mode && !internet);
    // --lan (and LAN-by-default) imply no relay (LAN discovery only).
    let no_relay = cli.no_relay || lan;

    // Load the node secret once so we know our id before binding. For `mesh`, we
    // self-assign the derived IPv4 backhaul address to the shared-segment iface
    // BEFORE building the endpoint, so iroh enumerates it at bind time and mDNS
    // announces it to peers (mjolnir-mesh-4pk). Assigning after bind misses the
    // initial address scan — and with no DHCP the iface has no other address.
    let secret = load_or_create_secret(cli.secret_file.as_deref())?;
    // pt9: load the persisted claim map BEFORE deriving the backhaul address, so
    // a node that previously lost a backhaul-address collision derives around
    // the winner's persisted claim instead of re-colliding at every boot. The
    // map is handed onward as the claim store seed (was loaded later, s9v).
    let restored_claims = match &cli.command {
        Command::Mesh { claims_file, .. } => load_claims(claims_file),
        _ => HashMap::new(),
    };
    let backhaul_ip = pick_backhaul_addr(&restored_claims, &secret.public().to_string());
    let l2_backhaul = match &cli.command {
        // Overlay mode: mjolnir0 carries the backhaul address, so don't put it on
        // the underlay iface too. (Overlay also ignores the l2 wired backhaul.)
        Command::Mesh { backhaul_iface, .. } if !overlay_mode => {
            assign_backhaul_addr(backhaul_iface, backhaul_ip).await
        }
        _ => None,
    };
    // Pin the iroh socket to the derived backhaul address in LAN/mesh mode so
    // peers can dial us at a fully-derived address with no discovery lookup
    // (mjolnir-mesh-0yb.1). NOTE: the `MultipathNotNegotiated` tunnel death this
    // was once thought to prevent was NOT an iroh multipath bug — it was an
    // L23/RouterOS-container artifact (a duplicate `172.20.0.2` on the shared L2,
    // identical on every node, advertised as a bogus second candidate), proven by
    // the auu native retest (OpenWrt: single DIRECT path, stable). So the pin is
    // retained only for deterministic dialing, not to avoid multipath — iroh
    // handles multiple candidates fine on clean networks. Explicit `--bind` still
    // wins; overlay + non-mesh/non-LAN paths are unchanged.
    let bind = match cli.bind {
        Some(addr) => Some(addr),
        // Overlay mode binds the underlay normally (mjolnir0 owns 10.254.x); only
        // the per-peer LAN path pins iroh to the derived backhaul address.
        None if lan && mesh_mode && !overlay_mode => {
            // Pin a well-known port so peers can dial us at a fully-derived
            // address (backhaul_addr + MESH_IROH_PORT), no mDNS needed (0yb.1).
            // `backhaul_ip` is claim-aware (pt9): a collision loser binds its
            // re-derived address, and peers learn it from the gossiped claim.
            Some(SocketAddr::new(std::net::IpAddr::V4(backhaul_ip), MESH_IROH_PORT))
        }
        None => None,
    };
    let endpoint = build_endpoint(secret, no_relay, bind, lan, &cli.relay).await?;

    match cli.command {
        Command::Id => {
            wait_until_addressable(&endpoint, no_relay).await;
            print_identity(&endpoint)?;
        }
        Command::Listen => run_listen(endpoint, no_relay).await?,
        Command::Connect { addr } => run_connect(endpoint, &addr).await?,
        Command::TunListen => run_tun_listen(endpoint, no_relay).await?,
        Command::TunConnect { addr } => run_tun_connect(endpoint, &addr).await?,
        Command::Mesh {
            roster,
            peer,
            babel_config,
            client_iface,
            // backhaul_iface was used before bind in `main`; the resolved name
            // flows in via `l2_backhaul`.
            backhaul_iface: _,
            lan_tunnels,
            claims_file,
            directory_file,
            spool_dir,
            overlay,
            gateway,
        } => {
            // In LAN mode babel routes over the shared-L2 backhaul directly; pass
            // the resolved interface so the reconciler can add it as the wireless L2 iface
            // and skip the per-peer iroh tunnels (mjolnir-mesh-auu).
            let l2 = if lan { l2_backhaul } else { None };
            run_mesh(
                endpoint,
                no_relay,
                roster,
                peer,
                babel_config,
                client_iface,
                lan,
                lan_tunnels,
                l2,
                claims_file,
                directory_file,
                spool_dir,
                overlay,
                gateway,
                backhaul_ip,
                restored_claims,
            )
            .await?
        }
        Command::TunTest | Command::Status => unreachable!("handled above"),
    }
    Ok(())
}

/// A production [`DatagramConn`] over an iroh connection — the glue that lets the
/// substrate's TUN encap loops shuttle IP packets over iroh QUIC datagrams.
#[derive(Clone)]
struct IrohDatagramConn {
    conn: Connection,
}

#[async_trait::async_trait]
impl DatagramConn for IrohDatagramConn {
    async fn send_datagram(&self, packet: Bytes) -> Result<(), EncapError> {
        // Fire-and-forget into iroh's bounded outgoing datagram buffer. When that
        // buffer is full (e.g. a transient congestion/cwnd dip), iroh drops the
        // OLDEST datagram and returns immediately. For an L3 packet tunnel that is
        // the right policy: bounded latency with occasional loss, which TCP (and
        // friends) recover from — far better than backpressuring the single TUN
        // reader, which would head-of-line-block every other flow and balloon
        // latency (bufferbloat). In-flight loss on a relay-only path is not a
        // buffer problem and is not fixable here; a direct path is the lever.
        let len = packet.len();
        self.conn.send_datagram(packet).map_err(|e| {
            use iroh::endpoint::SendDatagramError;
            match e {
                SendDatagramError::TooLarge => EncapError::DatagramTooLarge(len),
                other => EncapError::Io(std::io::Error::other(other.to_string())),
            }
        })
    }

    async fn recv_datagram(&self) -> Result<Bytes, EncapError> {
        // Any read error means the connection is no longer usable; surface it as
        // ConnectionClosed so the encap loop exits cleanly.
        self.conn
            .read_datagram()
            .await
            .map_err(|_| EncapError::ConnectionClosed)
    }
}

/// A production [`GossipTransport`] over iroh-gossip — the daemon-side concrete
/// impl of the substrate's iroh-free gossip seam (the gossip analogue of
/// [`IrohDatagramConn`]). Wraps a topic's sender/receiver halves; the receiver
/// needs `&mut` to poll, so it lives behind an async mutex (only the single
/// dispatch loop ever reads it). Neighbor up/down events feed the watch
/// channel that gates claiming and drives the rejoin loop (mjolnir-mesh-eon).
struct IrohGossipTransport {
    sender: GossipSender,
    receiver: tokio::sync::Mutex<GossipReceiver>,
    neighbors_tx: tokio::sync::watch::Sender<usize>,
}

#[async_trait::async_trait]
impl GossipTransport for IrohGossipTransport {
    async fn broadcast(&self, payload: Bytes) -> Result<(), GossipError> {
        self.sender
            .broadcast(payload)
            .await
            .map_err(|e| GossipError::Transport(e.to_string()))
    }

    async fn recv(&self) -> Result<Bytes, GossipError> {
        use futures_lite::StreamExt;
        let mut rx = self.receiver.lock().await;
        loop {
            match rx.next().await {
                // Only `Received` carries an application payload.
                Some(Ok(Event::Received(msg))) => return Ok(msg.content),
                // Track swarm membership: the count gates the first claim and
                // wakes the rejoin loop when we drop to an island (eon).
                Some(Ok(Event::NeighborUp(id))) => {
                    self.neighbors_tx.send_modify(|c| *c += 1);
                    info!(peer = %id, count = *self.neighbors_tx.borrow(), "gossip: neighbor up");
                    continue;
                }
                Some(Ok(Event::NeighborDown(id))) => {
                    self.neighbors_tx.send_modify(|c| *c = c.saturating_sub(1));
                    info!(peer = %id, count = *self.neighbors_tx.borrow(), "gossip: neighbor down");
                    continue;
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(GossipError::Transport(e.to_string())),
                None => return Err(GossipError::Closed),
            }
        }
    }
}

/// Keep this node in the gossip swarm (mjolnir-mesh-eon). iroh-gossip's
/// bootstrap join is a one-shot dial: at boot the 802.11s radio and mDNS
/// discovery usually aren't up yet, every bootstrap dial fails ("No
/// addressing information available"), and the node stays a gossip island
/// forever — its anti-entropy broadcasts reach nobody and it merges nobody's,
/// so the claim-conflict machinery never fires. Meanwhile the tunnel data
/// plane comes up fine because `connector_loop` redials with backoff. This is
/// that same retry policy for the gossip swarm: whenever we have zero
/// neighbors, re-issue `join_peers` with the roster bootstrap set, capped
/// exponential backoff, resetting once we've been joined.
async fn gossip_rejoin_loop(
    sender: GossipSender,
    bootstrap: Vec<EndpointId>,
    mut neigh_rx: tokio::sync::watch::Receiver<usize>,
) {
    if bootstrap.is_empty() {
        return;
    }
    let min_backoff = Duration::from_secs(5);
    let max_backoff = Duration::from_secs(60);
    let mut backoff = min_backoff;
    loop {
        while *neigh_rx.borrow() > 0 {
            backoff = min_backoff; // we were joined; next outage starts fresh
            if neigh_rx.changed().await.is_err() {
                return; // transport dropped — shutting down
            }
        }
        info!(peers = bootstrap.len(), "gossip: no neighbors — (re)joining bootstrap peers");
        if let Err(e) = sender.join_peers(bootstrap.clone()).await {
            warn!("gossip: join_peers failed: {e}");
        }
        // Let the join attempt land (or fail) before trying again.
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

/// The fixed gossip topic for a mesh's CRDT overlay. Every node in the mesh
/// joins the same topic; the id is a constant hash so no coordination is needed.
fn mesh_topic_id() -> TopicId {
    TopicId::from_bytes(*blake3::hash(b"mjolnir/mesh/crdt/v0").as_bytes())
}

/// Short peer id for the interface name (8 hex chars is unique enough).
fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

/// P1 listener: accept tunnel connections, bring up a /31 TUN per peer.
async fn run_tun_listen(endpoint: Endpoint, no_relay: bool) -> Result<()> {
    wait_until_addressable(&endpoint, no_relay).await;
    print_identity(&endpoint)?;
    info!("tun-listen: hand the address above to a peer's `tun-connect`");

    let self_id = endpoint.id().to_string();
    let registry: TunnelRegistry = Arc::new(Mutex::new(HashMap::new()));
    let router = Router::builder(endpoint)
        .accept(TUN_ALPN, TunnelHandler { self_id, registry })
        .spawn();

    tokio::signal::ctrl_c().await.context("waiting for Ctrl-C")?;
    router.shutdown().await.context("router shutdown")?;
    Ok(())
}

/// P1 connector: dial a peer, bring up the tunnel, probe reachability across it.
async fn run_tun_connect(endpoint: Endpoint, addr_blob: &str) -> Result<()> {
    let addr = parse_peer(addr_blob).context("parsing peer")?;
    let peer = addr.id;
    let self_id = endpoint.id().to_string();

    info!(%peer, "tun-connect: dialing");
    let conn = endpoint
        .connect(addr, TUN_ALPN)
        .await
        .context("connect failed")?;

    let (self_addr, peer_addr) = mjolnir_mesh::tun::pick_link_31(&self_id, &peer.to_string());
    let tunnel = spawn_tunnel(
        short_id(&peer.to_string()),
        self_addr,
        peer_addr,
        IrohDatagramConn { conn: conn.clone() },
    )
    .await
    .context("bringing up tunnel")?;

    info!(
        iface = %tunnel.iface_name, %self_addr, %peer_addr,
        "tunnel up — probing reachability across it"
    );
    // Echo server on our own link addr (so the peer can probe us too).
    spawn_udp_echo(self_addr);
    // Give the peer a moment to bring up its side. iroh returns from connect()
    // as soon as a QUIC connection exists — which is over the *relay* initially;
    // hole-punching to a direct path happens asynchronously over the next few
    // seconds. Probing inside that window measures relay-only loss, which is high
    // for unreliable datagrams. Wait (bounded) for a direct path before the
    // headline probe, then report which path actually carried it.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let direct = wait_for_direct_path(&conn, Duration::from_secs(10)).await;
    log_conn_paths(&conn);
    probe_peer(peer_addr, direct).await;

    info!("tunnel established; holding open (Ctrl-C to exit)");
    tokio::signal::ctrl_c().await.context("waiting for Ctrl-C")?;
    drop(tunnel);
    Ok(())
}

/// P2 multi-peer mesh: accept inbound tunnels and dial every roster peer for
/// which this node is the initiator, maintaining one /31 TUN per peer with
/// redial-on-drop. Holds until Ctrl-C.
#[allow(clippy::too_many_arguments)] // cohesive mesh config; a struct would just move the noise
async fn run_mesh(
    endpoint: Endpoint,
    no_relay: bool,
    roster_path: Option<PathBuf>,
    peer_args: Vec<String>,
    babel_config: PathBuf,
    client_iface: String,
    lan: bool,
    lan_tunnels: bool,
    l2_backhaul: Option<String>,
    claims_file: PathBuf,
    directory_file: PathBuf,
    spool_dir: PathBuf,
    overlay: bool,
    gateway: bool,
    backhaul_ip: Ipv4Addr,
    restored_claims: HashMap<String, SubnetClaim>,
) -> Result<()> {
    // Client-gateway IP for the well-known `.mesh` names (e21.1.2): `None`
    // until this node's client /24 claim lands, at which point `claim_manager`
    // writes the claimed `.1` address through this handle. The responder
    // table only ever reads it, so the claim can land at any point after
    // `run_mesh` starts without re-plumbing the table.
    let gateway_handle: mjolnir_mesh::dns_responder::GatewayHandle = Arc::new(RwLock::new(None));

    // Service directory v2 (e21.2.3, owner-bound TOFU model — e21.2.1/e21.2.2):
    // book, tombstones, and local lost-names bookkeeping each get their own
    // lock, same convention as the other CRDT stores below. Restored from a
    // sibling `services2.state` — distinct from the pre-existing v1
    // `services.state` (7jb) restored further down, which is left untouched
    // for fleet compat until it's retired. Created here, ahead of `self_id`,
    // so the book can be handed to the DNS responder's `ServiceTable`
    // (e21.1.3) before the responder binds; nothing in this restore needs
    // `self_id` (that's only needed later for the own-vs-learned split in
    // gossip dispatch and anti-entropy re-announce).
    let service_book_v2_file = service_book_v2_path(&claims_file);
    let restored_v2 = load_service_state_v2(&service_book_v2_file);
    if !restored_v2.book.is_empty() || !restored_v2.tombstones.is_empty() {
        info!(
            services = restored_v2.book.len(),
            tombstones = restored_v2.tombstones.len(),
            path = %service_book_v2_file.display(),
            "restored v2 service directory from disk"
        );
    }
    let service_book_v2: Arc<Mutex<ServiceBookV2>> = Arc::new(Mutex::new(restored_v2.book));
    let service_tombstones_v2: Arc<Mutex<ServiceTombstoneBook>> =
        Arc::new(Mutex::new(restored_v2.tombstones));
    let lost_names_v2: Arc<Mutex<LostNameMap>> = Arc::new(Mutex::new(restored_v2.lost_names));

    // .mesh DNS responder (e21.1.1): bind BEFORE any UCI/dnsmasq reconcile
    // (FR14) — first thing in `run_mesh` so dnsmasq's `.mesh` upstream
    // (`server=/mesh/127.0.0.1#5335`) is answerable the instant it's
    // configured, however early that reconcile step lands. `CompositeTable`
    // (e21.1.2) stacks the well-known table ahead of the CRDT-projected v2
    // service table (e21.1.3), which reads straight from `service_book_v2`.
    let dns_table: Arc<dyn mjolnir_mesh::dns_responder::NameTable> =
        Arc::new(mjolnir_mesh::dns_responder::CompositeTable::new(vec![
            Arc::new(mjolnir_mesh::dns_responder::WellKnownTable::new(gateway_handle.clone())),
            Arc::new(mjolnir_mesh::dns_responder::ServiceTable::new(service_book_v2.clone())),
        ]));
    let dns_responder = mjolnir_mesh::dns_responder::start(
        SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            mjolnir_mesh::dns_responder::DEFAULT_DNS_PORT,
        ),
        dns_table,
    )
    .await
    .context("binding .mesh DNS responder")?;

    let self_id = endpoint.id();
    let self_id_str = self_id.to_string();
    // NB: the effective IPv4 backhaul address (`backhaul_ip`, claim-aware per
    // pt9) was already assigned to the shared-segment iface in `main`, before
    // the endpoint was built, so iroh picks it up at bind time and mDNS
    // announces it (mjolnir-mesh-4pk).

    wait_until_addressable(&endpoint, no_relay).await;
    print_identity(&endpoint)?;

    // Peer set = roster file (if any) merged with --peer args, deduped by token.
    let mut peer_entries: Vec<PeerEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(path) = roster_path.as_deref() {
        let roster = PeerRoster::load(path)
            .with_context(|| format!("loading roster {}", path.display()))?;
        for e in roster.peers() {
            if seen.insert(e.token.clone()) {
                peer_entries.push(e.clone());
            }
        }
    }
    for token in peer_args {
        if seen.insert(token.clone()) {
            peer_entries.push(PeerEntry { token, label: None });
        }
    }
    info!(peers = peer_entries.len(), "peer set resolved");

    // Forward client traffic between the TUN tunnels and the veth/bridge.
    enable_ip_forwarding();

    let registry: TunnelRegistry = Arc::new(Mutex::new(HashMap::new()));

    // buw overlay data plane (--overlay): bring up ONE mjolnir0 for all peers and
    // start its reader/writer/FIB-mirror. `overlay_state` carries the connection
    // manager + inbound sender the accept handler and dialers register into.
    // When off, this is None and everything below takes the per-peer path.
    let overlay_state: Option<(ConnManager, tokio::sync::mpsc::Sender<Bytes>)> = if overlay {
        let (device, link) = spawn_overlay_tun(backhaul_ip, OVERLAY_IFACE)
            .await
            .context("bringing up overlay TUN mjolnir0")?;
        info!(iface = %link.iface_name, addr = %link.self_addr, ll = %link.link_local, "overlay mode: single mjolnir0 up");
        let (writer, reader) = device.split().context("splitting overlay TUN")?;

        let conns = ConnManager::new();
        let fib = Arc::new(Mutex::new(Fib::new()));
        let (inbound_tx, mut inbound_rx) = tokio::sync::mpsc::channel::<Bytes>(1024);

        // Writer task: every peer's inbound datagrams funnel here -> mjolnir0.
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut writer = writer;
            while let Some(pkt) = inbound_rx.recv().await {
                if writer.write_all(&pkt).await.is_err() {
                    break;
                }
            }
        });
        // Mirror babeld's mjolnir0 routes into the FIB (demux next hops).
        tokio::spawn(fib_mirror(link.iface_name.clone(), fib.clone()));
        // Reader task: mjolnir0 -> route unicast / flood multicast to peers.
        let router = OverlayRouter {
            fib,
            conns: conns.clone(),
        };
        tokio::spawn(overlay_reader(reader, conns.clone(), router, TUNNEL_MTU as usize));

        Some((conns, inbound_tx))
    } else {
        None
    };

    // Shared CRDT subnet-claim store (mjolnir-mesh-chn): cidr -> claim. Written
    // by the gossip apply loop and the local claim routine; babeld (83k) reads
    // it for the local subnet to redistribute. Seeded from disk (mjolnir-mesh-s9v)
    // so a rebooting node has its own and any known peers' claims immediately,
    // before gossip has a chance to relearn them.
    // Loaded once in `main` before backhaul-address derivation (pt9); reused
    // here as the store seed so boot and claim state agree on one snapshot.
    let restored = restored_claims;
    if !restored.is_empty() {
        info!(count = restored.len(), path = %claims_file.display(), "restored subnet claims from disk");
    }
    let claims: ClaimStore = Arc::new(Mutex::new(restored));

    // Gossip address book (mjolnir-mesh-0yb): node_id → self-announced reachable
    // addresses. Seeded from disk so a rebooting node can dial known peers before
    // gossip relearns them, then augmented as PeerAddrUpdate messages arrive.
    // Persisted alongside the claims file (sibling addrbook.state) with the same
    // tolerant load / tmp+rename write semantics.
    let addr_book_file = addr_book_path(&claims_file);
    let restored_book = load_addr_book(&addr_book_file);
    if !restored_book.is_empty() {
        info!(count = restored_book.len(), path = %addr_book_file.display(), "restored peer address book from disk");
    }
    let addr_book: Arc<Mutex<AddrBook>> = Arc::new(Mutex::new(restored_book));

    // User directory (mjolnir-mesh-2xd / p6u): username → user identity record,
    // the first hello.mesh front-desk record type. Same persistence pattern as
    // the address book — a sibling `users.state`, tolerant load, tmp+rename
    // write — plus a plaintext `users.seed` (sibling) that lets a node ORIGINATE
    // user records with no control plane yet: each `username:Display Name` line
    // is stamped with a fresh HLC and gossiped on the anti-entropy cadence. Empty
    // by default (no seed file) so nodes only relay/persist what they receive.
    let user_book_file = user_book_path(&claims_file);
    let user_seed_file = user_seed_path(&claims_file);
    let restored_users = load_user_book(&user_book_file);
    if !restored_users.is_empty() {
        info!(count = restored_users.len(), path = %user_book_file.display(), "restored user directory from disk");
    }
    let user_book: Arc<Mutex<UserBook>> = Arc::new(Mutex::new(restored_users));

    // Service directory (mjolnir-mesh-7jb): service name → service record, the
    // focused e21 slice the hello.mesh directory needs. Same persistence pattern
    // as the user directory — a sibling `services.state`, tolerant load,
    // tmp+rename write. There is no seed file: services are learned over gossip
    // (and, later, originated by the node's mDNS bridge), not injected as text,
    // so the book is empty by default and nodes only relay/persist what they
    // receive. Anti-entropy re-broadcasts the full book each tick so a late
    // joiner or a node that missed a packet still converges.
    let service_book_file = service_book_path(&claims_file);
    let restored_services = load_service_book(&service_book_file);
    if !restored_services.is_empty() {
        info!(count = restored_services.len(), path = %service_book_file.display(), "restored service directory from disk");
    }
    let service_book: Arc<Mutex<ServiceBook>> = Arc::new(Mutex::new(restored_services));

    // CRDT gossip overlay (mjolnir-mesh-k8c): all mesh nodes join one fixed
    // topic and exchange CRDT updates best-effort, as a second protocol on the
    // same endpoint alongside the TUN data plane.
    let gossip = Gossip::builder().spawn(endpoint.clone());

    // Accept inbound tunnels (peers with a higher node id dial in) and gossip.
    // In overlay mode the TUN_ALPN handler serves connections into the connection
    // manager; otherwise it brings up a per-peer tunnel. Both spawn the same
    // Router type (handlers are boxed via ProtocolHandler).
    let router = if let Some((conns, inbound)) = &overlay_state {
        Router::builder(endpoint.clone())
            .accept(
                TUN_ALPN,
                OverlayHandler {
                    conns: conns.clone(),
                    inbound: inbound.clone(),
                },
            )
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn()
    } else {
        Router::builder(endpoint.clone())
            .accept(
                TUN_ALPN,
                TunnelHandler {
                    self_id: self_id_str.clone(),
                    registry: registry.clone(),
                },
            )
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn()
    };

    // Subscribe to the mesh CRDT topic, bootstrapping the gossip swarm with the
    // roster peers. On success, spawn two tasks: a dispatch loop that applies
    // inbound subnet claims to the store (merge/conflict), and a claim manager
    // that claims a /24 after a warmup and re-claims on conflict. A subscribe
    // failure is non-fatal: the TUN data plane still runs without the overlay.
    let bootstrap: Vec<EndpointId> = peer_entries
        .iter()
        .filter_map(|e| parse_peer(&e.token).ok())
        .map(|a| a.id)
        .filter(|id| *id != self_id)
        .collect();
    // LAN mode: seed iroh's address book with every roster peer's fully
    // DERIVED address — backhaul ip from the node id + the well-known mesh
    // port (0yb.1). Every node BINDS at that address, but dialing by bare id
    // relied on mDNS, which is unreliable over 802.11s and not yet resolved
    // at boot — so gossip bootstrap dials failed outright ("No addressing
    // information available") and, with the one-shot join, left every node a
    // permanent gossip island (mjolnir-mesh-eon). Derivation needs no
    // discovery at all; mDNS stays as a second candidate source.
    // One MemoryLookup the daemon owns for the whole run (mjolnir-mesh-0yb):
    // seeded at boot with every roster peer's DERIVED LAN address (unchanged
    // pre-existing behavior) and with the restored address book, then augmented
    // as gossiped PeerAddrUpdate entries arrive so dialing by node id works for
    // peers that were never L2 neighbors (multi-hop / cross-site). Registered
    // once with the endpoint's address-lookup services; a clone feeds the gossip
    // dispatch loop. `None` if the services are unavailable — dialing then falls
    // back to derived seeding / mDNS as before.
    let addr_lookup: Option<MemoryLookup> = match endpoint.address_lookup() {
        Ok(services) => {
            let lookup = MemoryLookup::with_provenance("mjolnir_addrbook");
            if lan {
                for id in &bootstrap {
                    // Claim-aware (pt9): a peer that lost a backhaul collision
                    // sits at a re-derived address, learned from its gossiped
                    // (and persisted) /32 claim; derivation is the fallback.
                    let ip = {
                        let s = claims.lock().expect("claim store poisoned");
                        peer_backhaul_hint(&s, &id.to_string())
                    };
                    let addr = SocketAddr::new(std::net::IpAddr::V4(ip), MESH_IROH_PORT);
                    lookup.add_endpoint_info(EndpointAddr::new(*id).with_ip_addr(addr));
                    info!(peer = %id, %addr, "seeded derived peer address (no-discovery dialing)");
                }
            }
            // Also seed from the restored address book (0yb): a rebooting node
            // can dial peers it learned last run before gossip re-announces them.
            {
                let book = addr_book.lock().expect("address book poisoned");
                for entry in book.values() {
                    feed_addr_lookup(&lookup, entry);
                }
            }
            services.add(lookup.clone());
            Some(lookup)
        }
        Err(e) => {
            warn!("address-lookup services unavailable — cannot seed peer addresses: {e}");
            None
        }
    };
    let (gossip_dispatch, claim_task, anti_entropy_task, rejoin_task) = match gossip
        .subscribe(mesh_topic_id(), bootstrap.clone())
        .await
    {
        Ok(topic) => {
            let (sender, receiver) = topic.split();
            // Neighbor count: fed by the dispatch loop's NeighborUp/Down
            // events; gates the first claim and drives the rejoin loop (eon).
            let (neighbors_tx, neigh_rx) = tokio::sync::watch::channel(0usize);
            let rejoin = tokio::spawn(gossip_rejoin_loop(
                sender.clone(),
                bootstrap,
                neigh_rx.clone(),
            ));
            let sync = Arc::new(GossipSync::new(IrohGossipTransport {
                sender,
                receiver: tokio::sync::Mutex::new(receiver),
                neighbors_tx,
            }));
            info!("gossip topic subscribed; joining swarm in background");

            // Signalled by the apply loop when a conflict costs us our claim;
            // carries the lost /24 so the claim manager can retract its address.
            let (reclaim_tx, reclaim_rx) = tokio::sync::mpsc::unbounded_channel::<Ipv4Net>();

            let dispatch = {
                let sync = sync.clone();
                let store = claims.clone();
                let me = self_id_str.clone();
                let claims_path = claims_file.clone();
                let book = addr_book.clone();
                let book_path = addr_book_file.clone();
                let lookup = addr_lookup.clone();
                let user_book = user_book.clone();
                let user_book_path = user_book_file.clone();
                let service_book = service_book.clone();
                let service_book_path = service_book_file.clone();
                let service_book_v2 = service_book_v2.clone();
                let service_tombstones_v2 = service_tombstones_v2.clone();
                let lost_names_v2 = lost_names_v2.clone();
                let service_book_v2_persist_path = service_book_v2_file.clone();
                tokio::spawn(async move {
                    let result = sync
                        .run(move |msg| {
                            // Address book (0yb): learn a peer's self-announced
                            // addresses, feed them into iroh so we can dial the
                            // peer by node id, persist, and log for field
                            // validation. Own echoes and stale (LWW) updates are
                            // dropped by apply_peer_addr_message. Handled first
                            // with an early return so a PeerAddrUpdate never takes
                            // the claim-store lock below.
                            if matches!(msg, GossipMessage::PeerAddrUpdate { .. }) {
                                let learned = {
                                    let mut b = book.lock().expect("address book poisoned");
                                    apply_peer_addr_message(&mut b, &msg, &me)
                                };
                                if let Some(entry) = learned {
                                    let snapshot =
                                        book.lock().expect("address book poisoned").clone();
                                    persist_addr_book(&snapshot, &book_path);
                                    if let Some(l) = &lookup {
                                        feed_addr_lookup(l, &entry);
                                    }
                                    info!(peer = %entry.node_id, addrs = entry.direct_addrs.len(),
                                        relay = ?entry.relay_url, "addrbook: learned peer address");
                                }
                                return;
                            }
                            // User directory (2xd/p6u): learn a user record from a
                            // peer, persist it, and log for field validation.
                            // Handled with an early return so it never takes the
                            // claim-store lock below. LWW/duplicate drops happen in
                            // apply_user_message.
                            if matches!(msg, GossipMessage::UserUpdate { .. }) {
                                let learned = {
                                    let mut u = user_book.lock().expect("user directory poisoned");
                                    apply_user_message(&mut u, &msg)
                                };
                                if let Some(entry) = learned {
                                    let snapshot =
                                        user_book.lock().expect("user directory poisoned").clone();
                                    persist_user_book(&snapshot, &user_book_path);
                                    info!(user = %entry.username, display = %entry.display_name,
                                        by = %entry.registered_by, "gossip: received user record");
                                }
                                return;
                            }
                            // Service directory (7jb): learn a service record from
                            // a peer, persist it, and log for field validation.
                            // Early return so it never takes the claim-store lock
                            // below. LWW/duplicate drops happen in
                            // apply_service_message.
                            if matches!(msg, GossipMessage::ServiceUpdate { .. }) {
                                let learned = {
                                    let mut s = service_book.lock().expect("service directory poisoned");
                                    apply_service_message(&mut s, &msg)
                                };
                                if let Some((name, entry)) = learned {
                                    let snapshot =
                                        service_book.lock().expect("service directory poisoned").clone();
                                    persist_service_book(&snapshot, &service_book_path);
                                    info!(service = %name, host = %entry.hostname,
                                        ip = %entry.ip, port = entry.port, "gossip: received service record");
                                }
                                return;
                            }
                            // Service directory v2 publish (e21.2.2/e21.2.3):
                            // apply the owner-bound merge, tracking a
                            // conflict loss against `lost_names` (e21.2.4)
                            // when it makes US the loser for this name.
                            // Early return so it never touches the v1
                            // service book or the claim-store lock below.
                            if matches!(msg, GossipMessage::ServicePublishV2 { .. }) {
                                if let GossipMessage::ServicePublishV2 { name, entry } = &msg {
                                    let outcome = {
                                        let mut b = service_book_v2.lock().expect("v2 service book poisoned");
                                        let tombstones =
                                            service_tombstones_v2.lock().expect("v2 service tombstones poisoned");
                                        let mut lost = lost_names_v2.lock().expect("v2 service lost-names poisoned");
                                        apply_service_publish_v2_tracking_loss(
                                            &mut b, &tombstones, &mut lost, &me, name, entry.clone(),
                                        )
                                    };
                                    match outcome {
                                        Ok(outcome) => {
                                            info!(service = %name, owner = %entry.owner_node_id, ?outcome,
                                                "gossip: received v2 service publish");
                                        }
                                        Err(e) => {
                                            warn!(service = %name, "gossip: rejected v2 service publish: {e}");
                                        }
                                    }
                                    let snapshot = snapshot_service_state_v2(
                                        &service_book_v2, &service_tombstones_v2, &lost_names_v2,
                                    );
                                    persist_service_state_v2(&snapshot, &service_book_v2_persist_path);
                                }
                                return;
                            }
                            // Service directory v2 unpublish (e21.2.2/e21.2.3):
                            // apply the tombstone-vs-publish rules. Early
                            // return, same reasoning as the publish arm above.
                            if matches!(msg, GossipMessage::ServiceUnpublishV2 { .. }) {
                                if let GossipMessage::ServiceUnpublishV2 { name, owner_node_id, hlc } = &msg {
                                    let outcome = {
                                        let mut b = service_book_v2.lock().expect("v2 service book poisoned");
                                        let mut tombstones =
                                            service_tombstones_v2.lock().expect("v2 service tombstones poisoned");
                                        apply_service_unpublish_v2(&mut b, &mut tombstones, name, owner_node_id, hlc.clone())
                                    };
                                    info!(service = %name, owner = %owner_node_id, ?outcome,
                                        "gossip: received v2 service unpublish");
                                    let snapshot = snapshot_service_state_v2(
                                        &service_book_v2, &service_tombstones_v2, &lost_names_v2,
                                    );
                                    persist_service_state_v2(&snapshot, &service_book_v2_persist_path);
                                }
                                return;
                            }
                            // Log peer claims received over gossip — proves CRDT
                            // convergence (a node seeing another's claim cross the mesh).
                            if let GossipMessage::SubnetClaimUpdate { cidr, entry } = &msg
                                && entry.owner_node_id != me
                            {
                                info!(%cidr, owner = %entry.owner_node_id, "gossip: received peer subnet claim");
                            }
                            let mut s = store.lock().expect("claim store poisoned");
                            if let Some(lost) = apply_subnet_message(&mut s, &msg, &me) {
                                if mjolnir_mesh::tun::in_backhaul_block(&lost) {
                                    // Lost our backhaul /32 claim (pt9): the earlier
                                    // claimant keeps 10.254.x. The address is baked
                                    // into the bound iroh socket and interface config,
                                    // so persist the winner's claim and exit — procd
                                    // respawns meshd, and pick_backhaul_addr() derives
                                    // around the persisted winner at next boot.
                                    error!(addr = %lost,
                                        "backhaul address collision lost — restarting to re-derive (pt9)");
                                    persist_claims(&s, &claims_path);
                                    drop(s);
                                    std::process::exit(EXIT_BACKHAUL_COLLISION);
                                }
                                drop(s);
                                let _ = reclaim_tx.send(lost);
                            }
                        })
                        .await;
                    if let Err(e) = result {
                        warn!("gossip dispatch loop ended: {e}");
                    }
                })
            };

            let claim = {
                let sync = sync.clone();
                let store = claims.clone();
                let me = self_id_str.clone();
                let neigh_rx = neigh_rx.clone();
                let claims_path = claims_file.clone();
                let gateway = gateway_handle.clone();
                tokio::spawn(async move {
                    claim_manager(
                        sync,
                        store,
                        me,
                        client_iface,
                        backhaul_ip,
                        claims_path,
                        reclaim_rx,
                        neigh_rx,
                        gateway,
                    )
                    .await
                })
            };

            // Anti-entropy (mjolnir-mesh-s9v, part 1 of 5r0): periodically re-broadcast
            // the FULL known claim map (not just our own claim — that weaker form is
            // `claim_and_publish` above) and persist it to disk. Fixes late-joiner /
            // dropped-packet / restart convergence cheaply since the map is tiny.
            let anti_entropy = {
                let sync = sync.clone();
                let store = claims.clone();
                let path = claims_file.clone();
                let book = addr_book.clone();
                let book_path = addr_book_file.clone();
                let users = user_book.clone();
                let users_path = user_book_file.clone();
                let users_seed = user_seed_file.clone();
                let services = service_book.clone();
                let services_path = service_book_file.clone();
                let services_v2 = service_book_v2.clone();
                let tombstones_v2 = service_tombstones_v2.clone();
                let lost_names_v2 = lost_names_v2.clone();
                let services_v2_path = service_book_v2_file.clone();
                let directory_path = directory_file.clone();
                let spool_path = spool_dir.clone();
                let announce = SelfAnnounce {
                    endpoint: endpoint.clone(),
                    self_id: self_id_str.clone(),
                    backhaul_ip,
                    no_relay,
                };
                tokio::spawn(async move {
                    anti_entropy_loop(
                        sync, store, path, book, book_path, users, users_path, users_seed,
                        services, services_path, services_v2, tombstones_v2, lost_names_v2,
                        services_v2_path, directory_path, spool_path, announce,
                    )
                    .await
                })
            };

            (Some(dispatch), Some(claim), Some(anti_entropy), Some(rejoin))
        }
        Err(e) => {
            warn!("gossip subscribe failed: {e}; continuing without CRDT overlay");
            (None, None, None, None)
        }
    };

    // babeld config reconciler (mjolnir-mesh-83k / m8t): regenerates babeld.conf
    // from the live tunnel set (TunnelRegistry) plus our subnet claim (ClaimStore)
    // and triggers the `mjolnir-babeld` procd service to (re)load on change. procd
    // — not meshd — owns the babeld PROCESS (start/respawn/boot/stop); meshd only
    // owns the config. babeld absence is non-fatal.
    let babel_task = if overlay_state.is_some() {
        // Overlay: render ONE static mjolnir0 config from the claim store; no
        // per-peer interfaces means the config never churns (qz9 by construction).
        let claims = claims.clone();
        let me = self_id_str.clone();
        tokio::spawn(async move { babel_reconciler_overlay(claims, me, babel_config, gateway).await })
    } else {
        let registry = registry.clone();
        let claims = claims.clone();
        let me = self_id_str.clone();
        let l2 = l2_backhaul.clone();
        tokio::spawn(
            async move { babel_reconciler(registry, claims, me, babel_config, l2, gateway).await },
        )
    };
    if let Some(iface) = &l2_backhaul {
        info!(%iface, "LAN mode: routing babel over the shared-L2 backhaul (no per-peer iroh tunnels)");
    }

    // Spawn one dialer task per peer we initiate to. Tie-break by node id so
    // exactly one side of each pair dials (the lexicographically-lower id) and
    // the other accepts — otherwise both ends would race to create the same
    // deterministic /31 interface. This mirrors `pick_link_31`'s ordering.
    //
    // In LAN mode we DON'T dial per-peer iroh tunnels at all: babel routes over
    // the shared-L2 backhaul (above), which is stable, while iroh's path manager
    // churned the per-peer tunnels (mjolnir-mesh-auu). iroh/gossip stays up for
    // the CRDT control plane; only the L3 data-plane tunnels are dropped here.
    let mut dialers = Vec::new();
    // LAN default routes data over the shared-L2 backhaul (babel), NOT per-peer
    // iroh tunnels — they churned in the container era (mjolnir-mesh-auu). The
    // --lan-tunnels flag re-enables them for the native retest; internet mode
    // always tunnels.
    // Overlay mode always dials every peer (it needs one connection to each);
    // otherwise the per-peer tunnel policy applies.
    let want_tunnels = overlay_state.is_some() || !lan || lan_tunnels;
    if !want_tunnels {
        info!("LAN mode: not dialing per-peer iroh tunnels — babel routes over the shared L2");
    } else {
        if lan && lan_tunnels {
            info!("LAN mode: per-peer iroh tunnels ENABLED (--lan-tunnels; mjolnir-mesh-auu retest)");
        }
        for entry in &peer_entries {
            let addr = match parse_peer(&entry.token) {
                Ok(a) => a,
                Err(e) => {
                    warn!(token = %entry.token, "skipping unparseable roster entry: {e}");
                    continue;
                }
            };
            let peer = addr.id;
            if peer == self_id {
                continue; // our own id appears in the roster — skip
            }
            // Per-peer LAN: dial the peer at its DERIVED backhaul address
            // (10.254.x:MESH_IROH_PORT), reachable over the babel-routed underlay
            // with no flat-L2 mDNS (mjolnir-mesh-0yb.1). NOT in overlay mode: there
            // 10.254.x is the peer's OVERLAY address (on mjolnir0, reachable only
            // over the overlay itself), and iroh isn't pinned to that port — so
            // overlay dials by NODE ID and lets iroh discovery (mDNS/relay) resolve
            // the underlay address (mjolnir-mesh-buw.8). Internet mode likewise
            // keeps discovery/relay resolution.
            let addr = if lan && overlay_state.is_none() {
                let ip = {
                    let s = claims.lock().expect("claim store poisoned");
                    peer_backhaul_hint(&s, &peer.to_string())
                };
                addr.with_ip_addr(SocketAddr::new(std::net::IpAddr::V4(ip), MESH_IROH_PORT))
            } else {
                addr
            };
            if self_id_str < peer.to_string() {
                let ep = endpoint.clone();
                let label = entry.label.clone();
                if let Some((conns, inbound)) = &overlay_state {
                    let conns = conns.clone();
                    let inbound = inbound.clone();
                    dialers.push(tokio::spawn(async move {
                        connector_loop_overlay(ep, addr, conns, inbound, label).await;
                    }));
                } else {
                    let reg = registry.clone();
                    let sid = self_id_str.clone();
                    dialers.push(tokio::spawn(async move {
                        connector_loop(ep, addr, sid, reg, label).await;
                    }));
                }
            } else {
                info!(%peer, label = ?entry.label, "peer has the higher id — waiting for it to dial us");
            }
        }
    }
    info!(dialing = dialers.len(), "mesh up — holding (Ctrl-C to exit)");

    tokio::signal::ctrl_c().await.context("waiting for Ctrl-C")?;
    info!("shutting down mesh");
    for d in &dialers {
        d.abort();
    }
    if let Some(t) = &gossip_dispatch {
        t.abort();
    }
    if let Some(t) = &claim_task {
        t.abort();
    }
    if let Some(t) = &anti_entropy_task {
        t.abort();
    }
    if let Some(t) = &rejoin_task {
        t.abort();
    }
    dns_responder.abort();
    babel_task.abort();
    // babeld runs as its own procd service (mjolnir-babeld); intentionally NOT
    // stopped here so a meshd restart doesn't churn it (mjolnir-mesh-m8t).
    router.shutdown().await.context("router shutdown")?;
    Ok(())
}

/// Maintain a tunnel to one peer: dial, serve until it drops, then redial with
/// capped exponential backoff. Runs until the task is aborted (mesh shutdown).
async fn connector_loop(
    endpoint: Endpoint,
    addr: EndpointAddr,
    self_id: String,
    registry: TunnelRegistry,
    label: Option<String>,
) {
    let peer = addr.id;
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);
    loop {
        info!(%peer, label = ?label, "dialing peer");
        match endpoint.connect(addr.clone(), TUN_ALPN).await {
            Ok(conn) => {
                backoff = Duration::from_secs(1); // reset after a successful dial
                if let Err(e) = serve_tunnel(conn, &self_id, &registry).await {
                    warn!(%peer, "tunnel ended with error: {e}");
                }
                // Connection closed; brief pause before redialing.
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                warn!(%peer, ?backoff, "dial failed: {e}; retrying after backoff");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        }
    }
}

// --- subnet claim (mjolnir-mesh-chn) -------------------------------------

/// How long a fresh node (no restored claim) waits for a first gossip
/// neighbor before claiming blind. Covers slow radio/mDNS bring-up at boot;
/// the cap exists so the genuinely-first node of a new mesh still claims.
const CLAIM_JOIN_WAIT_CAP: Duration = Duration::from_secs(60);

/// After the first gossip neighbor appears, wait one full anti-entropy period
/// (plus slack) so every neighbor's claim map has a chance to arrive before we
/// pick a subnet. The old blind 8s warmup lost this race in the field by 13s
/// and re-collided by construction — the deterministic blake3 preferred slot
/// picks the SAME /24 again unless the peer's claim is already in the store
/// (mjolnir-mesh-eon).
const CLAIM_POST_JOIN_WARMUP: Duration = Duration::from_secs(25);

/// Client-subnet size each router claims from the mesh space (10.42.0.0/16).
const CLIENT_PREFIX_LEN: u8 = 24;

/// Anti-entropy period (mjolnir-mesh-s9v): how often each node re-broadcasts
/// its full known claim map and rewrites the on-disk claims file. The claim
/// map is tiny (~64KB at the 256-node cap), so this is cheap; it exists to fix
/// late-joiner / dropped-packet / restart convergence without any new gossip
/// protocol, just a resend of what `SubnetClaimUpdate` already carries.
const ANTI_ENTROPY_INTERVAL: Duration = Duration::from_secs(20);

/// Shared CRDT subnet-claim store: cidr string -> claim. Written by the gossip
/// apply loop and the local claim routine; babeld (mjolnir-mesh-83k) will read
/// it for the local subnet to redistribute.
type ClaimStore = Arc<Mutex<HashMap<String, SubnetClaim>>>;

/// Build an HLC stamped with the current wall clock for `node_id`.
fn now_hlc(node_id: &str) -> HLC {
    let wall_clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    HLC {
        wall_clock,
        counter: 0,
        node_id: node_id.to_string(),
    }
}

/// Apply an inbound subnet CRDT message to the claim store. Returns the /24
/// THIS node lost if a conflict cost us our claim — the caller must retract
/// its gateway address and re-claim. Pure over the map (no I/O) so it's
/// unit-tested below.
fn apply_subnet_message(
    store: &mut HashMap<String, SubnetClaim>,
    msg: &GossipMessage,
    self_id: &str,
) -> Option<Ipv4Net> {
    match msg {
        GossipMessage::SubnetClaimUpdate { cidr, entry } => {
            match merge_subnet_claim(store.get(cidr), entry) {
                MergeResult::Inserted | MergeResult::Updated => {
                    store.insert(cidr.clone(), entry.clone());
                    None
                }
                MergeResult::Unchanged => None,
                MergeResult::Conflict { winner, loser } => {
                    let we_lost =
                        loser.owner_node_id == self_id && winner.owner_node_id != self_id;
                    let lost = match (we_lost, loser.cidr) {
                        (true, IpNet::V4(n)) => Some(n),
                        _ => None,
                    };
                    store.insert(cidr.clone(), winner);
                    lost
                }
            }
        }
        GossipMessage::SubnetClaimRelease { cidr, hlc } => {
            if store
                .get(cidr)
                .is_some_and(|existing| *hlc >= existing.claimed_at)
            {
                store.remove(cidr);
            }
            None
        }
        // Lease/DNS/Service CRDT messages are out of scope for the subnet claim.
        _ => None,
    }
}

/// Wait until the gossip swarm has at least one neighbor, capped at `cap`.
/// Returns `true` if a neighbor appeared, `false` on timeout or a dropped
/// channel. Pure over the watch channel so it's unit-tested below.
async fn wait_for_first_neighbor(
    mut neigh_rx: tokio::sync::watch::Receiver<usize>,
    cap: Duration,
) -> bool {
    tokio::time::timeout(cap, async {
        while *neigh_rx.borrow() == 0 {
            if neigh_rx.changed().await.is_err() {
                return false;
            }
        }
        true
    })
    .await
    .unwrap_or(false)
}

/// Manage this node's subnet claim: learn existing claims, pick a free /24 and
/// publish it; re-claim whenever a conflict costs us ours, retracting the lost
/// subnet's gateway address first so the node doesn't keep answering on a /24
/// the mesh has routed elsewhere.
///
/// A node with a restored claim publishes immediately — re-publishing our own
/// claim is conflict-free (first-writer seniority), so the LAN comes up fast.
/// A fresh node gates its first pick on gossip actually joining: wait (capped)
/// for a neighbor, then a full anti-entropy period so existing claims arrive —
/// the old blind 8s warmup claimed 13s before the first peer claim landed and
/// re-collided by construction (deterministic preferred slot).
#[allow(clippy::too_many_arguments)] // one more thread-through param (gateway) on an already-cohesive set
async fn claim_manager<T: GossipTransport>(
    sync: Arc<GossipSync<T>>,
    store: ClaimStore,
    self_id: String,
    client_iface: String,
    backhaul_ip: Ipv4Addr,
    claims_file: PathBuf,
    mut reclaim_rx: tokio::sync::mpsc::UnboundedReceiver<Ipv4Net>,
    neigh_rx: tokio::sync::watch::Receiver<usize>,
    gateway: mjolnir_mesh::dns_responder::GatewayHandle,
) {
    // Backhaul /32 claim first (pt9): the address is already assigned and the
    // socket bound to it, so publish immediately — no neighbor gating. FWW
    // arbitrates any collision; the loser restarts and re-derives.
    claim_backhaul_and_publish(&sync, &store, &self_id, backhaul_ip, &claims_file).await;
    let has_own_claim = {
        let s = store.lock().expect("claim store poisoned");
        // Backhaul /32 claims don't count — this gates the CLIENT /24 pick.
        s.values().any(|c| {
            c.owner_node_id == self_id
                && matches!(c.cidr, IpNet::V4(n) if !mjolnir_mesh::tun::in_backhaul_block(&n))
        })
    };
    if !has_own_claim {
        if wait_for_first_neighbor(neigh_rx, CLAIM_JOIN_WAIT_CAP).await {
            tokio::time::sleep(CLAIM_POST_JOIN_WARMUP).await;
        } else {
            warn!(
                cap = ?CLAIM_JOIN_WAIT_CAP,
                "no gossip neighbor within the join cap — claiming blind (first node of a new mesh, or peers unreachable)"
            );
        }
    }
    claim_and_publish(&sync, &store, &self_id, &client_iface, &gateway).await;
    while let Some(lost) = reclaim_rx.recv().await {
        // Brief pause so a conflict storm settles before we re-pick.
        tokio::time::sleep(Duration::from_secs(1)).await;
        info!(subnet = %lost, "lost our subnet claim in a conflict — retracting its address and re-claiming");
        retract_client_addr(lost, &client_iface).await;
        // No claim held between retraction and re-claim: the well-known
        // names fall back to the pre-claim gateway (D-003) rather than keep
        // answering on a /24 this node no longer owns.
        *gateway.write().expect("gateway handle poisoned") = None;
        claim_and_publish(&sync, &store, &self_id, &client_iface, &gateway).await;
    }
}

/// Partition the claim map from `self_id`'s point of view: the senior claim we
/// own (lowest HLC — first-writer-wins seniority), any extra claims we own
/// beyond it (to be released), and every other node's claimed v4 subnets (to
/// be avoided when picking fresh). Pure so it's unit-tested below.
fn partition_claims(
    store: &HashMap<String, SubnetClaim>,
    self_id: &str,
) -> (
    Option<(Ipv4Net, SubnetClaim)>,
    Vec<Ipv4Net>,
    HashSet<Ipv4Net>,
) {
    // Backhaul /32 claims (pt9) share the store but are NOT client subnets —
    // exclude them from both sides so they can't be picked as the senior
    // client claim, released as "extras", or fed to the /24 allocator.
    let mut own: Vec<(Ipv4Net, SubnetClaim)> = store
        .values()
        .filter(|c| c.owner_node_id == self_id)
        .filter_map(|c| match c.cidr {
            IpNet::V4(n) if !mjolnir_mesh::tun::in_backhaul_block(&n) => Some((n, c.clone())),
            _ => None,
        })
        .collect();
    let foreign: HashSet<Ipv4Net> = store
        .values()
        .filter(|c| c.owner_node_id != self_id)
        .filter_map(|c| match c.cidr {
            IpNet::V4(n) if !mjolnir_mesh::tun::in_backhaul_block(&n) => Some(n),
            _ => None,
        })
        .collect();
    own.sort_by(|a, b| a.1.claimed_at.cmp(&b.1.claimed_at));
    let mut own = own.into_iter();
    let keep = own.next();
    let extras = own.map(|(n, _)| n).collect();
    (keep, extras, foreign)
}

/// Exit code used when this node loses its backhaul-address claim (pt9): the
/// address is baked into the bound socket and interface config, so the clean
/// resolution is a supervised restart — `pick_backhaul_addr` then derives
/// around the persisted winner. procd respawns meshd on any nonzero exit.
const EXIT_BACKHAUL_COLLISION: i32 = 86;

/// How many salted derivations to try before giving up on avoidance and letting
/// FWW arbitrate at runtime. 64 misses in a ~65k slot space would take a mesh of
/// thousands of pathologically colliding nodes — effectively unreachable.
const BACKHAUL_PICK_ATTEMPTS: u32 = 64;

/// Pick this node's effective backhaul address (pt9). Prefers a backhaul /32
/// claim we already own in the (restored) claim map — the address survives
/// restarts with its first-writer seniority intact, including a re-derived one
/// after a lost collision. Otherwise walks the salted derivation sequence,
/// skipping addresses another node is known to have claimed; attempt 0 is the
/// legacy `backhaul_addr` derivation, so the common case is unchanged.
fn pick_backhaul_addr(store: &HashMap<String, SubnetClaim>, self_id: &str) -> Ipv4Addr {
    let mut own: Vec<&SubnetClaim> = store
        .values()
        .filter(|c| c.owner_node_id == self_id)
        .filter(|c| matches!(c.cidr, IpNet::V4(n) if mjolnir_mesh::tun::in_backhaul_block(&n)))
        .collect();
    own.sort_by(|a, b| a.claimed_at.cmp(&b.claimed_at));
    if let Some(c) = own.first()
        && let IpNet::V4(n) = c.cidr
    {
        return n.addr();
    }
    let taken: HashSet<Ipv4Addr> = store
        .values()
        .filter(|c| c.owner_node_id != self_id)
        .filter_map(|c| match c.cidr {
            IpNet::V4(n) if mjolnir_mesh::tun::in_backhaul_block(&n) && n.prefix_len() == 32 => {
                Some(n.addr())
            }
            _ => None,
        })
        .collect();
    for attempt in 0..BACKHAUL_PICK_ATTEMPTS {
        let addr = mjolnir_mesh::tun::backhaul_addr_salted(self_id, attempt);
        if !taken.contains(&addr) {
            return addr;
        }
    }
    mjolnir_mesh::tun::backhaul_addr(self_id)
}

/// Resolve the address to dial a roster peer at: its gossiped backhaul /32
/// claim if we know one (a collision loser sits at a re-derived address), else
/// the attempt-0 derivation — the pre-pt9 behavior (0yb.1 derived seeding).
fn peer_backhaul_hint(store: &HashMap<String, SubnetClaim>, peer_id: &str) -> Ipv4Addr {
    let mut owned: Vec<&SubnetClaim> = store
        .values()
        .filter(|c| c.owner_node_id == peer_id)
        .filter(|c| matches!(c.cidr, IpNet::V4(n) if mjolnir_mesh::tun::in_backhaul_block(&n)))
        .collect();
    owned.sort_by(|a, b| a.claimed_at.cmp(&b.claimed_at));
    if let Some(c) = owned.first()
        && let IpNet::V4(n) = c.cidr
    {
        return n.addr();
    }
    mjolnir_mesh::tun::backhaul_addr(peer_id)
}

/// Record and gossip this node's backhaul /32 claim (pt9). Reuses an existing
/// own claim on the address (restored across restarts — preserves first-writer
/// seniority, mirroring the eon rule for client /24s). If another node already
/// holds a claim on this address, the deterministic merge decides: if we would
/// lose, exit for a supervised restart (pick_backhaul_addr avoids the winner);
/// if we would win (e.g. the other claim is wall-clock-skewed into our future),
/// claim anyway — the other node is the one that must move.
async fn claim_backhaul_and_publish<T: GossipTransport>(
    sync: &GossipSync<T>,
    store: &ClaimStore,
    self_id: &str,
    addr: Ipv4Addr,
    claims_file: &Path,
) {
    let net = Ipv4Net::new(addr, 32).expect("/32 prefix is always valid");
    let key = IpNet::V4(net).to_string();
    let claim = {
        let mut s = store.lock().expect("claim store poisoned");
        let fresh = SubnetClaim {
            cidr: IpNet::V4(net),
            owner_node_id: self_id.to_string(),
            site_name: None,
            claimed_at: now_hlc(self_id),
        };
        match merge_subnet_claim(s.get(&key), &fresh) {
            MergeResult::Conflict { winner, .. } if winner.owner_node_id != self_id => {
                error!(%addr, winner = %winner.owner_node_id,
                    "backhaul address already claimed by an earlier writer — restarting to re-derive (pt9)");
                persist_claims(&s, claims_file);
                drop(s);
                std::process::exit(EXIT_BACKHAUL_COLLISION);
            }
            _ => {
                let entry = match s.get(&key) {
                    Some(c) if c.owner_node_id == self_id => c.clone(),
                    _ => fresh,
                };
                s.insert(key.clone(), entry.clone());
                entry
            }
        }
    };
    match sync
        .publish(GossipMessage::SubnetClaimUpdate {
            cidr: key,
            entry: claim,
        })
        .await
    {
        Ok(()) => info!(%addr, "published backhaul address claim (pt9)"),
        Err(e) => warn!(%addr, "backhaul claim publish failed: {e}"),
    }
}

/// Publish this node's subnet claim. A claim we already own — typically
/// restored from disk across a restart — is reused and re-published as-is
/// (same `claimed_at`, preserving first-writer seniority), NOT avoided:
/// treating our own restored claim as foreign made a rebooting node claim a
/// fresh /24 while still holding and gossiping the old one (mjolnir-mesh-eon).
/// Extra self-owned claims accumulated by that bug are released. Otherwise
/// pick a free /24 (avoiding known claims), record it, assign its `.1` to the
/// client interface as a connected route (so babeld can redistribute it), and
/// gossip the claim.
async fn claim_and_publish<T: GossipTransport>(
    sync: &GossipSync<T>,
    store: &ClaimStore,
    self_id: &str,
    client_iface: &str,
    gateway: &mjolnir_mesh::dns_responder::GatewayHandle,
) {
    let (keep, extras, foreign) = {
        let s = store.lock().expect("claim store poisoned");
        partition_claims(&s, self_id)
    };
    for extra in extras {
        release_claim(sync, store, self_id, extra, client_iface).await;
    }
    if let Some((net, claim)) = keep {
        match sync
            .publish(GossipMessage::SubnetClaimUpdate {
                cidr: net.to_string(),
                entry: claim,
            })
            .await
        {
            Ok(()) => info!(subnet = %net, "re-published held subnet claim"),
            Err(e) => warn!(subnet = %net, "re-publishing held claim failed: {e}"),
        }
        assign_client_addr(net, client_iface).await;
        reconcile_client_uci(net).await;
        *gateway.write().expect("gateway handle poisoned") = Some(client_gateway_addr(net));
        return;
    }
    let net = match alloc::pick_subnet_or_smaller(
        self_id,
        &foreign,
        alloc::DEFAULT_MESH_SPACE,
        CLIENT_PREFIX_LEN,
    ) {
        Some(n) => n,
        None => {
            warn!("no free subnet available in the mesh space to claim");
            return;
        }
    };
    let cidr_key = net.to_string();
    let claim = SubnetClaim {
        cidr: IpNet::V4(net),
        owner_node_id: self_id.to_string(),
        site_name: None,
        claimed_at: now_hlc(self_id),
    };
    store
        .lock()
        .expect("claim store poisoned")
        .insert(cidr_key.clone(), claim.clone());
    match sync
        .publish(GossipMessage::SubnetClaimUpdate {
            cidr: cidr_key,
            entry: claim,
        })
        .await
    {
        Ok(()) => info!(subnet = %net, "claimed client subnet and published it"),
        Err(e) => warn!(subnet = %net, "claimed subnet but gossip publish failed: {e}"),
    }

    // Assign the /24's gateway address (.1) to the client interface, so babeld has
    // a concrete connected route to redistribute and inbound mesh traffic for the
    // /24 is delivered on-link (mjolnir-mesh-e4r, supersedes the df4 gateway route).
    assign_client_addr(net, client_iface).await;
    reconcile_client_uci(net).await;
    *gateway.write().expect("gateway handle poisoned") = Some(client_gateway_addr(net));
}

/// Release a claim this node owns but should no longer hold: drop it from the
/// store, gossip a `SubnetClaimRelease` stamped now (≥ its `claimed_at`, so
/// peers drop it too), and retract its gateway address from the client
/// interface. Self-heals the duplicate claims a restart could accumulate
/// before the eon fix.
async fn release_claim<T: GossipTransport>(
    sync: &GossipSync<T>,
    store: &ClaimStore,
    self_id: &str,
    net: Ipv4Net,
    client_iface: &str,
) {
    let cidr_key = net.to_string();
    store
        .lock()
        .expect("claim store poisoned")
        .remove(&cidr_key);
    match sync
        .publish(GossipMessage::SubnetClaimRelease {
            cidr: cidr_key,
            hlc: now_hlc(self_id),
        })
        .await
    {
        Ok(()) => info!(subnet = %net, "released extra subnet claim"),
        Err(e) => warn!(subnet = %net, "releasing subnet claim: gossip publish failed: {e}"),
    }
    retract_client_addr(net, client_iface).await;
}

/// Load the persisted claim map from `path`. Returns an empty map (not an
/// error) if the file is absent — the normal case on first boot — or if it
/// fails to decode, since the claim store is best-effort and will relearn
/// current state over gossip either way.
fn load_claims(path: &Path) -> HashMap<String, SubnetClaim> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HashMap::new(),
        Err(e) => {
            warn!(path = %path.display(), "failed to read persisted claims: {e}");
            return HashMap::new();
        }
    };
    match postcard::from_bytes(&bytes) {
        Ok(map) => map,
        Err(e) => {
            warn!(path = %path.display(), "failed to decode persisted claims: {e}");
            HashMap::new()
        }
    }
}

/// Persist a claim-map snapshot to `path`, writing to a sibling temp file and
/// renaming over the target so a crash or power loss mid-write (a real risk
/// on routers) can't leave a truncated, undecodable file. Best effort: a
/// failure is logged, not fatal.
fn persist_claims(snapshot: &HashMap<String, SubnetClaim>, path: &Path) {
    let bytes = match postcard::to_allocvec(snapshot) {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to encode claims for persistence: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(path = %parent.display(), "failed to create claims dir: {e}");
        return;
    }
    let tmp_path = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp_path, &bytes) {
        warn!(path = %tmp_path.display(), "failed to write claims tmp file: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        warn!(path = %path.display(), "failed to rename claims tmp file into place: {e}");
    }
}

/// The address-book state file path (0yb): a sibling of the claims file
/// (default `/etc/mjolnir/addrbook.state`). Derived rather than a new CLI flag
/// so the fleet picks it up with no config change; follows however the claims
/// file was configured.
fn addr_book_path(claims_file: &Path) -> PathBuf {
    claims_file.with_file_name("addrbook.state")
}

/// Load the persisted address book from `path`. Returns an empty book (not an
/// error) if the file is absent — the normal case on first boot — or if it
/// fails to decode, since the book is best-effort and relearns current state
/// over gossip. Mirrors [`load_claims`].
fn load_addr_book(path: &Path) -> AddrBook {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AddrBook::new(),
        Err(e) => {
            warn!(path = %path.display(), "failed to read persisted address book: {e}");
            return AddrBook::new();
        }
    };
    match postcard::from_bytes(&bytes) {
        Ok(book) => book,
        Err(e) => {
            warn!(path = %path.display(), "failed to decode persisted address book: {e}");
            AddrBook::new()
        }
    }
}

/// Persist an address-book snapshot to `path`, writing to a sibling temp file
/// and renaming over the target so a crash or power loss mid-write can't leave
/// a truncated, undecodable file. Best effort: a failure is logged, not fatal.
/// Mirrors [`persist_claims`].
fn persist_addr_book(snapshot: &AddrBook, path: &Path) {
    let bytes = match postcard::to_allocvec(snapshot) {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to encode address book for persistence: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(path = %parent.display(), "failed to create address book dir: {e}");
        return;
    }
    let tmp_path = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp_path, &bytes) {
        warn!(path = %tmp_path.display(), "failed to write address book tmp file: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        warn!(path = %path.display(), "failed to rename address book tmp file into place: {e}");
    }
}

/// The user-directory state file path (2xd/p6u): a sibling of the claims file
/// (default `/etc/mjolnir/users.state`). Derived, not a new CLI flag, so the
/// fleet picks it up with no config change. Mirrors [`addr_book_path`].
fn user_book_path(claims_file: &Path) -> PathBuf {
    claims_file.with_file_name("users.state")
}

/// The user-directory SEED file path (2xd/p6u): a sibling of the claims file
/// (default `/etc/mjolnir/users.seed`). Plaintext `username:Display Name` lines,
/// one per record this node ORIGINATES. Absent by default — the normal case —
/// so nodes only relay/persist what they receive over gossip. This is the
/// stand-in for the real identity-submission control plane (p6u) so a new
/// record type can be injected and observed on the fleet today.
fn user_seed_path(claims_file: &Path) -> PathBuf {
    claims_file.with_file_name("users.seed")
}

/// Load the persisted user directory from `path`. Empty (not an error) if the
/// file is absent (first boot) or fails to decode — the book is best-effort and
/// relearns over gossip. Mirrors [`load_addr_book`].
fn load_user_book(path: &Path) -> UserBook {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return UserBook::new(),
        Err(e) => {
            warn!(path = %path.display(), "failed to read persisted user directory: {e}");
            return UserBook::new();
        }
    };
    match postcard::from_bytes(&bytes) {
        Ok(book) => book,
        Err(e) => {
            warn!(path = %path.display(), "failed to decode persisted user directory: {e}");
            UserBook::new()
        }
    }
}

/// Persist a user-directory snapshot via tmp+rename (crash-safe). Best effort:
/// failures are logged, not fatal. Mirrors [`persist_addr_book`].
fn persist_user_book(snapshot: &UserBook, path: &Path) {
    let bytes = match postcard::to_allocvec(snapshot) {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to encode user directory for persistence: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(path = %parent.display(), "failed to create user directory dir: {e}");
        return;
    }
    let tmp_path = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp_path, &bytes) {
        warn!(path = %tmp_path.display(), "failed to write user directory tmp file: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        warn!(path = %path.display(), "failed to rename user directory tmp file into place: {e}");
    }
}

/// The service-directory state file path (7jb): a sibling of the claims file
/// (default `/etc/mjolnir/services.state`). Derived, not a new CLI flag, so the
/// fleet picks it up with no config change. Mirrors [`user_book_path`].
fn service_book_path(claims_file: &Path) -> PathBuf {
    claims_file.with_file_name("services.state")
}

/// Load the persisted service directory from `path`. Empty (not an error) if the
/// file is absent (first boot) or fails to decode — the book is best-effort and
/// relearns over gossip. Mirrors [`load_user_book`].
fn load_service_book(path: &Path) -> ServiceBook {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ServiceBook::new(),
        Err(e) => {
            warn!(path = %path.display(), "failed to read persisted service directory: {e}");
            return ServiceBook::new();
        }
    };
    match postcard::from_bytes(&bytes) {
        Ok(book) => book,
        Err(e) => {
            warn!(path = %path.display(), "failed to decode persisted service directory: {e}");
            ServiceBook::new()
        }
    }
}

/// Persist a service-directory snapshot via tmp+rename (crash-safe). Best effort:
/// failures are logged, not fatal. Mirrors [`persist_user_book`].
fn persist_service_book(snapshot: &ServiceBook, path: &Path) {
    let bytes = match postcard::to_allocvec(snapshot) {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to encode service directory for persistence: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(path = %parent.display(), "failed to create service directory dir: {e}");
        return;
    }
    let tmp_path = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp_path, &bytes) {
        warn!(path = %tmp_path.display(), "failed to write service directory tmp file: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        warn!(path = %path.display(), "failed to rename service directory tmp file into place: {e}");
    }
}

/// Combined v2 service persistence shape (bead e21.2.3): the owner-bound
/// book, its tombstones, and this node's local lost-names bookkeeping
/// (e21.2.4) travel together in one sibling file, `services2.state` —
/// distinct from the pre-existing v1 `services.state` (7jb), which holds a
/// different wire type (`ServiceEntry`, not `ServiceEntryV2`) and is left
/// untouched: the v1 lane stays live for fleet compat until it's retired.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ServiceStateV2 {
    book: ServiceBookV2,
    tombstones: ServiceTombstoneBook,
    lost_names: LostNameMap,
}

/// The v2 service-state file path (e21.2.3): a sibling of the claims file
/// (default `/etc/mjolnir/services2.state`). Derived, not a new CLI flag, so
/// the fleet picks it up with no config change. Mirrors [`service_book_path`].
fn service_book_v2_path(claims_file: &Path) -> PathBuf {
    claims_file.with_file_name("services2.state")
}

/// Load the persisted v2 service state from `path`. Empty (not an error) if
/// the file is absent (first boot, or a v1-only fleet member) or fails to
/// decode — best-effort, same tolerant-load discipline as every other CRDT
/// state file here. Mirrors [`load_service_book`].
fn load_service_state_v2(path: &Path) -> ServiceStateV2 {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ServiceStateV2::default(),
        Err(e) => {
            warn!(path = %path.display(), "failed to read persisted v2 service state: {e}");
            return ServiceStateV2::default();
        }
    };
    match postcard::from_bytes(&bytes) {
        Ok(state) => state,
        Err(e) => {
            warn!(path = %path.display(), "failed to decode persisted v2 service state: {e}");
            ServiceStateV2::default()
        }
    }
}

/// Persist a v2 service-state snapshot via tmp+rename (crash-safe). Best
/// effort: failures are logged, not fatal. Mirrors [`persist_service_book`].
fn persist_service_state_v2(snapshot: &ServiceStateV2, path: &Path) {
    let bytes = match postcard::to_allocvec(snapshot) {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to encode v2 service state for persistence: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(path = %parent.display(), "failed to create v2 service state dir: {e}");
        return;
    }
    let tmp_path = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp_path, &bytes) {
        warn!(path = %tmp_path.display(), "failed to write v2 service state tmp file: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        warn!(path = %path.display(), "failed to rename v2 service state tmp file into place: {e}");
    }
}

/// Snapshot the three v2 service-state locks into one [`ServiceStateV2`] for
/// persistence. Locks in the fixed order book → tombstones → lost_names
/// everywhere in this daemon, to avoid any lock-ordering deadlock risk.
fn snapshot_service_state_v2(
    book: &Arc<Mutex<ServiceBookV2>>,
    tombstones: &Arc<Mutex<ServiceTombstoneBook>>,
    lost_names: &Arc<Mutex<LostNameMap>>,
) -> ServiceStateV2 {
    let book = book.lock().expect("v2 service book poisoned").clone();
    let tombstones = tombstones.lock().expect("v2 service tombstones poisoned").clone();
    let lost_names = lost_names.lock().expect("v2 service lost-names poisoned").clone();
    ServiceStateV2 { book, tombstones, lost_names }
}

/// Schema version for the `directory.json` projection (bead avs). Bump this
/// whenever the on-disk shape changes in a way `mjolnir-hello` needs to know
/// about, so the daemon and the hello server can evolve independently.
const DIRECTORY_SCHEMA_VERSION: u32 = 1;

/// Read-only snapshot of mesh state that `mjolnir-hello` reads directly (it
/// does NOT re-derive state from the CRDT stores itself). Written atomically
/// (tmp+rename) by [`persist_directory`] on the anti-entropy cadence. See bead
/// mjolnir-mesh-avs.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct DirectorySnapshot {
    version: u32,
    node: DirectoryNode,
    neighbors: Vec<DirectoryNeighbor>,
    identities: Vec<DirectoryIdentity>,
    services: Vec<DirectoryService>,
}

/// "You are here": this node's own identity, claimed client subnet (if any),
/// and derived overlay backhaul address.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct DirectoryNode {
    node_id: String,
    /// This node's claimed client `/24` (e.g. `10.42.1.0/24`), if it has
    /// claimed one yet. `None` during the post-boot warmup window.
    subnet: Option<String>,
    backhaul_addr: String,
}

/// One other mesh node, joining its [`AddrBook`] entry with any subnet claim
/// it owns.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct DirectoryNeighbor {
    node_id: String,
    addrs: Vec<String>,
    subnet: Option<String>,
}

/// One `/users` record, projected for the front desk.
#[derive(Debug, Clone, PartialEq, Serialize)]
struct DirectoryIdentity {
    username: String,
    display_name: String,
}

/// One service-directory record, projected for the front desk. `name` is the
/// `ServiceBook` map key (the fully-qualified service name), not
/// [`ServiceEntry::hostname`].
#[derive(Debug, Clone, PartialEq, Serialize)]
struct DirectoryService {
    name: String,
    ip: String,
    port: u16,
    protocol: String,
}

/// Find the client `/24` (if any) owned by `node_id` in the claim map, e.g.
/// `10.42.1.0/24`. Excludes backhaul `/32` claims (mjolnir-mesh-pt9) — those
/// are overlay addressing, not a client subnet. Pure; shared by
/// [`build_directory_snapshot`] for both the "you are here" node section and
/// each neighbor.
fn owned_client_subnet(claims: &HashMap<String, SubnetClaim>, node_id: &str) -> Option<String> {
    claims
        .values()
        .filter(|c| c.owner_node_id == node_id)
        .find_map(|c| match c.cidr {
            IpNet::V4(n) if !mjolnir_mesh::tun::in_backhaul_block(&n) => Some(n.to_string()),
            _ => None,
        })
}

/// Project the daemon's four in-memory CRDT stores into the read-only
/// `directory.json` shape `mjolnir-hello` reads (bead avs). Pure over plain
/// snapshots (no locks, no I/O) so it's natively unit-testable without the
/// `daemon` feature's Linux-only dependencies — the timer/write wiring that
/// calls this is exercised by [`write_directory_projection`] instead.
fn build_directory_snapshot(
    claims: &HashMap<String, SubnetClaim>,
    addr_book: &AddrBook,
    user_book: &UserBook,
    service_book: &ServiceBook,
    self_id: &str,
    backhaul_ip: Ipv4Addr,
) -> DirectorySnapshot {
    let node = DirectoryNode {
        node_id: self_id.to_string(),
        subnet: owned_client_subnet(claims, self_id),
        backhaul_addr: backhaul_ip.to_string(),
    };

    let neighbors = addr_book
        .values()
        .filter(|entry| entry.node_id != self_id)
        .map(|entry| DirectoryNeighbor {
            node_id: entry.node_id.clone(),
            addrs: entry.direct_addrs.iter().map(ToString::to_string).collect(),
            subnet: owned_client_subnet(claims, &entry.node_id),
        })
        .collect();

    let identities = user_book
        .values()
        .map(|u| DirectoryIdentity {
            username: u.username.clone(),
            display_name: u.display_name.clone(),
        })
        .collect();

    let services = service_book
        .iter()
        .map(|(name, entry)| DirectoryService {
            name: name.clone(),
            ip: entry.ip.to_string(),
            port: entry.port,
            protocol: entry.protocol.clone(),
        })
        .collect();

    DirectorySnapshot {
        version: DIRECTORY_SCHEMA_VERSION,
        node,
        neighbors,
        identities,
        services,
    }
}

/// Persist a directory-projection snapshot to `path` as pretty JSON, writing
/// to a sibling temp file and renaming over the target so a crash or power
/// loss mid-write can't leave `mjolnir-hello` reading a torn file. Best
/// effort: a failure is logged, not fatal. Mirrors [`persist_service_book`],
/// but JSON (via `serde_json`) rather than postcard, since this file is read
/// by another process rather than round-tripped by this daemon.
fn persist_directory(snapshot: &DirectorySnapshot, path: &Path) {
    let bytes = match serde_json::to_vec_pretty(snapshot) {
        Ok(b) => b,
        Err(e) => {
            warn!("failed to encode directory projection for persistence: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(path = %parent.display(), "failed to create directory projection dir: {e}");
        return;
    }
    let tmp_path = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp_path, &bytes) {
        warn!(path = %tmp_path.display(), "failed to write directory projection tmp file: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        warn!(path = %path.display(), "failed to rename directory projection tmp file into place: {e}");
    }
}

/// Briefly lock each of the four CRDT stores to clone a cheap snapshot,
/// release the locks, then build and persist the `directory.json` projection
/// (bead avs). Called once up front and once per anti-entropy tick from
/// [`anti_entropy_loop`], mirroring how the other books re-persist on the same
/// cadence.
fn write_directory_projection(
    claims: &ClaimStore,
    addr_book: &Arc<Mutex<AddrBook>>,
    user_book: &Arc<Mutex<UserBook>>,
    service_book: &Arc<Mutex<ServiceBook>>,
    self_id: &str,
    backhaul_ip: Ipv4Addr,
    directory_file: &Path,
) {
    let claims_snapshot = claims.lock().expect("claim store poisoned").clone();
    let addr_snapshot = addr_book.lock().expect("address book poisoned").clone();
    let user_snapshot = user_book.lock().expect("user directory poisoned").clone();
    let service_snapshot = service_book.lock().expect("service directory poisoned").clone();
    let snapshot = build_directory_snapshot(
        &claims_snapshot,
        &addr_snapshot,
        &user_snapshot,
        &service_snapshot,
        self_id,
        backhaul_ip,
    );
    persist_directory(&snapshot, directory_file);
}

/// Apply an inbound service CRDT message to the directory. Returns the
/// `(name, entry)` newly inserted or updated (so the caller can persist and
/// log), or `None` for another CRDT type or an LWW-stale/duplicate update. Pure
/// over the map (no I/O) so it's unit-tested below — mirrors [`apply_user_message`].
fn apply_service_message(
    book: &mut ServiceBook,
    msg: &GossipMessage,
) -> Option<(String, ServiceEntry)> {
    let GossipMessage::ServiceUpdate { name, entry } = msg else {
        return None;
    };
    match merge_service(book.get(name), entry) {
        MergeResult::Inserted | MergeResult::Updated => {
            book.insert(name.clone(), entry.clone());
            Some((name.clone(), entry.clone()))
        }
        // merge_service is pure LWW — never Conflict.
        MergeResult::Unchanged | MergeResult::Conflict { .. } => None,
    }
}

/// Parse the seed file into user records this node originates. Each non-empty,
/// non-`#` line is `username:Display Name` (display defaults to username if the
/// colon is omitted). Every record is stamped with a fresh HLC and
/// `registered_by = self_id`. A missing file yields an empty vec (the default).
fn load_user_seed(path: &Path, self_id: &str) -> Vec<UserEntry> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            warn!(path = %path.display(), "failed to read user seed: {e}");
            return Vec::new();
        }
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let (username, display) = match line.split_once(':') {
                Some((u, d)) => (u.trim(), d.trim()),
                None => (line, line),
            };
            UserEntry {
                username: username.to_string(),
                display_name: display.to_string(),
                registered_by: self_id.to_string(),
                attrs: std::collections::BTreeMap::new(),
                updated_at: now_hlc(self_id),
            }
        })
        .collect()
}

/// A pending identity submission written by `mjolnir-hello` into the spool dir
/// (story 5zn): `pending/{pubkey}.json`. `mjolnir-hello` has already
/// Ed25519-verified `sig` over `challenge` before spooling it, so meshd's job
/// is purely to turn an accepted submission into a `/users` record and gossip
/// it mesh-wide — this is the real p6u identity-submission control plane that
/// replaces the `users.seed` plaintext stand-in.
#[derive(Debug, Clone, Deserialize)]
struct SpoolSubmission {
    pubkey: String,
    #[allow(dead_code)] // not re-verified here; see spool_submission_to_user_entry doc
    sig: String,
    #[allow(dead_code)]
    challenge: String,
    #[serde(default)]
    label: Option<String>,
}

/// A short, human-scannable form of a hex pubkey for use as a default display
/// name when a submission carries no `label` (first 8 hex chars + ellipsis).
fn short_pubkey(pubkey: &str) -> String {
    let n = pubkey.len().min(8);
    format!("{}…", &pubkey[..n])
}

/// Map a parsed spool submission into a `/users` CRDT record (p6u). The pubkey
/// is the stable identity key (`username`, mirroring how [`load_user_seed`]
/// uses a stable handle as the key); `display_name` is the caller-chosen
/// `label` if present, else [`short_pubkey`]. `registered_by` is this node's
/// id — the node that ingested the submission, not necessarily the node the
/// user connected to — and the record is stamped with a fresh HLC, same as a
/// freshly-read seed line. The pubkey is duplicated into `attrs` so it survives
/// alongside the record even though it's already the key. Re-verifying the
/// Ed25519 signature here was left out: `mjolnir-hello` already verified it
/// before spooling, and the daemon build (`--features daemon`) has no ed25519
/// dependency wired in today — see the p6u Dev Agent Record for the tradeoff.
fn spool_submission_to_user_entry(sub: &SpoolSubmission, self_id: &str) -> UserEntry {
    let display_name = sub
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| short_pubkey(&sub.pubkey));
    let mut attrs = std::collections::BTreeMap::new();
    attrs.insert("pubkey".to_string(), sub.pubkey.clone());
    UserEntry {
        username: sub.pubkey.clone(),
        display_name,
        registered_by: self_id.to_string(),
        attrs,
        updated_at: now_hlc(self_id),
    }
}

/// Sweep the identity spool dir for `*.json` submissions and merge each into
/// the user directory (p6u). Called from [`anti_entropy_loop`] right before
/// [`announce_user_book`], which re-broadcasts the FULL book — so a newly
/// merged entry rides that same tick's gossip, exactly like a freshly-read
/// `users.seed` line. Idempotent: `merge_user` is LWW, so re-ingesting the same
/// file (should the delete below ever fail) is harmless. Malformed JSON is
/// logged and the file is quarantined to a `.bad` sidecar rather than deleted,
/// so a human can inspect what `mjolnir-hello` wrote; any I/O error is logged
/// and skipped. Neither ever aborts the sweep — one bad file must never wedge
/// the anti-entropy loop.
fn ingest_identity_spool(spool_dir: &Path, user_book: &Arc<Mutex<UserBook>>, self_id: &str) {
    let entries = match std::fs::read_dir(spool_dir) {
        Ok(e) => e,
        // No spool dir yet (nothing submitted, or mjolnir-hello hasn't run) —
        // not an error, mirrors the tolerant load of the other books.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            warn!(path = %spool_dir.display(), "identity spool: failed to read dir: {e}");
            return;
        }
    };
    for dir_entry in entries.flatten() {
        let path = dir_entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                warn!(path = %path.display(), "identity spool: failed to read file: {e}");
                continue;
            }
        };
        let sub: SpoolSubmission = match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(e) => {
                warn!(path = %path.display(), "identity spool: malformed submission, quarantining: {e}");
                let bad = path.with_extension("json.bad");
                if let Err(e) = std::fs::rename(&path, &bad) {
                    warn!(path = %path.display(), "identity spool: failed to quarantine malformed file: {e}");
                }
                continue;
            }
        };
        let entry = spool_submission_to_user_entry(&sub, self_id);
        {
            let mut book = user_book.lock().expect("user directory poisoned");
            match merge_user(book.get(&entry.username), &entry) {
                MergeResult::Inserted | MergeResult::Updated => {
                    info!(pubkey = %entry.username, display = %entry.display_name,
                        "identity spool: ingested submission into /users");
                    book.insert(entry.username.clone(), entry);
                }
                // Stale/duplicate (already ingested by us or a peer, LWW) — still
                // remove the spool file below, same as a successful ingest.
                MergeResult::Unchanged | MergeResult::Conflict { .. } => {}
            }
        }
        if let Err(e) = std::fs::remove_file(&path) {
            warn!(path = %path.display(), "identity spool: failed to remove ingested file: {e}");
        }
    }
}

/// Apply an inbound user CRDT message to the directory. Returns the entry newly
/// inserted or updated (so the caller can persist and log), or `None` for
/// another CRDT type or an LWW-stale/duplicate update. Pure over the map (no
/// I/O) so it's unit-tested below — mirrors [`apply_peer_addr_message`].
fn apply_user_message(book: &mut UserBook, msg: &GossipMessage) -> Option<UserEntry> {
    let GossipMessage::UserUpdate { username, entry } = msg else {
        return None;
    };
    match merge_user(book.get(username), entry) {
        MergeResult::Inserted | MergeResult::Updated => {
            book.insert(username.clone(), entry.clone());
            Some(entry.clone())
        }
        // merge_user is pure LWW — never Conflict.
        MergeResult::Unchanged | MergeResult::Conflict { .. } => None,
    }
}

/// Apply an inbound peer-address CRDT message to the address book. Returns the
/// entry that was newly inserted or updated (so the caller can feed iroh,
/// persist, and log), or `None` if the message was for another CRDT type, was
/// our own self-announcement echoed back to us, or carried nothing newer (LWW).
/// Pure over the map (no I/O) so it's unit-tested below — mirrors
/// [`apply_subnet_message`].
fn apply_peer_addr_message(
    book: &mut AddrBook,
    msg: &GossipMessage,
    self_id: &str,
) -> Option<PeerAddrEntry> {
    let GossipMessage::PeerAddrUpdate { node_id, entry } = msg else {
        return None;
    };
    // Never learn our own address from the mesh — we announce it authoritatively.
    if node_id == self_id {
        return None;
    }
    match merge_peer_addr(book.get(node_id), entry) {
        MergeResult::Inserted | MergeResult::Updated => {
            book.insert(node_id.clone(), entry.clone());
            Some(entry.clone())
        }
        // Unchanged (stale/duplicate). merge_peer_addr never yields Conflict —
        // a single node is the sole announcer of its own entry (pure LWW).
        MergeResult::Unchanged | MergeResult::Conflict { .. } => None,
    }
}

/// Feed a learned peer address-book entry into the daemon's [`MemoryLookup`] so
/// iroh can dial the peer by node id even when it was never an L2 neighbor
/// (multi-hop / cross-site — 0yb). Direct addresses are always added; the relay
/// URL is attached when it parses as a [`RelayUrl`] (the iroh API supports this
/// cleanly via `EndpointAddr::with_relay_url`). A node id that doesn't parse as
/// an [`EndpointId`] is skipped and logged.
fn feed_addr_lookup(lookup: &MemoryLookup, entry: &PeerAddrEntry) {
    let id: EndpointId = match entry.node_id.parse() {
        Ok(id) => id,
        Err(e) => {
            warn!(node_id = %entry.node_id, "addrbook: peer id does not parse as EndpointId, not feeding iroh: {e}");
            return;
        }
    };
    let mut ep_addr = EndpointAddr::new(id);
    for a in &entry.direct_addrs {
        ep_addr = ep_addr.with_ip_addr(*a);
    }
    if let Some(url) = &entry.relay_url {
        match url.parse::<RelayUrl>() {
            Ok(r) => ep_addr = ep_addr.with_relay_url(r),
            Err(e) => {
                warn!(node_id = %entry.node_id, relay = %url, "addrbook: relay URL unparseable, skipping relay: {e}")
            }
        }
    }
    lookup.add_endpoint_info(ep_addr);
}

/// Anti-entropy loop (mjolnir-mesh-s9v): every [`ANTI_ENTROPY_INTERVAL`],
/// re-broadcast every claim this node currently knows about — not just its
/// own — and rewrite the on-disk claims file. Re-broadcasting the full map
/// (rather than only our own claim, the weaker form `claim_and_publish`
/// already does) is what lets a late joiner, a node that missed a gossip
/// packet, or a node that just rebooted converge without any pull-based
/// reconciliation protocol.
#[allow(clippy::too_many_arguments)]
async fn anti_entropy_loop<T: GossipTransport>(
    sync: Arc<GossipSync<T>>,
    store: ClaimStore,
    claims_file: PathBuf,
    addr_book: Arc<Mutex<AddrBook>>,
    addr_book_file: PathBuf,
    user_book: Arc<Mutex<UserBook>>,
    user_book_file: PathBuf,
    user_seed_file: PathBuf,
    service_book: Arc<Mutex<ServiceBook>>,
    service_book_file: PathBuf,
    service_book_v2: Arc<Mutex<ServiceBookV2>>,
    service_tombstones_v2: Arc<Mutex<ServiceTombstoneBook>>,
    lost_names_v2: Arc<Mutex<LostNameMap>>,
    service_book_v2_file: PathBuf,
    directory_file: PathBuf,
    spool_dir: PathBuf,
    self_announce: SelfAnnounce,
) {
    // Self-announce our address once up front (0yb): unlike the claim map, whose
    // initial publish is `claim_manager`'s warmup, the address book has no
    // separate warmup publisher, so the first broadcast happens here before the
    // ticker's immediately-consumed first tick.
    announce_addr_book(&sync, &addr_book, &addr_book_file, &self_announce).await;
    // Ingest any identity submissions already waiting in the spool (p6u) before
    // the first announce, so a pending submission from before this boot isn't
    // stuck an extra tick.
    ingest_identity_spool(&spool_dir, &user_book, &self_announce.self_id);
    // Likewise seed+announce the user directory up front (2xd/p6u).
    announce_user_book(
        &sync,
        &user_book,
        &user_book_file,
        &user_seed_file,
        &self_announce.self_id,
    )
    .await;
    // Likewise re-broadcast the service directory up front (7jb).
    announce_service_book(&sync, &service_book, &service_book_file).await;
    // Likewise re-announce this node's own v2 service entries up front
    // (e21.2.3, D-006) — learned entries are served but never re-announced.
    announce_service_book_v2(
        &sync,
        &service_book_v2,
        &service_tombstones_v2,
        &lost_names_v2,
        &service_book_v2_file,
        &self_announce.self_id,
    )
    .await;
    // Write the initial directory.json projection up front too (avs), so
    // mjolnir-hello has a snapshot to read before the first anti-entropy tick.
    write_directory_projection(
        &store,
        &addr_book,
        &user_book,
        &service_book,
        &self_announce.self_id,
        self_announce.backhaul_ip,
        &directory_file,
    );
    let mut ticker = tokio::time::interval(ANTI_ENTROPY_INTERVAL);
    ticker.tick().await; // first tick fires immediately; the warmup claim publish already covered this
    loop {
        ticker.tick().await;
        let snapshot = store.lock().expect("claim store poisoned").clone();
        for (cidr, entry) in &snapshot {
            if let Err(e) = sync
                .publish(GossipMessage::SubnetClaimUpdate {
                    cidr: cidr.clone(),
                    entry: entry.clone(),
                })
                .await
            {
                warn!(%cidr, "anti-entropy: re-broadcast failed: {e}");
            }
        }
        info!(count = snapshot.len(), "anti-entropy: re-broadcast full claim map");
        persist_claims(&snapshot, &claims_file);
        // Re-announce our own address and re-broadcast the full address book
        // alongside the claim map, on the same cadence (0yb).
        announce_addr_book(&sync, &addr_book, &addr_book_file, &self_announce).await;
        // Sweep the identity spool (p6u): each accepted submission `mjolnir-hello`
        // wrote becomes a `/users` record here, merged into the book so the
        // announce call right below picks it up in its full-book re-broadcast —
        // exactly the model the `users.seed` stand-in used for injecting records.
        ingest_identity_spool(&spool_dir, &user_book, &self_announce.self_id);
        // Re-read the seed, re-stamp our originated records, and re-broadcast the
        // full user directory on the same cadence (2xd/p6u).
        announce_user_book(
            &sync,
            &user_book,
            &user_book_file,
            &user_seed_file,
            &self_announce.self_id,
        )
        .await;
        // Re-broadcast the full service directory on the same cadence (7jb).
        announce_service_book(&sync, &service_book, &service_book_file).await;
        // Re-announce this node's own v2 service entries on the same cadence
        // (e21.2.3, D-006).
        announce_service_book_v2(
            &sync,
            &service_book_v2,
            &service_tombstones_v2,
            &lost_names_v2,
            &service_book_v2_file,
            &self_announce.self_id,
        )
        .await;
        // Re-project the read-only directory.json snapshot on the same cadence
        // (avs), after the books above have been refreshed for this tick.
        write_directory_projection(
            &store,
            &addr_book,
            &user_book,
            &service_book,
            &self_announce.self_id,
            self_announce.backhaul_ip,
            &directory_file,
        );
    }
}

/// Inputs for rebuilding this node's own address-book entry each anti-entropy
/// tick (0yb). Cloning is cheap — an [`Endpoint`] handle is internally an `Arc`.
struct SelfAnnounce {
    endpoint: Endpoint,
    self_id: String,
    backhaul_ip: Ipv4Addr,
    no_relay: bool,
}

/// Build this node's self-announced address-book entry: our observed direct
/// addresses (from the endpoint) plus the deterministic bound backhaul address
/// every node binds in LAN mode (`backhaul_ip:MESH_IROH_PORT`), and our relay
/// URL unless relays are disabled. Stamped with a fresh HLC each call so LWW
/// always carries the latest snapshot — re-stamping every tick is simpler than
/// diffing the address set and still converges, since a single node is the sole
/// announcer of its own entry (no conflict arm). See mjolnir-mesh-0yb.
fn build_self_addr_entry(ctx: &SelfAnnounce) -> PeerAddrEntry {
    let observed = ctx.endpoint.addr();
    let mut direct: Vec<SocketAddr> = observed.ip_addrs().copied().collect();
    // Always include the derived bound backhaul address: in LAN mode this is the
    // address peers dial, and it may not appear in the endpoint's observed set
    // until discovery settles (PeerAddrEntry::new dedups if it does).
    direct.push(SocketAddr::new(IpAddr::V4(ctx.backhaul_ip), MESH_IROH_PORT));
    let relay_url = if ctx.no_relay {
        None
    } else {
        observed.relay_urls().next().map(|u| u.to_string())
    };
    PeerAddrEntry::new(ctx.self_id.clone(), direct, relay_url, now_hlc(&ctx.self_id))
}

/// Refresh this node's own entry (fresh HLC), then re-broadcast the FULL known
/// address book — ours and every peer's — and rewrite the on-disk book.
/// Full-map anti-entropy mirroring the claim map, so a late joiner or a node
/// that missed a packet still converges without a pull protocol (0yb). The lock
/// is held only to insert-and-clone the snapshot; it is never held across an
/// `.await`.
async fn announce_addr_book<T: GossipTransport>(
    sync: &GossipSync<T>,
    addr_book: &Arc<Mutex<AddrBook>>,
    addr_book_file: &Path,
    self_announce: &SelfAnnounce,
) {
    let snapshot = {
        let entry = build_self_addr_entry(self_announce);
        let mut book = addr_book.lock().expect("address book poisoned");
        book.insert(self_announce.self_id.clone(), entry);
        book.clone()
    };
    for (node_id, entry) in &snapshot {
        if let Err(e) = sync
            .publish(GossipMessage::PeerAddrUpdate {
                node_id: node_id.clone(),
                entry: entry.clone(),
            })
            .await
        {
            warn!(%node_id, "addrbook anti-entropy: re-broadcast failed: {e}");
        }
    }
    info!(count = snapshot.len(), "addrbook anti-entropy: re-broadcast full address book");
    persist_addr_book(&snapshot, addr_book_file);
}

/// Re-read the seed file, merge our originated records (fresh HLC each tick, so
/// LWW always carries this node's latest edit — mirroring how the address book
/// re-stamps its self entry), then re-broadcast the FULL user directory and
/// rewrite the on-disk book. Full-map anti-entropy so a late joiner or a node
/// that missed a packet still converges without a pull protocol (2xd/p6u). The
/// lock is only held to merge-and-clone the snapshot, never across an `.await`.
async fn announce_user_book<T: GossipTransport>(
    sync: &GossipSync<T>,
    user_book: &Arc<Mutex<UserBook>>,
    user_book_file: &Path,
    user_seed_file: &Path,
    self_id: &str,
) {
    let snapshot = {
        let seeded = load_user_seed(user_seed_file, self_id);
        let mut book = user_book.lock().expect("user directory poisoned");
        for entry in seeded {
            // Merge so a peer's newer record for the same username isn't clobbered
            // by our (older) seed; our fresh HLC normally wins for records we own.
            if matches!(
                merge_user(book.get(&entry.username), &entry),
                MergeResult::Inserted | MergeResult::Updated
            ) {
                book.insert(entry.username.clone(), entry);
            }
        }
        book.clone()
    };
    for (username, entry) in &snapshot {
        if let Err(e) = sync
            .publish(GossipMessage::UserUpdate {
                username: username.clone(),
                entry: entry.clone(),
            })
            .await
        {
            warn!(%username, "user anti-entropy: re-broadcast failed: {e}");
        }
    }
    info!(count = snapshot.len(), "user anti-entropy: re-broadcast full user directory");
    persist_user_book(&snapshot, user_book_file);
}

/// Re-broadcast the FULL service directory and rewrite the on-disk book (7jb).
/// Full-map anti-entropy so a late joiner or a node that missed a packet still
/// converges without a pull protocol. Unlike the user directory there is no seed
/// to re-stamp — services are learned over gossip (or originated elsewhere) — so
/// this simply clones the current book and re-publishes it. The lock is only held
/// to clone the snapshot, never across an `.await`.
async fn announce_service_book<T: GossipTransport>(
    sync: &GossipSync<T>,
    service_book: &Arc<Mutex<ServiceBook>>,
    service_book_file: &Path,
) {
    let snapshot = service_book.lock().expect("service directory poisoned").clone();
    for (name, entry) in &snapshot {
        if let Err(e) = sync
            .publish(GossipMessage::ServiceUpdate {
                name: name.clone(),
                entry: entry.clone(),
            })
            .await
        {
            warn!(%name, "service anti-entropy: re-broadcast failed: {e}");
        }
    }
    info!(count = snapshot.len(), "service anti-entropy: re-broadcast full service directory");
    persist_service_book(&snapshot, service_book_file);
}

/// Re-announce THIS node's own v2 service entries and unpublish tombstones,
/// and rewrite the on-disk v2 service state (bead e21.2.3, decision D-006).
///
/// Unlike [`announce_service_book`]'s v1 full-map re-broadcast, only entries
/// (and tombstones) `owner_node_id == self_id` are re-published here — a
/// LEARNED entry (owned by a different node) is served by the DNS projection
/// but never re-announced: the owning node alone is responsible for keeping
/// its own entry alive over gossip, exactly like [`announce_addr_book`]'s
/// self-only re-announce discipline. Own tombstones are re-announced for the
/// same reason (a learned tombstone came from its owner; re-announcing it
/// here would be speaking for a node that isn't us).
///
/// The locks are only held to clone what's needed for the broadcast/persist
/// below, never across an `.await`.
async fn announce_service_book_v2<T: GossipTransport>(
    sync: &GossipSync<T>,
    service_book_v2: &Arc<Mutex<ServiceBookV2>>,
    service_tombstones_v2: &Arc<Mutex<ServiceTombstoneBook>>,
    lost_names_v2: &Arc<Mutex<LostNameMap>>,
    service_book_v2_file: &Path,
    self_id: &str,
) {
    let (own_entries, own_tombstones, snapshot) = {
        let book = service_book_v2.lock().expect("v2 service book poisoned");
        let tombstones = service_tombstones_v2.lock().expect("v2 service tombstones poisoned");
        let lost_names = lost_names_v2.lock().expect("v2 service lost-names poisoned");
        let own_entries: Vec<(String, ServiceEntryV2)> = book
            .iter()
            .filter(|(_, entry)| entry.owner_node_id == self_id)
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect();
        let own_tombstones: Vec<(String, ServiceTombstone)> = tombstones
            .iter()
            .filter(|(_, tombstone)| tombstone.owner_node_id == self_id)
            .map(|(name, tombstone)| (name.clone(), tombstone.clone()))
            .collect();
        let snapshot =
            ServiceStateV2 { book: book.clone(), tombstones: tombstones.clone(), lost_names: lost_names.clone() };
        (own_entries, own_tombstones, snapshot)
    };
    for (name, entry) in &own_entries {
        if let Err(e) = sync
            .publish(GossipMessage::ServicePublishV2 { name: name.clone(), entry: entry.clone() })
            .await
        {
            warn!(%name, "v2 service anti-entropy: re-broadcast publish failed: {e}");
        }
    }
    for (name, tombstone) in &own_tombstones {
        if let Err(e) = sync
            .publish(GossipMessage::ServiceUnpublishV2 {
                name: name.clone(),
                owner_node_id: tombstone.owner_node_id.clone(),
                hlc: tombstone.hlc.clone(),
            })
            .await
        {
            warn!(%name, "v2 service anti-entropy: re-broadcast unpublish failed: {e}");
        }
    }
    info!(
        own = own_entries.len(),
        tombstones = own_tombstones.len(),
        "v2 service anti-entropy: re-broadcast own entries"
    );
    persist_service_state_v2(&snapshot, service_book_v2_file);
}

/// Daemon-facing local publish (bead e21.2.3 FR25, e21.2.4 FR34) — the seam
/// S3.1's control API calls to claim/refresh a service name on behalf of
/// THIS node. Delegates the reserved-name/lost-to-a-peer/conflict-tracking
/// logic to [`publish_service_v2`] (lib-side, unit-tested there); on success
/// broadcasts the publish IMMEDIATELY (not deferred to the next anti-entropy
/// tick — the whole point of FR25's demo-responsiveness requirement) and
/// persists. No external IPC surface yet — S3.1 wires a control-API handler
/// to this function.
#[allow(dead_code, clippy::too_many_arguments)] // consumed by S3.1's control API; no caller yet in this story
async fn publish_service<T: GossipTransport>(
    sync: &GossipSync<T>,
    service_book_v2: &Arc<Mutex<ServiceBookV2>>,
    service_tombstones_v2: &Arc<Mutex<ServiceTombstoneBook>>,
    lost_names_v2: &Arc<Mutex<LostNameMap>>,
    service_book_v2_file: &Path,
    self_id: &str,
    name: &str,
    entry: ServiceEntryV2,
) -> Result<PublishOutcome, ServicePublishError> {
    let (outcome, snapshot) = {
        let mut book = service_book_v2.lock().expect("v2 service book poisoned");
        let tombstones = service_tombstones_v2.lock().expect("v2 service tombstones poisoned");
        let mut lost_names = lost_names_v2.lock().expect("v2 service lost-names poisoned");
        let outcome =
            publish_service_v2(&mut book, &tombstones, &mut lost_names, self_id, name, entry.clone())?;
        let snapshot =
            ServiceStateV2 { book: book.clone(), tombstones: tombstones.clone(), lost_names: lost_names.clone() };
        (outcome, snapshot)
    };
    if let Err(e) = sync
        .publish(GossipMessage::ServicePublishV2 { name: name.to_string(), entry })
        .await
    {
        warn!(%name, "local service publish: immediate broadcast failed: {e}");
    }
    persist_service_state_v2(&snapshot, service_book_v2_file);
    Ok(outcome)
}

/// Daemon-facing local unpublish, mirroring [`publish_service`]: applies the
/// tombstone via [`apply_service_unpublish_v2`], broadcasts immediately, and
/// persists. No external IPC surface yet — S3.1 wires a control-API handler
/// to this function.
#[allow(dead_code, clippy::too_many_arguments)] // consumed by S3.1's control API; no caller yet in this story
async fn unpublish_service<T: GossipTransport>(
    sync: &GossipSync<T>,
    service_book_v2: &Arc<Mutex<ServiceBookV2>>,
    service_tombstones_v2: &Arc<Mutex<ServiceTombstoneBook>>,
    lost_names_v2: &Arc<Mutex<LostNameMap>>,
    service_book_v2_file: &Path,
    name: &str,
    owner_node_id: &str,
    hlc: HLC,
) -> UnpublishOutcome {
    let (outcome, snapshot) = {
        let mut book = service_book_v2.lock().expect("v2 service book poisoned");
        let mut tombstones = service_tombstones_v2.lock().expect("v2 service tombstones poisoned");
        let outcome = apply_service_unpublish_v2(&mut book, &mut tombstones, name, owner_node_id, hlc.clone());
        let lost_names = lost_names_v2.lock().expect("v2 service lost-names poisoned");
        let snapshot =
            ServiceStateV2 { book: book.clone(), tombstones: tombstones.clone(), lost_names: lost_names.clone() };
        (outcome, snapshot)
    };
    if let Err(e) = sync
        .publish(GossipMessage::ServiceUnpublishV2 {
            name: name.to_string(),
            owner_node_id: owner_node_id.to_string(),
            hlc,
        })
        .await
    {
        warn!(%name, "local service unpublish: immediate broadcast failed: {e}");
    }
    persist_service_state_v2(&snapshot, service_book_v2_file);
    outcome
}

/// Assign this node's claimed /24 gateway address (`<net>.1/prefix`) to the local
/// client interface, giving babeld a concrete *connected* route to redistribute and
/// letting inbound mesh traffic for the /24 be delivered on-link. Replaces the old
/// container-gateway route hop (mjolnir-mesh-e4r): native OpenWrt has no veth
/// gateway — the router is itself on the client L2. Idempotent in effect: an
/// already-present address (EEXIST) is fine. Best-effort: a missing interface is
/// logged, not fatal.
/// The gateway address (`.1`) of a claimed client /24 — the address this
/// node assigns to its client interface (e4r) and answers as `hello.mesh`/
/// `id.mesh` once the claim lands (e21.1.2). Single source of truth for the
/// formula so the interface assignment and the DNS answer never diverge.
fn client_gateway_addr(subnet: Ipv4Net) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(subnet.network()) + 1)
}

#[cfg(target_os = "linux")]
async fn assign_client_addr(subnet: Ipv4Net, iface: &str) {
    use rtnetlink::new_connection;
    // The router takes `.1` of its claimed /24.
    let gw = client_gateway_addr(subnet);
    let prefix = subnet.prefix_len();
    let index = match std::fs::read_to_string(format!("/sys/class/net/{iface}/ifindex"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        Some(i) => i,
        None => {
            warn!(%subnet, iface, "client interface not found — cannot assign client subnet address");
            return;
        }
    };
    let (connection, handle, _) = match new_connection() {
        Ok(c) => c,
        Err(e) => {
            warn!(%subnet, "netlink connect for client address failed: {e}");
            return;
        }
    };
    tokio::spawn(connection);
    match handle
        .address()
        .add(index, std::net::IpAddr::V4(gw), prefix)
        .execute()
        .await
    {
        Ok(()) => {
            info!(%subnet, %gw, iface, "assigned client subnet gateway address (connected route for babeld)")
        }
        Err(e) => {
            warn!(%subnet, %gw, iface, "could not assign client address (may already exist): {e}")
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn assign_client_addr(_subnet: Ipv4Net, _iface: &str) {}

/// Remove this node's gateway address (`<net>.1/prefix`) from the client
/// interface — the inverse of [`assign_client_addr`], used when a claim is
/// lost in a conflict or released. Leaving the address up kept collision
/// losers answering on a /24 the mesh had routed elsewhere (mjolnir-mesh-eon).
/// Best-effort: an absent interface or address is logged, not fatal.
#[cfg(target_os = "linux")]
async fn retract_client_addr(subnet: Ipv4Net, iface: &str) {
    use futures_util::stream::TryStreamExt;
    use rtnetlink::new_connection;
    use rtnetlink::packet_route::address::AddressAttribute;
    let gw = client_gateway_addr(subnet);
    let prefix = subnet.prefix_len();
    let index = match std::fs::read_to_string(format!("/sys/class/net/{iface}/ifindex"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        Some(i) => i,
        None => {
            warn!(%subnet, iface, "client interface not found — cannot retract client subnet address");
            return;
        }
    };
    let (connection, handle, _) = match new_connection() {
        Ok(c) => c,
        Err(e) => {
            warn!(%subnet, "netlink connect for client address retraction failed: {e}");
            return;
        }
    };
    tokio::spawn(connection);
    let mut astream = handle.address().get().execute();
    while let Ok(Some(msg)) = astream.try_next().await {
        if msg.header.index != index || msg.header.prefix_len != prefix {
            continue;
        }
        let is_gw = msg.attributes.iter().any(|a| {
            matches!(
                a,
                AddressAttribute::Local(IpAddr::V4(v)) | AddressAttribute::Address(IpAddr::V4(v))
                    if *v == gw
            )
        });
        if !is_gw {
            continue;
        }
        match handle.address().del(msg).execute().await {
            Ok(()) => info!(%subnet, %gw, iface, "retracted client subnet gateway address"),
            Err(e) => warn!(%subnet, %gw, iface, "could not retract client address: {e}"),
        }
        return;
    }
    info!(%subnet, %gw, iface, "no client subnet gateway address to retract (already absent)");
}

#[cfg(not(target_os = "linux"))]
async fn retract_client_addr(_subnet: Ipv4Net, _iface: &str) {}

/// True when the lan config's PRIMARY (first) address is already the wanted
/// gateway CIDR — the idempotence check for [`reconcile_client_uci`]. Pure so
/// it's unit-tested below. `current` is `uci get network.lan.ipaddr` output:
/// space-joined list entries, or the bare stock `192.168.1.1`.
fn lan_uci_is_current(current: &str, want_primary: &str) -> bool {
    current.split_whitespace().next() == Some(want_primary)
}

/// Make the claimed /24 own the OpenWrt lan config (mjolnir-mesh-659):
/// `<gw>/24` becomes the FIRST (primary) entry of `network.lan.ipaddr`, so
/// dnsmasq's `dhcp.lan` pool (subnet-relative start/limit) serves the claimed
/// subnet instead of the stock 192.168.1.0/24. The stock subnet is identical
/// on every node, so clients it leases black-hole across the mesh (replies
/// exit the far node's own br-lan); the claimed /24 is what babel routes
/// fleet-wide. 192.168.1.1/24 is kept as a SECOND alias: dnsmasq stops
/// leasing from it, but the wired-recovery convention survives — a
/// statically-addressed laptop on the LAN port still reaches the node.
///
/// Best-effort and OpenWrt-only: skips silently when `uci` or a `lan`
/// interface is absent (RouterOS containers, desktops). Idempotent: no
/// network-reload churn when the primary is already the claimed address —
/// this runs on every claim publish, including anti-entropy-era re-claims.
#[cfg(target_os = "linux")]
async fn reconcile_client_uci(subnet: Ipv4Net) {
    use tokio::process::Command;
    let gw_cidr = format!(
        "{}/{}",
        Ipv4Addr::from(u32::from(subnet.network()) + 1),
        subnet.prefix_len()
    );
    // No uci binary → not OpenWrt; no network.lan → nothing to own.
    let lan_exists = Command::new("uci")
        .args(["-q", "get", "network.lan"])
        .output()
        .await;
    match lan_exists {
        Err(_) => return, // uci not present
        Ok(out) if !out.status.success() => return,
        Ok(_) => {}
    }
    let current = Command::new("uci")
        .args(["-q", "get", "network.lan.ipaddr"])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if lan_uci_is_current(&current, &gw_cidr) {
        return;
    }
    info!(%subnet, %gw_cidr, was = %current, "reconciling lan UCI: claimed /24 becomes primary (dnsmasq follows)");
    let script = format!(
        "uci -q delete network.lan.ipaddr; \
         uci -q delete network.lan.netmask; \
         uci add_list network.lan.ipaddr='{gw_cidr}'; \
         uci add_list network.lan.ipaddr='192.168.1.1/24'; \
         uci commit network && \
         /etc/init.d/network reload && \
         /etc/init.d/dnsmasq restart"
    );
    // Same lesson as babeld_service (qz9): a procd/ubus call can wedge — never
    // let one stall the claim manager. Network reload takes a few seconds
    // legitimately, so the cap is generous.
    let run = Command::new("sh").args(["-c", &script]).output();
    match tokio::time::timeout(Duration::from_secs(30), run).await {
        Ok(Ok(out)) if out.status.success() => {
            info!(%gw_cidr, "lan UCI reconciled — DHCP now serves the claimed /24 (192.168.1.1 kept as recovery alias)")
        }
        Ok(Ok(out)) => warn!(
            %gw_cidr,
            "lan UCI reconcile failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Ok(Err(e)) => warn!(%gw_cidr, "lan UCI reconcile could not run: {e}"),
        Err(_) => warn!(%gw_cidr, "lan UCI reconcile timed out after 30s"),
    }
}

#[cfg(not(target_os = "linux"))]
async fn reconcile_client_uci(_subnet: Ipv4Net) {}

/// Self-assign this node's derived IPv4 backhaul address (`10.254.0.0/16`, host
/// from the node id) to the shared-segment interface, so every node has a stable,
/// collision-free, DHCP-free underlay address in one shared /16. Peers are then
/// on-link to each other and iroh/mDNS discover + connect directly over the LAN
/// (mjolnir-mesh-4pk). IPv4 (not an IPv6 ULA) because iroh surfaces private IPv4
/// as a connection candidate and announces it over mDNS, but not IPv6 ULAs — see
/// the `iroh-lan-backhaul-findings` memory. Best-effort: an unreachable interface
/// or an already-present address is logged, not fatal — the node still runs.
///
/// Returns the resolved backhaul interface name (which may differ from the
/// configured `iface` — RouterOS doesn't name it `eth0` — via the sole-interface
/// fallback below). Callers use it as babel's wireless L2 interface
/// (mjolnir-mesh-auu). `None` means no usable interface was found.
#[cfg(target_os = "linux")]
async fn assign_backhaul_addr(iface: &str, addr: Ipv4Addr) -> Option<String> {
    use rtnetlink::new_connection;

    let prefix = mjolnir_mesh::tun::BACKHAUL_PREFIX_LEN;

    let (connection, handle, _) = match new_connection() {
        Ok(c) => c,
        Err(e) => {
            warn!(%addr, "netlink connect for backhaul address failed: {e}");
            return None;
        }
    };
    tokio::spawn(connection);

    // Resolve the backhaul interface from sysfs. RouterOS (a) brings the
    // container veth up a moment AFTER the process starts, and (b) does NOT name
    // it `eth0` like the plain Linux containers 4pk was validated on. So retry for
    // the startup race and be name-agnostic: prefer the configured name if it
    // appears, else fall back to the SOLE non-loopback interface — a fresh
    // container has just `lo` + the backhaul veth (the mj-peer-* TUNs don't exist
    // yet). The address must be assigned before iroh binds, so we wait here.
    let deadline = Instant::now() + Duration::from_secs(20);
    let (target, index) = loop {
        let candidates: Vec<String> = std::fs::read_dir("/sys/class/net")
            .map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                    .filter(|n| n != "lo" && !n.starts_with("mj-peer-"))
                    .collect()
            })
            .unwrap_or_default();
        let chosen = if candidates.iter().any(|n| n == iface) {
            Some(iface.to_string())
        } else if candidates.len() == 1 {
            Some(candidates[0].clone())
        } else {
            None
        };
        if let Some(name) = chosen {
            // ifindex straight from sysfs — avoids the netlink "No such device" path.
            if let Some(idx) = std::fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
            {
                break (name, idx);
            }
        }
        if Instant::now() >= deadline {
            warn!(
                configured = iface, available = ?candidates,
                "no backhaul interface found — is the container bridged onto the shared L2 \
                 segment? set --backhaul-iface to one of the available interfaces"
            );
            return None;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    match handle
        .address()
        .add(index, std::net::IpAddr::V4(addr), prefix)
        .execute()
        .await
    {
        Ok(()) => info!(
            %addr, iface = %target, prefix,
            "assigned IPv4 backhaul address — peers discover this node here via mDNS"
        ),
        Err(e) => {
            warn!(%addr, iface = %target, "could not assign backhaul address (may already exist): {e}")
        }
    }
    // The interface exists either way (the address may already be present); hand
    // its resolved name back so babel can route over it as the wireless L2 iface.
    Some(target)
}

#[cfg(not(target_os = "linux"))]
async fn assign_backhaul_addr(_iface: &str, _addr: Ipv4Addr) -> Option<String> {
    None
}

/// Enable IPv4 forwarding in this (container) network namespace so the kernel
/// routes client traffic between the TUN tunnels and the veth/bridge. Required
/// for cross-mesh client transit (the container half of mjolnir-mesh-ag3); the
/// RouterOS-side routes live in deploy/mikrotik/client-routing.rsc.
#[cfg(target_os = "linux")]
fn enable_ip_forwarding() {
    match std::fs::write("/proc/sys/net/ipv4/ip_forward", "1") {
        Ok(()) => info!("enabled net.ipv4.ip_forward (client transit)"),
        Err(e) => warn!("could not enable ip_forward — cross-mesh client transit needs it: {e}"),
    }
}

#[cfg(not(target_os = "linux"))]
fn enable_ip_forwarding() {}

// --- babeld supervision (mjolnir-mesh-83k) -------------------------------

/// Run a procd action (`start`/`stop`/`restart`/`enable`) on the `mjolnir-babeld`
/// service. procd owns the babeld PROCESS lifecycle; meshd only renders the config
/// and triggers reloads through here (mjolnir-mesh-m8t). Returns whether it
/// succeeded. Best-effort: a failure is logged, not fatal.
#[cfg(target_os = "linux")]
async fn babeld_service(action: &str) -> bool {
    let mut cmd = tokio::process::Command::new("/etc/init.d/mjolnir-babeld");
    cmd.arg(action);
    // On timeout the future is dropped — without this the rc script keeps
    // running detached and its stop/start completes LATER, racing whatever
    // action the reconciler issues next (a stray-babeld source, nrr).
    cmd.kill_on_drop(true);
    // Hard 10s timeout: a procd/ubus service call has wedged under rapid
    // invocation (qz9). Never let one stall the reconciler — bail and let procd
    // (which independently respawns + file-watches the config) sort itself out.
    match tokio::time::timeout(Duration::from_secs(10), cmd.status()).await {
        Ok(Ok(s)) if s.success() => true,
        Ok(Ok(s)) => {
            warn!(action, code = ?s.code(), "mjolnir-babeld action failed");
            false
        }
        Ok(Err(e)) => {
            warn!(action, "could not run /etc/init.d/mjolnir-babeld: {e}");
            false
        }
        Err(_) => {
            warn!(action, "mjolnir-babeld action timed out after 10s — leaving it to procd");
            false
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn babeld_service(_action: &str) -> bool {
    false
}

/// Reconcile babeld's CONFIG against live mesh state (mjolnir-mesh-m8t). Every few
/// seconds it renders babeld.conf from the current tunnel interfaces
/// ([`TunnelRegistry`]) and our local subnet claim ([`ClaimStore`]). procd owns the
/// babeld process via the `mjolnir-babeld` service; this loop only enables/starts it
/// once there's an interface to route over, asks procd to restart it when the
/// rendered config changes, and stops it when no interface remains. babeld being
/// absent is non-fatal — routing is disabled but the data plane keeps running.
async fn babel_reconciler(
    registry: TunnelRegistry,
    claims: ClaimStore,
    self_id: String,
    config_path: PathBuf,
    l2_backhaul: Option<String>,
    gateway: bool,
) {
    // Debounce window: wait this long for the mesh state to settle before
    // (re)writing babeld.conf, so a convergence burst doesn't thrash babeld (qz9).
    const BABEL_SETTLE: Duration = Duration::from_secs(2);

    // The shared-L2 backhaul interface, if any, is a permanent wireless-type
    // babel link (mjolnir-mesh-auu) — present from startup, so babeld runs
    // continuously instead of flapping with the per-peer tunnels.
    let l2_refs: Vec<&str> = l2_backhaul.as_deref().into_iter().collect();
    if let Some(parent) = config_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!("could not create babeld config dir {}: {e}", parent.display());
    }

    let mut started = false;
    let mut last_rendered: Option<String> = None;
    loop {
        // Snapshot the live tunnel interfaces and our own claimed subnet.
        let mut ifaces: Vec<String> = {
            let r = registry.lock().expect("registry poisoned");
            r.values().filter(|s| !s.is_empty()).cloned().collect()
        };
        ifaces.sort();
        let local_subnet: Option<Ipv4Net> = {
            let c = claims.lock().expect("claim store poisoned");
            c.values()
                .filter(|claim| claim.owner_node_id == self_id)
                .find_map(|claim| match claim.cidr {
                    // Skip backhaul /32 claims (pt9) — babel redistributes the
                    // client /24, never the backhaul address claim.
                    IpNet::V4(n) if !mjolnir_mesh::tun::in_backhaul_block(&n) => Some(n),
                    _ => None,
                })
        };

        let iface_refs: Vec<&str> = ifaces.iter().map(String::as_str).collect();
        let inputs = BabelConfigInputs::new(local_subnet, &iface_refs)
            .l2_interfaces(&l2_refs)
            .gateway(gateway);
        let conf = render_babeld_conf(&inputs);

        // Debounce (qz9): when the desired config changes, let the mesh state
        // settle before writing — a tunnel coming up and then a subnet claim
        // landing are two changes seconds apart, and rewriting on each one makes
        // procd thrash babeld (and historically meshd wedged driving those
        // restarts). Wait one settle window, re-render, and only reconcile a
        // config that held steady across it.
        if last_rendered.as_deref() != Some(conf.as_str()) {
            last_rendered = Some(conf);
            tokio::time::sleep(BABEL_SETTLE).await;
            continue;
        }

        // babeld needs at least one interface — the L2 backhaul (if present) or a
        // live tunnel. With an L2 backhaul this is always true, so babeld runs
        // continuously rather than flapping with the tunnel set.
        let have_ifaces = !ifaces.is_empty() || !l2_refs.is_empty();

        match write_atomic_if_changed(&config_path, &conf) {
            Ok(_changed) => {
                // procd owns babeld restarts now: the mjolnir-babeld init watches
                // this file (`procd_set_param file`) and restarts babeld whenever
                // it changes. meshd only (a) starts babeld once when the first
                // valid config is ready and (b) stops it on zero interfaces
                // (dynamic --internet mode; LAN keeps the L2 backhaul permanently).
                // meshd no longer drives per-change restarts — that synchronous
                // procd loop wedged the daemon (qz9).
                if !have_ifaces {
                    if started {
                        warn!("no live interfaces — stopping babeld until one returns");
                        babeld_service("stop").await;
                        started = false;
                    }
                } else if !started {
                    // Enable (survive reboot) then start it once; from here procd's
                    // file-watch handles every config-change restart.
                    babeld_service("enable").await;
                    if babeld_service("restart").await {
                        started = true;
                        let count = ifaces.len() + l2_refs.len();
                        info!(config = %config_path.display(), ifaces = count, "babeld started (procd: mjolnir-babeld); procd watches the config from here");
                    }
                }
            }
            Err(e) => warn!("failed to write babeld config {}: {e}", config_path.display()),
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Live per-peer tunnel registry: maps each connected peer to its TUN interface
/// name. Shared between the accept handler and the per-peer dialer tasks. The
/// babeld layer (mjolnir-mesh-83k) reads this to learn the live tunnel set; for
/// now it also enforces the one-tunnel-per-peer invariant (the per-pair /31 and
/// `mj-peer-<id>` name are deterministic, so a second tunnel for the same peer
/// would collide on the interface name).
type TunnelRegistry = Arc<Mutex<HashMap<EndpointId, String>>>;

/// Bring up a per-peer /31 TUN over `conn`, register it, and hold it open until
/// the connection closes. Shared by the inbound (accept) and outbound (dial)
/// paths so both enforce the same one-tunnel-per-peer invariant and feed the
/// same registry. Returns when the tunnel tears down.
async fn serve_tunnel(conn: Connection, self_id: &str, registry: &TunnelRegistry) -> Result<()> {
    let peer = conn.remote_id();
    let peer_str = peer.to_string();
    let (self_addr, peer_addr) = mjolnir_mesh::tun::pick_link_31(self_id, &peer_str);

    // Atomically reserve this peer's slot. If one already exists, refuse the new
    // connection rather than collide on the deterministic interface name. The
    // empty-string sentinel is replaced with the real iface name once it's up.
    {
        let mut reg = registry.lock().expect("registry poisoned");
        if reg.contains_key(&peer) {
            drop(reg);
            warn!(%peer, "already have a tunnel for this peer — refusing duplicate");
            conn.close(2u32.into(), b"duplicate tunnel");
            return Ok(());
        }
        reg.insert(peer, String::new());
    }

    let tunnel = match spawn_tunnel(
        short_id(&peer_str),
        self_addr,
        peer_addr,
        IrohDatagramConn { conn: conn.clone() },
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            registry.lock().expect("registry poisoned").remove(&peer);
            conn.close(1u32.into(), b"tunnel setup failed");
            return Err(anyhow::anyhow!("bringing up tunnel for {peer}: {e}"));
        }
    };

    let iface = tunnel.iface_name.clone();
    registry
        .lock()
        .expect("registry poisoned")
        .insert(peer, iface.clone());
    info!(%iface, %self_addr, %peer_addr, %peer, "tunnel up");
    spawn_path_logger(conn.clone(), peer);
    spawn_udp_echo(self_addr);

    // Hold the tunnel open until the connection closes, then deregister.
    let reason = conn.closed().await;
    info!(%peer, %iface, ?reason, "tunnel closed");
    registry.lock().expect("registry poisoned").remove(&peer);
    drop_tunnel(tunnel);
    Ok(())
}

/// Log this connection's selected transmission path whenever it changes —
/// `DIRECT` (hole-punched / same-LAN) vs `RELAY` (n0 relay fallback) — plus the
/// remote address, RTT, path count, and the negotiated max datagram size.
///
/// This is the observability that mjolnir-mesh-67h was missing. A relay-only or
/// half-open path manifests downstream as asymmetric babeld costs (one side's
/// IHUs never arrive → txcost 65535), forcing the operator to *infer* the
/// transport from routing metrics. With this, the path type is a single log
/// line per peer: a tunnel still on `RELAY` after warmup is the smoking gun
/// (relay-only loss / a peer that never published a direct addr), and a flip to
/// `DIRECT` confirms a healthy hole-punched path. `max_datagram=None` means
/// datagrams can't flow on this path at all — an even earlier failure signal.
/// The task ends when the connection closes.
fn spawn_path_logger(conn: Connection, peer: EndpointId) {
    use futures_lite::StreamExt;
    tokio::spawn(async move {
        let mut stream = conn.paths_stream();
        while let Some(paths) = stream.next().await {
            match paths.iter().find(|p| p.is_selected()) {
                Some(p) => {
                    let kind = if p.is_relay() { "RELAY" } else { "DIRECT" };
                    info!(
                        %peer,
                        kind,
                        remote = ?p.remote_addr(),
                        rtt = ?p.rtt(),
                        paths = paths.len(),
                        max_datagram = ?conn.max_datagram_size(),
                        "tunnel path",
                    );
                }
                None => warn!(
                    %peer,
                    paths = paths.len(),
                    "tunnel has no selected path — datagrams cannot flow",
                ),
            }
        }
    });
}

/// Explicit drop helper — makes the teardown point obvious at call sites and
/// documents that dropping a [`Tunnel`] aborts its encap tasks and releases the
/// TUN fd (so the kernel removes the interface).
fn drop_tunnel(tunnel: Tunnel) {
    drop(tunnel);
}

/// iroh protocol handler that brings up a per-peer TUN tunnel on each accepted
/// connection, registering it in the shared [`TunnelRegistry`].
#[derive(Clone, Debug)]
struct TunnelHandler {
    self_id: String,
    registry: TunnelRegistry,
}

impl ProtocolHandler for TunnelHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = serve_tunnel(conn, &self.self_id, &self.registry).await {
            warn!("accepted tunnel ended with error: {e}");
        }
        Ok(())
    }
}

// ===== buw single-overlay-TUN data plane (mjolnir-mesh-buw.3/4/5) ===========
// Opt-in via `--overlay`. Replaces the per-peer TUN registry + tunnels with ONE
// `mjolnir0` multiplexing every peer. The per-peer path above is untouched.

/// buw connection manager (mjolnir-mesh-buw.3): the live iroh [`Connection`] for
/// each peer, indexed BOTH by node id (lifecycle / dedup) and by the peer's
/// derived overlay address `10.254.x` (data-plane demux: FIB next-hop -> conn).
/// Decoupled from any interface — a dropped connection removes a map entry, not
/// a babel interface, so babeld's config never churns (the qz9 fix).
#[derive(Clone, Default)]
struct ConnManager {
    inner: Arc<Mutex<ConnManagerInner>>,
}

#[derive(Default)]
struct ConnManagerInner {
    by_peer: HashMap<EndpointId, Connection>,
    by_addr: HashMap<Ipv4Addr, Connection>,
}

impl std::fmt::Debug for ConnManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.lock().expect("connmgr poisoned");
        write!(f, "ConnManager({} conns)", g.by_peer.len())
    }
}

impl ConnManager {
    fn new() -> Self {
        Self::default()
    }

    /// The peer's derived overlay address (`10.254.x`) — the demux key, matching
    /// the address `spawn_overlay_tun` assigns and babeld's next hops.
    fn addr_of(peer: &EndpointId) -> Ipv4Addr {
        mjolnir_mesh::tun::backhaul_addr(&peer.to_string())
    }

    /// Register `conn` for `peer`, updating BOTH indexes atomically. Returns
    /// `false` if a connection for this peer already exists (caller refuses the
    /// duplicate) — the one-connection-per-peer invariant.
    fn register(&self, peer: EndpointId, conn: Connection) -> bool {
        let addr = Self::addr_of(&peer);
        let mut g = self.inner.lock().expect("connmgr poisoned");
        if g.by_peer.contains_key(&peer) {
            return false;
        }
        g.by_peer.insert(peer, conn.clone());
        g.by_addr.insert(addr, conn);
        true
    }

    /// Remove `peer` from both indexes. No-op if absent.
    fn deregister(&self, peer: &EndpointId) {
        let addr = Self::addr_of(peer);
        let mut g = self.inner.lock().expect("connmgr poisoned");
        g.by_peer.remove(peer);
        g.by_addr.remove(&addr);
    }

    /// The connection whose peer owns overlay address `addr`, if connected.
    fn by_addr(&self, addr: Ipv4Addr) -> Option<Connection> {
        self.inner
            .lock()
            .expect("connmgr poisoned")
            .by_addr
            .get(&addr)
            .cloned()
    }

    /// Snapshot of every live connection (for multicast flooding).
    fn all(&self) -> Vec<Connection> {
        self.inner
            .lock()
            .expect("connmgr poisoned")
            .by_peer
            .values()
            .cloned()
            .collect()
    }
}

/// Resolves a unicast overlay packet to the peer connection that should carry
/// it (mjolnir-mesh-buw.4): v4 client/overlay traffic via the FIB (or on-link
/// `10.254/16`), v6 babel link-local (`fe80::X`) via the `fe80 <-> 10.254`
/// derivation, then the connection manager's addr index -> [`Connection`].
#[derive(Clone)]
struct OverlayRouter {
    fib: Arc<Mutex<Fib>>,
    conns: ConnManager,
}

impl OverlayRouter {
    /// The `10.254.x` next hop for a unicast packet, or `None` if unroutable.
    fn next_hop(&self, packet: &[u8]) -> Option<Ipv4Addr> {
        match packet.first()? >> 4 {
            4 => {
                let d: [u8; 4] = packet.get(16..20)?.try_into().ok()?;
                let dest = Ipv4Addr::from(d);
                // Overlay-block (10.254.0.0/16) unicast is NEVER forwarded across
                // the overlay: those are the mjolnir0 interface/neighbour
                // addresses, not a data path. Client traffic routes by its OWN
                // destination via the FIB — the 10.254.x next hop is resolved from
                // the FIB entry, never carried as a packet destination (true for
                // single- and multi-hop). Dropping the block (return None) stops
                // iroh — which advertises mjolnir0's own 10.254.x as a candidate
                // direct address, with no public API to suppress it (buw.8) — from
                // forming a bogus, fragile iroh-over-overlay path to a peer's
                // overlay address. The underlay reaches peers via iroh's native
                // discovery (mDNS/relay), not the overlay.
                if dest.octets()[..2] == [10, 254] {
                    None
                } else {
                    self.fib.lock().expect("fib poisoned").lookup(dest)
                }
            }
            6 => {
                let d: [u8; 16] = packet.get(24..40)?.try_into().ok()?;
                let seg = std::net::Ipv6Addr::from(d).segments();
                // fe80::X (babel IHU to a neighbour) -> next-hop 10.254.X, the
                // reverse of tun::iface::overlay_link_local.
                if seg[0] == 0xfe80 {
                    let host = seg[7];
                    Some(Ipv4Addr::new(10, 254, (host >> 8) as u8, (host & 0xff) as u8))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl UnicastRouter<IrohDatagramConn> for OverlayRouter {
    fn resolve(&self, packet: &[u8]) -> Option<IrohDatagramConn> {
        let nh = self.next_hop(packet)?;
        self.conns.by_addr(nh).map(|conn| IrohDatagramConn { conn })
    }
}

/// The mjolnir0 reader: read each IP packet off the overlay TUN and forward it —
/// multicast (babel Hello) flooded to EVERY live peer (emulation), unicast
/// routed to the ONE peer `router` resolves (or dropped). Mirrors
/// [`mjolnir_mesh::tun::spawn_overlay_routed`] (the tested reference) but with a
/// LIVE flood set from the connection manager, since peers join/leave at runtime.
async fn overlay_reader<R>(mut reader: R, conns: ConnManager, router: OverlayRouter, mtu: usize)
where
    R: tokio::io::AsyncRead + Unpin + Send,
{
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; mtu];
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                warn!("overlay reader: TUN read error: {e}");
                break;
            }
        };
        let pkt = Bytes::copy_from_slice(&buf[..n]);
        match classify(&pkt) {
            Some(OverlayDest::Multicast) => {
                for conn in conns.all() {
                    let _ = IrohDatagramConn { conn }.send_datagram(pkt.clone()).await;
                }
            }
            Some(OverlayDest::Unicast) => {
                if let Some(dc) = router.resolve(&pkt) {
                    let _ = dc.send_datagram(pkt).await;
                }
                // else: unroutable — dropped (no flood), so a transit node can't loop.
            }
            None => {}
        }
    }
}

/// Mirror babeld's kernel routes on `iface` (mjolnir0) into `fib` by polling
/// `ip -4 route show dev <iface>` (route-event subscription is a later
/// optimization). A `dest/len via 10.254.x` line becomes `fib[dest/len] =
/// 10.254.x` — the demux the overlay reader needs, since the raw packet carries
/// only the client dest, not babeld's next hop.
async fn fib_mirror(iface: String, fib: Arc<Mutex<Fib>>) {
    loop {
        let out = tokio::process::Command::new("ip")
            .args(["-4", "route", "show", "dev", &iface])
            .output()
            .await;
        if let Ok(out) = out {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut next = Fib::new();
            for line in text.lines() {
                let toks: Vec<&str> = line.split_whitespace().collect();
                let Some(prefix) = toks.first() else { continue };
                let gw = toks
                    .iter()
                    .position(|t| *t == "via")
                    .and_then(|i| toks.get(i + 1))
                    .and_then(|s| s.parse::<Ipv4Addr>().ok());
                if let (Some((net, len)), Some(gw)) = (parse_cidr(prefix), gw) {
                    next.upsert(net, len, gw);
                }
            }
            *fib.lock().expect("fib poisoned") = next;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Parse `a.b.c.d/len` (or a bare host = /32) into `(network, len)`.
fn parse_cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
    match s.split_once('/') {
        Some((ip, len)) => Some((ip.parse().ok()?, len.parse().ok()?)),
        None => Some((s.parse().ok()?, 32)),
    }
}

/// Overlay analogue of [`serve_tunnel`]: register the peer's connection, pump its
/// inbound datagrams onto the shared mjolnir0 writer via `inbound`, and hold
/// until the connection closes — then deregister. NO per-peer interface.
async fn serve_overlay_conn(
    conn: Connection,
    conns: ConnManager,
    inbound: tokio::sync::mpsc::Sender<Bytes>,
) -> Result<()> {
    let peer = conn.remote_id();
    if !conns.register(peer, conn.clone()) {
        warn!(%peer, "already have a connection for this peer — refusing duplicate");
        conn.close(2u32.into(), b"duplicate connection");
        return Ok(());
    }
    info!(%peer, addr = %ConnManager::addr_of(&peer), "overlay peer connected");
    spawn_path_logger(conn.clone(), peer);

    // Receiver: each inbound datagram from this peer -> the single mjolnir0 writer.
    let recv = {
        let conn = conn.clone();
        tokio::spawn(async move {
            // Ok = a datagram; Err = connection closed (loop ends).
            while let Ok(pkt) = conn.read_datagram().await {
                if inbound.send(pkt).await.is_err() {
                    break; // writer gone
                }
            }
        })
    };

    let reason = conn.closed().await;
    info!(%peer, ?reason, "overlay peer disconnected");
    recv.abort();
    conns.deregister(&peer);
    Ok(())
}

/// Overlay analogue of [`connector_loop`]: dial a peer and serve the connection
/// into the connection manager, redialing with backoff until aborted.
async fn connector_loop_overlay(
    endpoint: Endpoint,
    addr: EndpointAddr,
    conns: ConnManager,
    inbound: tokio::sync::mpsc::Sender<Bytes>,
    label: Option<String>,
) {
    let peer = addr.id;
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);
    loop {
        info!(%peer, label = ?label, "dialing peer (overlay)");
        match endpoint.connect(addr.clone(), TUN_ALPN).await {
            Ok(conn) => {
                backoff = Duration::from_secs(1);
                if let Err(e) = serve_overlay_conn(conn, conns.clone(), inbound.clone()).await {
                    warn!(%peer, "overlay connection ended with error: {e}");
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                warn!(%peer, ?backoff, "dial failed: {e}; retrying after backoff");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }
        }
    }
}

/// iroh handler that serves each accepted connection into the overlay connection
/// manager (the overlay analogue of [`TunnelHandler`]).
#[derive(Clone, Debug)]
struct OverlayHandler {
    conns: ConnManager,
    inbound: tokio::sync::mpsc::Sender<Bytes>,
}

impl ProtocolHandler for OverlayHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = serve_overlay_conn(conn, self.conns.clone(), self.inbound.clone()).await {
            warn!("accepted overlay connection ended with error: {e}");
        }
        Ok(())
    }
}

/// Overlay babeld reconciler (mjolnir-mesh-buw.5): renders the SINGLE static
/// `mjolnir0` config from the claim store. mjolnir0 is always up, so babeld runs
/// continuously and the config only changes when our claimed /24 changes — no
/// per-peer churn (qz9 dissolved by construction, no debounce needed).
async fn babel_reconciler_overlay(
    claims: ClaimStore,
    self_id: String,
    config_path: PathBuf,
    gateway: bool,
) {
    if let Some(parent) = config_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!("could not create babeld config dir {}: {e}", parent.display());
    }
    let mut started = false;
    let mut last_rendered: Option<String> = None;
    loop {
        let local_subnet: Option<Ipv4Net> = {
            let c = claims.lock().expect("claim store poisoned");
            c.values()
                .filter(|claim| claim.owner_node_id == self_id)
                .find_map(|claim| match claim.cidr {
                    // Skip backhaul /32 claims (pt9) — babel redistributes the
                    // client /24, never the backhaul address claim.
                    IpNet::V4(n) if !mjolnir_mesh::tun::in_backhaul_block(&n) => Some(n),
                    _ => None,
                })
        };
        let conf =
            render_overlay_babeld_conf(OVERLAY_IFACE, local_subnet, OverlayRtt::default(), gateway);
        if last_rendered.as_deref() != Some(conf.as_str()) {
            match write_atomic_if_changed(&config_path, &conf) {
                Ok(_) => {
                    if !started {
                        babeld_service("enable").await;
                        if babeld_service("restart").await {
                            started = true;
                            info!(config = %config_path.display(), iface = OVERLAY_IFACE, "babeld started (overlay: single mjolnir0)");
                        }
                    }
                    last_rendered = Some(conf);
                }
                Err(e) => warn!("failed to write overlay babeld config {}: {e}", config_path.display()),
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Echo any UDP datagram back to its sender, bound to `bind_ip:TUN_PROBE_PORT`
/// (the TUN /31 address). Lets a peer prove the tunnel carries real IP traffic.
fn spawn_udp_echo(bind_ip: Ipv4Addr) {
    tokio::spawn(async move {
        let sock = match tokio::net::UdpSocket::bind((bind_ip, TUN_PROBE_PORT)).await {
            Ok(s) => s,
            Err(e) => {
                warn!(%bind_ip, "udp echo bind failed: {e}");
                return;
            }
        };
        info!(%bind_ip, port = TUN_PROBE_PORT, "udp echo up on tunnel address");
        let mut buf = [0u8; 1500];
        loop {
            match sock.recv_from(&mut buf).await {
                Ok((n, from)) => {
                    let _ = sock.send_to(&buf[..n], from).await;
                }
                Err(e) => {
                    warn!("udp echo recv error: {e}");
                    break;
                }
            }
        }
    });
}

/// Wait (bounded) for the connection to acquire a direct (hole-punched) path in
/// addition to the relay. Returns `true` if a direct path was established within
/// `timeout`, `false` if it stayed relay-only. A relay-only path forwards
/// unreliable datagrams best-effort and drops heavily under load, so the data
/// plane is far lossier before this returns true.
async fn wait_for_direct_path(conn: &Connection, timeout: Duration) -> bool {
    // Poll path snapshots rather than the path stream: the stream needs
    // `StreamExt` (futures-util), which is a Linux-only dep here, whereas
    // `paths()` is a plain snapshot that works on every platform.
    let deadline = Instant::now() + timeout;
    loop {
        if conn.paths().iter().any(|p| p.is_ip()) {
            return true;
        }
        if Instant::now() >= deadline {
            warn!(
                ?timeout,
                "no direct path within timeout — still relay-only; datagram loss \
                 will be high until a hole-punch succeeds"
            );
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Log a one-line summary of every QUIC path on the connection (relay vs direct,
/// selected, RTT) plus the current datagram-size ceiling. This is the diagnostic
/// that turns a bare "1/5 probes crossed" into "1/5 on a relay-only path".
fn log_conn_paths(conn: &Connection) {
    let paths = conn.paths();
    for p in paths.iter() {
        let kind = if p.is_relay() { "relay" } else { "direct" };
        info!(
            kind,
            selected = p.is_selected(),
            remote = %p.remote_addr(),
            rtt = ?p.rtt(),
            "tunnel path"
        );
    }
    info!(
        max_datagram_size = ?conn.max_datagram_size(),
        path_count = paths.len(),
        "tunnel connection datagram ceiling"
    );
}

/// Send a few UDP probes to `peer_ip:TUN_PROBE_PORT` over the tunnel and report
/// round-trip results. Success proves real IP traffic flows across the mesh.
/// `direct_path` records whether a hole-punched path was up, so the headline
/// makes relay-only loss legible rather than mysterious.
async fn probe_peer(peer_ip: Ipv4Addr, direct_path: bool) {
    let sock = match tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await {
        Ok(s) => s,
        Err(e) => {
            warn!("probe socket bind failed: {e}");
            return;
        }
    };
    let mut ok = 0u32;
    for i in 1..=5u32 {
        let payload = format!("mjolnir-tun-ping-{i}");
        let start = Instant::now();
        if let Err(e) = sock.send_to(payload.as_bytes(), (peer_ip, TUN_PROBE_PORT)).await {
            warn!("probe {i} send failed: {e}");
            continue;
        }
        let mut buf = [0u8; 256];
        match tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf)).await {
            Ok(Ok((n, _))) if &buf[..n] == payload.as_bytes() => {
                ok += 1;
                println!("tunnel ping {i}: reply from {peer_ip} in {:?}", start.elapsed());
            }
            Ok(Ok((n, _))) => println!("tunnel ping {i}: unexpected {n}-byte reply"),
            Ok(Err(e)) => warn!("probe {i} recv error: {e}"),
            Err(_) => println!("tunnel ping {i}: TIMEOUT (no reply across tunnel)"),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let path = if direct_path { "direct path" } else { "RELAY-ONLY path (lossy)" };
    println!(
        "tunnel reachability: {ok}/5 replies over {path} — {}",
        if ok > 0 { "DATA PLANE WORKS" } else { "no traffic crossed" }
    );
}

/// Probe TUN-device creation — the gating check for running the L3 data plane
/// inside a RouterOS container (needs /dev/net/tun + CAP_NET_ADMIN).
async fn run_tun_test() -> Result<()> {
    use mjolnir_mesh::tun::PeerInterface;
    use std::net::Ipv4Addr;

    // Throwaway /31 in the reserved link block.
    let self_addr = Ipv4Addr::new(10, 255, 0, 0);
    let peer_addr = Ipv4Addr::new(10, 255, 0, 1);

    info!("tun-test: attempting to create a TUN device…");
    match PeerInterface::create("tuntest0", self_addr, peer_addr).await {
        Ok(iface) => {
            println!(
                "TUN OK: created {} ({} <-> {})",
                iface.name(),
                iface.self_addr(),
                iface.peer_addr()
            );
            match iface.close().await {
                Ok(()) => println!("TUN teardown OK — the L3 data plane is viable here"),
                Err(e) => println!("TUN created but teardown failed: {e}"),
            }
            Ok(())
        }
        Err(e) => {
            println!("TUN FAILED: {e}");
            anyhow::bail!("tun-test failed: {e}")
        }
    }
}

/// QUIC transport config shared by both endpoint flavours (LAN and N0).
///
/// Only overrides the connection idle timeout ([`TUNNEL_MAX_IDLE`]); every
/// other knob — multipath (on, 8 paths), 5s keep-alive, 15s per-path idle —
/// keeps iroh's default. Applied to BOTH the dial and accept sides, since
/// `connect()` and the protocol router both read the endpoint's static
/// transport config (mjolnir-mesh-auu).
fn tunnel_transport_config() -> iroh::endpoint::QuicTransportConfig {
    iroh::endpoint::QuicTransportConfig::builder()
        .max_idle_timeout(Some(
            TUNNEL_MAX_IDLE
                .try_into()
                .expect("TUNNEL_MAX_IDLE is a valid QUIC idle timeout"),
        ))
        .build()
}

/// Build an iroh endpoint with a persisted (or ephemeral) identity. Relays are
/// on by default (they provide NAT traversal off-LAN); `--no-relay` forces
/// direct/LAN-only, and `--bind` pins the socket address.
async fn build_endpoint(
    secret: SecretKey,
    no_relay: bool,
    bind: Option<SocketAddr>,
    lan: bool,
    relays: &[String],
) -> Result<Endpoint> {
    if lan {
        // LAN-direct: start from the Minimal preset (crypto provider only, no
        // pkarr/n0-DNS publishing, so no internet dependency and no DNS spam),
        // relays off, and add ONLY mDNS address lookup for same-network peers.
        //
        // `bind` carries the single backhaul address in mesh mode (auu): pinning
        // the socket to it advertises exactly one reachable addr, avoiding the
        // multi-candidate-path prune that killed the tunnel. If that addr isn't
        // on an interface yet (the backhaul assign raced or failed), fall back to
        // all-interfaces rather than crash-loop the daemon.
        if let Some(addr) = bind {
            let attempt = Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .secret_key(secret.clone())
                .transport_config(tunnel_transport_config())
                .address_lookup(MdnsAddressLookup::builder())
                .bind_addr(addr)
                .context("invalid bind address")?;
            match attempt.bind().await {
                Ok(ep) => return Ok(ep),
                Err(e) => warn!(
                    %addr,
                    "binding iroh to the backhaul address failed ({e}); \
                     falling back to all interfaces",
                ),
            }
        }
        return Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .secret_key(secret)
            .transport_config(tunnel_transport_config())
            .address_lookup(MdnsAddressLookup::builder())
            .bind()
            .await
            .context("failed to bind iroh endpoint");
    }

    let relay_mode = if no_relay {
        RelayMode::Disabled
    } else if !relays.is_empty() {
        let urls = relays
            .iter()
            .map(|s| s.parse::<RelayUrl>())
            .collect::<Result<Vec<_>, _>>()
            .context("invalid --relay URL")?;
        RelayMode::custom(urls)
    } else {
        // iroh 0.96's RelayMode::Default points at the flaky `iroh-canary` test
        // network; Staging uses real n0 relays on relay.iroh.network.
        RelayMode::Staging
    };

    // N0 preset: publish to pkarr + resolve via n0 DNS (the internet path);
    // relay_mode below overrides the preset's default relay choice.
    //
    // ALSO add mDNS address lookup (the same swarm discovery used by `--lan`).
    // The N0 preset only knows pkarr+DNS, so a node whose pkarr publish fails —
    // e.g. a RouterOS container with no/limited internet egress — advertises NO
    // direct address and is reachable only over the lossy n0 relay, even when
    // its peer sits on the same physical LAN. That relay-only path is what made
    // the two-router tunnel asymmetric (mjolnir-mesh-67h): one side's IHUs never
    // arrived, so babeld saw txcost=65535 and routed nothing. mDNS advertises +
    // resolves direct LAN socket addresses with no relay or internet, so same-LAN
    // peers form a direct path regardless of pkarr — relay stays as the off-LAN
    // fallback, pkarr/DNS as global discovery. Best of all worlds, additive only.
    let mut builder = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .transport_config(tunnel_transport_config())
        .address_lookup(MdnsAddressLookup::builder())
        .relay_mode(relay_mode);
    if let Some(addr) = bind {
        builder = builder.bind_addr(addr).context("invalid --bind address")?;
    }
    builder.bind().await.context("failed to bind iroh endpoint")
}

/// Wait until the endpoint has at least one publishable address. With relays
/// on, also wait for the home relay so the blob is dialable off-LAN.
async fn wait_until_addressable(endpoint: &Endpoint, no_relay: bool) {
    if !no_relay {
        // home-relay handshake; bounded so we don't hang forever if relays are
        // unreachable (e.g. offline) — direct addrs may still suffice.
        let _ = tokio::time::timeout(Duration::from_secs(5), endpoint.online()).await;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while endpoint.addr().is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    info!(addr = ?endpoint.addr(), "endpoint addressable");
    check_reachability(endpoint, no_relay);
}

/// One loud, actionable line about whether this node is reachable by peers —
/// instead of leaving the operator to infer it from buried pkarr/DNS spam.
/// A node with no relay and only private/loopback addresses has an unroutable
/// address blob (the classic "container has no internet egress" failure).
fn check_reachability(endpoint: &Endpoint, no_relay: bool) {
    let addr = endpoint.addr();
    let has_relay = addr.relay_urls().next().is_some();
    let ips: Vec<IpAddr> = addr.ip_addrs().map(|sa| sa.ip()).collect();
    let has_public = ips.iter().any(|ip| is_globally_reachable(*ip));
    let has_nonloopback = ips.iter().any(|ip| !ip.is_loopback());

    if has_relay || has_public {
        info!(relay = has_relay, public_ip = has_public, "reachability OK — peers can connect");
    } else if no_relay && has_nonloopback {
        warn!(
            "--no-relay: only private/LAN addresses — reachable on the LOCAL network only, \
             not across NATs. Fine for a same-LAN test; useless for a real swarm peer."
        );
    } else {
        error!(
            "NOT REACHABLE: no iroh relay and no public address. Peers on other networks \
             CANNOT connect to this node and its address blob is UNROUTABLE. Almost always the \
             container has no internet egress — check, in order: (1) veth `gateway=` / default \
             route, (2) NAT masquerade for the container subnet, (3) a firewall forward 'accept' \
             rule for that subnet, (4) the container `dns=` setting. The router itself having \
             internet is not enough — the *container's* forwarded traffic must reach the internet."
        );
    }
}

/// Is `ip` routable from outside the local network (i.e. usable in a blob a
/// remote peer could dial)?
fn is_globally_reachable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified())
        }
        IpAddr::V6(v6) => !(v6.is_loopback() || v6.is_unspecified()),
    }
}

fn print_identity(endpoint: &Endpoint) -> Result<()> {
    println!("node id: {}", endpoint.id());
    println!("address: {}", encode_addr(&endpoint.addr())?);
    Ok(())
}

/// `status` subcommand (mjolnir-mesh OpenWrt enablement): a read-only,
/// daemon-free dump of ground truth. Identity + build stamp come from the
/// binary and secret; interfaces and routes come straight from the kernel via
/// netlink. The point is to answer the questions the auu session had to grep
/// logs for — is the backhaul addr assigned, is its interface dual-addressed,
/// did routing install mesh routes and via what next-hop — in one command.
async fn run_status(secret_file: Option<&std::path::Path>) -> Result<()> {
    // status is a diagnostic, never a provisioning step: resolve the SAME secret
    // path the procd service runs with (CLI flag > UCI meshd.secret_file > the
    // built-in default) and load it read-only. If no identity is persisted there,
    // report UNKNOWN rather than deriving a plausible-looking address from a
    // throwaway key — that garbage once read as a fleet-wide address regression
    // during pt9 validation (mjolnir-mesh-dbv).
    let path = resolve_status_secret_file(secret_file);
    let secret = load_secret_readonly(&path)?;

    println!("mjolnir-meshd status");
    println!("  build:    {}", env!("MJOLNIR_BUILD"));
    println!("  version:  {}", env!("CARGO_PKG_VERSION"));

    let Some(secret) = secret else {
        println!("  node id:  UNKNOWN (no secret at {})", path.display());
        println!("  backhaul: UNKNOWN (no node identity)");
        println!();
        // Interfaces/routes are still worth showing, but we have no derived
        // address to flag against.
        print_system_status(None).await;
        println!();
        print_addr_book_status(&addr_book_path(Path::new("/etc/mjolnir/claims.state")));
        return Ok(());
    };

    let id = secret.public().to_string();
    // Claim-aware (pt9): a node that lost a backhaul collision runs at a
    // re-derived address recorded in the persisted claim map — report THAT,
    // not the naive derivation, or the diagnosis chases the wrong address.
    let restored = load_claims(Path::new("/etc/mjolnir/claims.state"));
    let backhaul = pick_backhaul_addr(&restored, &id);
    let derived = mjolnir_mesh::tun::backhaul_addr(&id);
    let prefix = mjolnir_mesh::tun::BACKHAUL_PREFIX_LEN;

    println!("  node id:  {id}");
    if backhaul == derived {
        println!("  backhaul: {backhaul}/{prefix}  (derived from node id)");
    } else {
        println!("  backhaul: {backhaul}/{prefix}  (RE-DERIVED after collision, pt9; naive derivation would be {derived})");
    }
    println!();
    print_system_status(Some(backhaul)).await;
    println!();
    print_addr_book_status(&addr_book_path(Path::new("/etc/mjolnir/claims.state")));
    Ok(())
}

/// Print the persisted gossip address book (0yb) for `status`: peer id →
/// direct addrs, relay URL, and announced-at HLC. Reads the on-disk book only
/// (never a running daemon) and prints an explicit absence marker when it is
/// empty or missing — dbv discipline: report ground truth, never invent state.
fn print_addr_book_status(path: &Path) {
    let book = load_addr_book(path);
    println!("address book (0yb): {}", path.display());
    if book.is_empty() {
        println!("  (none — no peer addresses learned yet, or file absent)");
        return;
    }
    for (node_id, entry) in &book {
        let relay = entry.relay_url.as_deref().unwrap_or("none");
        let addrs = if entry.direct_addrs.is_empty() {
            "none".to_string()
        } else {
            entry
                .direct_addrs
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!("  {node_id}");
        println!("    direct: {addrs}");
        println!("    relay:  {relay}");
        println!(
            "    announced_at: wall={} counter={}",
            entry.announced_at.wall_clock, entry.announced_at.counter
        );
    }
}

/// The secret-file path `status` reads when `--secret-file` is omitted. Mirrors
/// the init script's `config_get secret_file meshd secret_file
/// '/etc/mjolnir/secret'` so a bare `mjolnir-meshd status` reports the deployed
/// node's real identity instead of inventing one: explicit flag wins, else the
/// UCI `meshd.secret_file` option, else the built-in default.
fn resolve_status_secret_file(cli: Option<&std::path::Path>) -> PathBuf {
    if let Some(p) = cli {
        return p.to_path_buf();
    }
    uci_secret_file().unwrap_or_else(|| PathBuf::from("/etc/mjolnir/secret"))
}

/// Best-effort parse of `option secret_file '<path>'` from the `meshd` section of
/// the UCI config (`/etc/config/mjolnir`). Any read/parse miss returns None and
/// the caller falls back to the built-in default. Comment lines start with `#`
/// so they never match the `option secret_file` prefix.
fn uci_secret_file() -> Option<PathBuf> {
    let text = std::fs::read_to_string("/etc/config/mjolnir").ok()?;
    parse_uci_secret_file(&text)
}

/// Pure parse of the `option secret_file '<path>'` value out of UCI config text.
/// Split from the file read so it's unit-testable.
fn parse_uci_secret_file(text: &str) -> Option<PathBuf> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("option secret_file") {
            let val = rest.trim().trim_matches(|c| c == '\'' || c == '"');
            if !val.is_empty() {
                return Some(PathBuf::from(val));
            }
        }
    }
    None
}

/// Read-only secret load for `status`: never generates or writes a key (unlike
/// `load_or_create_secret`, which provisions on miss). Returns None when neither
/// the file nor `IROH_SECRET` yields an identity, so the caller reports UNKNOWN
/// instead of deriving from a throwaway (mjolnir-mesh-dbv).
fn load_secret_readonly(path: &Path) -> Result<Option<SecretKey>> {
    if path.exists() {
        let hex = std::fs::read_to_string(path)
            .with_context(|| format!("reading secret file {}", path.display()))?;
        return Ok(Some(parse_secret_hex(hex.trim())?));
    }
    if let Ok(env) = std::env::var("IROH_SECRET") {
        return Ok(Some(env.parse::<SecretKey>().context("parsing IROH_SECRET")?));
    }
    Ok(None)
}

/// True for the mesh's reserved IPv4 spaces — client `10.42/16`, backhaul
/// `10.254/16`, per-peer tunnel /31s `10.255/16` — the routes worth showing.
#[cfg(target_os = "linux")]
fn is_mesh_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10 && matches!(o[1], 42 | 254 | 255)
}

/// Dump interfaces (IPv4) and mesh-space kernel routes via netlink. Flags the
/// dual-addressed-backhaul trap (the auu root cause) and a missing backhaul addr.
#[cfg(target_os = "linux")]
async fn print_system_status(backhaul: Option<Ipv4Addr>) {
    use futures_util::stream::TryStreamExt;
    use rtnetlink::packet_route::address::AddressAttribute;
    use rtnetlink::packet_route::link::LinkAttribute;
    use rtnetlink::packet_route::route::{RouteAddress, RouteAttribute};
    use rtnetlink::{new_connection, RouteMessageBuilder};
    use std::collections::HashMap;

    let (connection, handle, _) = match new_connection() {
        Ok(c) => c,
        Err(e) => {
            println!("(could not open netlink to read system state: {e})");
            return;
        }
    };
    tokio::spawn(connection);

    // ifindex -> interface name
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut links = handle.link().get().execute();
    while let Ok(Some(link)) = links.try_next().await {
        if let Some(name) = link.attributes.iter().find_map(|a| match a {
            LinkAttribute::IfName(n) => Some(n.clone()),
            _ => None,
        }) {
            names.insert(link.header.index, name);
        }
    }

    // ifindex -> [(ipv4, prefix_len)]
    let mut addrs: HashMap<u32, Vec<(Ipv4Addr, u8)>> = HashMap::new();
    let mut astream = handle.address().get().execute();
    while let Ok(Some(msg)) = astream.try_next().await {
        if let Some(v4) = msg.attributes.iter().find_map(|a| match a {
            AddressAttribute::Local(IpAddr::V4(v)) | AddressAttribute::Address(IpAddr::V4(v)) => {
                Some(*v)
            }
            _ => None,
        }) {
            addrs
                .entry(msg.header.index)
                .or_default()
                .push((v4, msg.header.prefix_len));
        }
    }

    println!("interfaces (IPv4):");
    let mut backhaul_seen = false;
    let mut idxs: Vec<u32> = addrs.keys().copied().collect();
    idxs.sort_unstable();
    for idx in idxs {
        let name = names.get(&idx).cloned().unwrap_or_else(|| format!("if{idx}"));
        if name == "lo" {
            continue;
        }
        let list = &addrs[&idx];
        let has_backhaul = backhaul.is_some_and(|b| list.iter().any(|(a, _)| *a == b));
        backhaul_seen |= has_backhaul;
        let shown = list
            .iter()
            .map(|(a, p)| format!("{a}/{p}"))
            .collect::<Vec<_>>()
            .join(", ");
        let flag = if has_backhaul && list.len() > 1 {
            "   <- backhaul; DUAL-ADDRESSED (extra addrs can leak as bogus next-hops — see auu)"
        } else if has_backhaul {
            "   <- backhaul"
        } else {
            ""
        };
        println!("  {name:<12} {shown}{flag}");
    }
    if let Some(backhaul) = backhaul {
        if !backhaul_seen {
            println!(
                "  WARNING: derived backhaul {backhaul} is not assigned on any interface \
                 (daemon not running, or the backhaul interface is down)"
            );
        }
    }
    println!();

    println!("mesh routes (10.42/16 client · 10.254/16 backhaul · 10.255/16 tunnels):");
    let mut found = false;
    let mut rstream = handle
        .route()
        .get(RouteMessageBuilder::<Ipv4Addr>::new().build())
        .execute();
    while let Ok(Some(r)) = rstream.try_next().await {
        let dst = r.attributes.iter().find_map(|a| match a {
            RouteAttribute::Destination(RouteAddress::Inet(v)) => Some(*v),
            _ => None,
        });
        let Some(dst) = dst else { continue };
        if !is_mesh_v4(dst) {
            continue;
        }
        let gw = r.attributes.iter().find_map(|a| match a {
            RouteAttribute::Gateway(RouteAddress::Inet(v)) => Some(format!("via {v} ")),
            _ => None,
        });
        let dev = r
            .attributes
            .iter()
            .find_map(|a| match a {
                RouteAttribute::Oif(i) => names.get(i).cloned(),
                _ => None,
            })
            .unwrap_or_else(|| "?".into());
        println!(
            "  {dst}/{:<3} {}dev {dev}",
            r.header.destination_prefix_length,
            gw.unwrap_or_default()
        );
        found = true;
    }
    if !found {
        println!("  (none installed — no peers converged yet, or routing not running)");
    }
}

#[cfg(not(target_os = "linux"))]
async fn print_system_status(_backhaul: Option<Ipv4Addr>) {
    println!("(interface/route inspection is Linux-only; identity is shown above)");
}

async fn run_listen(endpoint: Endpoint, no_relay: bool) -> Result<()> {
    wait_until_addressable(&endpoint, no_relay).await;
    print_identity(&endpoint)?;
    info!(
        alpn = %String::from_utf8_lossy(MESH_ALPN),
        "listening — hand the address above to `connect`"
    );

    let router = Router::builder(endpoint)
        .accept(MESH_ALPN, PingHandler)
        .spawn();

    tokio::signal::ctrl_c().await.context("waiting for Ctrl-C")?;
    info!("shutting down");
    router.shutdown().await.context("router shutdown")?;
    Ok(())
}

async fn run_connect(endpoint: Endpoint, addr_blob: &str) -> Result<()> {
    let addr = parse_peer(addr_blob).context("parsing peer")?;
    let peer = addr.id;
    info!(%peer, "dialing");

    let conn = endpoint
        .connect(addr, MESH_ALPN)
        .await
        .context("connect failed")?;
    info!(%peer, "connection established");

    let payload = Bytes::from_static(PING);
    let start = Instant::now();
    conn.send_datagram(payload.clone())
        .context("send_datagram failed")?;
    let echoed = conn.read_datagram().await.context("no echo received")?;
    let rtt = start.elapsed();

    if echoed == payload {
        println!("round-trip OK to {peer} in {rtt:?}");
    } else {
        println!("echo MISMATCH from {peer} ({} bytes back)", echoed.len());
    }

    conn.close(0u32.into(), b"done");
    Ok(())
}

/// iroh protocol handler that echoes every datagram back to the sender until
/// the connection closes. The P0 "shuttle packets" stand-in.
#[derive(Debug, Clone)]
struct PingHandler;

impl ProtocolHandler for PingHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = connection.remote_id();
        info!(%peer, "inbound mesh connection");
        loop {
            match connection.read_datagram().await {
                Ok(dg) => {
                    if let Err(e) = connection.send_datagram(dg) {
                        warn!(%peer, "echo failed: {e}");
                        break;
                    }
                }
                Err(e) => {
                    info!(%peer, "connection ended: {e}");
                    break;
                }
            }
        }
        Ok(())
    }
}

// --- identity persistence -------------------------------------------------

fn load_or_create_secret(path: Option<&Path>) -> Result<SecretKey> {
    if let Some(p) = path {
        if p.exists() {
            let hex = std::fs::read_to_string(p)
                .with_context(|| format!("reading secret file {}", p.display()))?;
            return parse_secret_hex(hex.trim());
        }
        let secret = SecretKey::generate();
        std::fs::write(p, encode_secret_hex(&secret))
            .with_context(|| format!("writing secret file {}", p.display()))?;
        info!(path = %p.display(), id = %secret.public(), "generated new node identity");
        return Ok(secret);
    }

    if let Ok(env) = std::env::var("IROH_SECRET") {
        return env.parse::<SecretKey>().context("parsing IROH_SECRET");
    }

    warn!("no --secret-file or IROH_SECRET set; using an ephemeral identity");
    Ok(SecretKey::generate())
}

fn encode_secret_hex(secret: &SecretKey) -> String {
    data_encoding::HEXLOWER.encode(&secret.to_bytes())
}

fn parse_secret_hex(hex: &str) -> Result<SecretKey> {
    let bytes = data_encoding::HEXLOWER
        .decode(hex.as_bytes())
        .context("secret file is not valid lowercase hex")?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .context("secret must be exactly 32 bytes")?;
    Ok(SecretKey::from_bytes(&arr))
}

// --- address blob (postcard + base32, matching the node's ticket scheme) ---

fn encode_addr(addr: &EndpointAddr) -> Result<String> {
    let bytes = postcard::to_allocvec(addr).context("serializing address")?;
    Ok(data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase())
}

fn decode_addr(blob: &str) -> Result<EndpointAddr> {
    let bytes = data_encoding::BASE32_NOPAD
        .decode(blob.to_uppercase().as_bytes())
        .context("address blob is not valid base32")?;
    postcard::from_bytes(&bytes).context("deserializing address")
}

/// Accept either a full address blob, or a bare 64-hex node id (whose address
/// is resolved via discovery — e.g. mDNS in `--lan` mode).
fn parse_peer(arg: &str) -> Result<EndpointAddr> {
    if arg.len() == 64 && arg.bytes().all(|b| b.is_ascii_hexdigit()) {
        let id: EndpointId = arg.parse().context("parsing node id")?;
        Ok(EndpointAddr::new(id))
    } else {
        decode_addr(arg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_of(claims: &[SubnetClaim]) -> HashMap<String, SubnetClaim> {
        claims
            .iter()
            .map(|c| (c.cidr.to_string(), c.clone()))
            .collect()
    }

    #[test]
    fn uci_secret_file_parsed_from_meshd_section() {
        // Mirrors deploy/openwrt/files/etc/config/mjolnir: status must read the
        // same path the service runs with, and ignore the commented example line
        // that mentions --secret-file (mjolnir-mesh-dbv).
        let cfg = "\
config meshd 'meshd'
\toption enabled '1'
\toption secret_file '/etc/mjolnir/secret'
\toption babeld 'babeld'
#   mjolnir-meshd id --secret-file /etc/mjolnir/other
";
        assert_eq!(
            parse_uci_secret_file(cfg),
            Some(PathBuf::from("/etc/mjolnir/secret"))
        );
    }

    #[test]
    fn uci_secret_file_absent_is_none() {
        assert_eq!(parse_uci_secret_file("config meshd 'meshd'\n"), None);
    }

    #[test]
    fn pick_backhaul_prefers_own_restored_claim() {
        // A node that moved to a re-derived address after a lost collision
        // keeps that address across restarts via its persisted claim.
        let moved = claim("10.254.9.9/32", "me", 500);
        let store = store_of(&[moved]);
        assert_eq!(pick_backhaul_addr(&store, "me"), "10.254.9.9".parse::<Ipv4Addr>().unwrap());
    }

    #[test]
    fn pick_backhaul_avoids_foreign_claim_on_attempt_zero() {
        let derived0 = mjolnir_mesh::tun::backhaul_addr("me");
        let foreign = claim(&format!("{derived0}/32"), "them", 100);
        let store = store_of(&[foreign]);
        let picked = pick_backhaul_addr(&store, "me");
        assert_ne!(picked, derived0, "must derive around the persisted winner");
        assert_eq!(picked, mjolnir_mesh::tun::backhaul_addr_salted("me", 1));
    }

    #[test]
    fn pick_backhaul_default_is_legacy_derivation() {
        // Empty store (fresh mesh): byte-identical to the pre-pt9 address.
        let store = HashMap::new();
        assert_eq!(pick_backhaul_addr(&store, "me"), mjolnir_mesh::tun::backhaul_addr("me"));
    }

    #[test]
    fn peer_hint_uses_gossiped_claim_else_derivation() {
        let moved = claim("10.254.7.7/32", "peer-a", 100);
        let store = store_of(&[moved]);
        assert_eq!(
            peer_backhaul_hint(&store, "peer-a"),
            "10.254.7.7".parse::<Ipv4Addr>().unwrap()
        );
        assert_eq!(
            peer_backhaul_hint(&store, "peer-b"),
            mjolnir_mesh::tun::backhaul_addr("peer-b")
        );
    }

    #[test]
    fn partition_claims_ignores_backhaul_claims() {
        // Own backhaul /32 must not become the senior client claim (or an
        // "extra" to be released); foreign backhaul /32s must not reach the
        // /24 allocator's avoid set.
        let own_backhaul = claim("10.254.9.9/32", "me", 100);
        let own_client = claim("10.42.5.0/24", "me", 200);
        let foreign_backhaul = claim("10.254.1.1/32", "them", 50);
        let store = store_of(&[own_backhaul, own_client, foreign_backhaul]);
        let (keep, extras, foreign) = partition_claims(&store, "me");
        let (net, _) = keep.expect("client claim must be kept");
        assert_eq!(net, "10.42.5.0/24".parse::<Ipv4Net>().unwrap());
        assert!(extras.is_empty(), "backhaul claim must not be an 'extra'");
        assert!(foreign.is_empty(), "foreign backhaul claim must not reach the allocator");
    }

    #[test]
    fn losing_backhaul_conflict_is_classified_by_block() {
        // The dispatch loop distinguishes a lost backhaul /32 (restart to
        // re-derive) from a lost client /24 (retract + re-claim) purely by
        // block membership of the returned net.
        let mut store = HashMap::new();
        let mine = claim("10.254.3.3/32", "me", 2_000); // later writer — loses
        store.insert(mine.cidr.to_string(), mine.clone());
        let theirs = claim("10.254.3.3/32", "them", 1_000); // first writer — wins
        let lost = apply_subnet_message(&mut store, &update(&theirs), "me")
            .expect("we must lose the FWW conflict");
        assert!(mjolnir_mesh::tun::in_backhaul_block(&lost));
        assert_eq!(
            store.get("10.254.3.3/32").unwrap().owner_node_id,
            "them",
            "winner's claim must replace ours in the store"
        );
    }

    fn claim(cidr: &str, owner: &str, wall: u64) -> SubnetClaim {
        SubnetClaim {
            cidr: cidr.parse().expect("valid cidr"),
            owner_node_id: owner.to_string(),
            site_name: None,
            claimed_at: HLC {
                wall_clock: wall,
                counter: 0,
                node_id: owner.to_string(),
            },
        }
    }

    fn update(c: &SubnetClaim) -> GossipMessage {
        GossipMessage::SubnetClaimUpdate {
            cidr: c.cidr.to_string(),
            entry: c.clone(),
        }
    }

    #[test]
    fn applies_new_claim() {
        let mut store = HashMap::new();
        let incoming = claim("10.42.1.0/24", "peer-b", 100);
        let reclaim = apply_subnet_message(&mut store, &update(&incoming), "self");
        assert!(reclaim.is_none());
        assert_eq!(store["10.42.1.0/24"].owner_node_id, "peer-b");
    }

    #[test]
    fn same_owner_newer_updates_no_reclaim() {
        let mut store = HashMap::new();
        store.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 100));
        let newer = claim("10.42.1.0/24", "peer-b", 200);
        let reclaim = apply_subnet_message(&mut store, &update(&newer), "self");
        assert!(reclaim.is_none());
        assert_eq!(store["10.42.1.0/24"].claimed_at.wall_clock, 200);
    }

    #[test]
    fn older_claim_is_unchanged() {
        let mut store = HashMap::new();
        store.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 200));
        let older = claim("10.42.1.0/24", "peer-b", 100);
        let reclaim = apply_subnet_message(&mut store, &update(&older), "self");
        assert!(reclaim.is_none());
        assert_eq!(store["10.42.1.0/24"].claimed_at.wall_clock, 200);
    }

    #[test]
    fn conflict_we_lose_triggers_reclaim() {
        // We hold the /24 (wall 200); a peer's earlier claim (wall 100) wins by
        // first-writer-wins, so we lose and must re-claim.
        let mut store = HashMap::new();
        store.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "self", 200));
        let earlier_peer = claim("10.42.1.0/24", "peer-b", 100);
        let reclaim = apply_subnet_message(&mut store, &update(&earlier_peer), "self");
        assert_eq!(
            reclaim,
            Some("10.42.1.0/24".parse().unwrap()),
            "we should retract + re-claim after losing our subnet"
        );
        assert_eq!(store["10.42.1.0/24"].owner_node_id, "peer-b");
    }

    #[test]
    fn conflict_we_win_no_reclaim() {
        // We hold the /24 with the earlier claim (wall 100); a peer's later
        // claim (wall 200) loses, so we keep it and do NOT re-claim.
        let mut store = HashMap::new();
        store.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "self", 100));
        let later_peer = claim("10.42.1.0/24", "peer-b", 200);
        let reclaim = apply_subnet_message(&mut store, &update(&later_peer), "self");
        assert!(reclaim.is_none());
        assert_eq!(store["10.42.1.0/24"].owner_node_id, "self");
    }

    #[test]
    fn release_removes_when_newer() {
        let mut store = HashMap::new();
        store.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 100));
        let release = GossipMessage::SubnetClaimRelease {
            cidr: "10.42.1.0/24".to_string(),
            hlc: HLC {
                wall_clock: 200,
                counter: 0,
                node_id: "peer-b".to_string(),
            },
        };
        let reclaim = apply_subnet_message(&mut store, &release, "self");
        assert!(reclaim.is_none());
        assert!(!store.contains_key("10.42.1.0/24"), "newer release should remove the claim");
    }

    #[test]
    fn release_ignored_when_older() {
        let mut store = HashMap::new();
        store.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 200));
        let stale_release = GossipMessage::SubnetClaimRelease {
            cidr: "10.42.1.0/24".to_string(),
            hlc: HLC {
                wall_clock: 100,
                counter: 0,
                node_id: "peer-b".to_string(),
            },
        };
        apply_subnet_message(&mut store, &stale_release, "self");
        assert!(store.contains_key("10.42.1.0/24"), "stale release must not remove a newer claim");
    }

    #[test]
    fn partition_reuses_own_restored_claim() {
        // The eon manifestation-1 setup: after a restart the store holds our
        // own claim restored from disk. It must come back as the claim to
        // keep, not land in the avoid set (which made us claim a fresh /24).
        let mut store = HashMap::new();
        store.insert("10.42.12.0/24".to_string(), claim("10.42.12.0/24", "self", 100));
        store.insert("10.42.7.0/24".to_string(), claim("10.42.7.0/24", "peer-b", 50));
        let (keep, extras, foreign) = partition_claims(&store, "self");
        let (net, entry) = keep.expect("own restored claim must be reused");
        assert_eq!(net, "10.42.12.0/24".parse::<Ipv4Net>().unwrap());
        assert_eq!(entry.claimed_at.wall_clock, 100, "claimed_at must be preserved (seniority)");
        assert!(extras.is_empty());
        assert_eq!(foreign.len(), 1);
        assert!(foreign.contains(&"10.42.7.0/24".parse().unwrap()));
    }

    #[test]
    fn partition_keeps_senior_own_claim_releases_extras() {
        // Damage from the pre-fix restart bug: we own TWO claims. Keep the
        // senior one (lowest HLC) and mark the newer one for release.
        let mut store = HashMap::new();
        store.insert("10.42.13.0/24".to_string(), claim("10.42.13.0/24", "self", 200));
        store.insert("10.42.12.0/24".to_string(), claim("10.42.12.0/24", "self", 100));
        let (keep, extras, foreign) = partition_claims(&store, "self");
        let (net, _) = keep.expect("a claim must be kept");
        assert_eq!(net, "10.42.12.0/24".parse::<Ipv4Net>().unwrap(), "senior claim wins");
        assert_eq!(extras, vec!["10.42.13.0/24".parse::<Ipv4Net>().unwrap()]);
        assert!(foreign.is_empty());
    }

    #[test]
    fn partition_empty_store_picks_fresh() {
        let store = HashMap::new();
        let (keep, extras, foreign) = partition_claims(&store, "self");
        assert!(keep.is_none());
        assert!(extras.is_empty());
        assert!(foreign.is_empty());
    }

    #[test]
    fn stock_lan_config_needs_reconcile() {
        // Fresh-from-flash: bare stock address, option form (no CIDR).
        assert!(!lan_uci_is_current("192.168.1.1", "10.42.61.1/24"));
        // Unset ipaddr (lan exists but empty) also reconciles.
        assert!(!lan_uci_is_current("", "10.42.61.1/24"));
    }

    #[test]
    fn reconciled_lan_config_is_idempotent() {
        // Claimed primary + recovery alias, as this fix writes it.
        assert!(lan_uci_is_current(
            "10.42.61.1/24 192.168.1.1/24",
            "10.42.61.1/24"
        ));
    }

    #[test]
    fn wrong_order_or_new_claim_needs_reconcile() {
        // The manual bench renumber put the claimed addr FIRST — current.
        // Stock-first ordering (the 659 bug state) is not.
        assert!(!lan_uci_is_current(
            "192.168.1.1/24 10.42.61.1/24",
            "10.42.61.1/24"
        ));
        // Claim changed (conflict loss → re-claim): old primary must give way.
        assert!(!lan_uci_is_current(
            "10.42.242.1/24 192.168.1.1/24",
            "10.42.243.1/24"
        ));
    }

    #[tokio::test]
    async fn first_neighbor_releases_the_claim_gate() {
        let (tx, rx) = tokio::sync::watch::channel(0usize);
        let waiter = tokio::spawn(wait_for_first_neighbor(rx, Duration::from_secs(5)));
        tokio::time::sleep(Duration::from_millis(20)).await;
        tx.send(1).unwrap();
        assert!(waiter.await.unwrap(), "neighbor up must release the gate");
    }

    #[tokio::test]
    async fn no_neighbor_hits_the_cap() {
        let (_tx, rx) = tokio::sync::watch::channel(0usize);
        assert!(!wait_for_first_neighbor(rx, Duration::from_millis(50)).await);
    }

    #[tokio::test]
    async fn already_joined_passes_immediately() {
        let (_tx, rx) = tokio::sync::watch::channel(1usize);
        assert!(wait_for_first_neighbor(rx, Duration::from_millis(50)).await);
    }

    #[tokio::test]
    async fn dropped_channel_does_not_hang_the_gate() {
        let (tx, rx) = tokio::sync::watch::channel(0usize);
        drop(tx);
        assert!(!wait_for_first_neighbor(rx, Duration::from_secs(5)).await);
    }

    #[test]
    fn load_claims_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.state");
        assert!(load_claims(&path).is_empty());
    }

    #[test]
    fn load_claims_corrupt_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claims.state");
        std::fs::write(&path, b"not a valid postcard payload at all").unwrap();
        assert!(load_claims(&path).is_empty());
    }

    #[test]
    fn persist_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claims.state");
        let mut snapshot = HashMap::new();
        snapshot.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 100));
        snapshot.insert("10.42.2.0/24".to_string(), claim("10.42.2.0/24", "peer-c", 200));

        persist_claims(&snapshot, &path);
        let loaded = load_claims(&path);

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded["10.42.1.0/24"].owner_node_id, "peer-b");
        assert_eq!(loaded["10.42.2.0/24"].owner_node_id, "peer-c");
    }

    #[test]
    fn persist_creates_missing_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("claims.state");
        let mut snapshot = HashMap::new();
        snapshot.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 100));

        persist_claims(&snapshot, &path);

        assert!(path.exists());
        assert_eq!(load_claims(&path).len(), 1);
    }

    #[test]
    fn persist_overwrites_stale_tmp_file() {
        // A previous crash mid-write could leave a stray .tmp sibling; persisting
        // again must still succeed and leave the target file correct.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claims.state");
        std::fs::write(path.with_extension("tmp"), b"leftover garbage").unwrap();

        let mut snapshot = HashMap::new();
        snapshot.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 100));
        persist_claims(&snapshot, &path);

        assert_eq!(load_claims(&path).len(), 1);
    }

    // --- address book (mjolnir-mesh-0yb) --------------------------------------

    fn addr_entry(node_id: &str, wall_clock: u64, addr: &str) -> PeerAddrEntry {
        PeerAddrEntry::new(
            node_id.to_string(),
            vec![addr.parse().unwrap()],
            None,
            HLC { wall_clock, counter: 0, node_id: node_id.to_string() },
        )
    }

    #[test]
    fn addr_book_path_is_sibling_of_claims_file() {
        assert_eq!(
            addr_book_path(Path::new("/etc/mjolnir/claims.state")),
            PathBuf::from("/etc/mjolnir/addrbook.state")
        );
        assert_eq!(
            addr_book_path(Path::new("/var/run/mjolnir/claims.state")),
            PathBuf::from("/var/run/mjolnir/addrbook.state")
        );
    }

    #[test]
    fn apply_peer_addr_inserts_new_peer_and_returns_it() {
        let mut book = AddrBook::new();
        let msg = GossipMessage::PeerAddrUpdate {
            node_id: "peer-a".to_string(),
            entry: addr_entry("peer-a", 1_000, "10.254.1.1:49737"),
        };
        let learned = apply_peer_addr_message(&mut book, &msg, "me");
        assert!(learned.is_some(), "a new peer entry is learned");
        assert_eq!(book.len(), 1);
        assert_eq!(book["peer-a"].direct_addrs.len(), 1);
    }

    #[test]
    fn apply_peer_addr_skips_self_announcement() {
        let mut book = AddrBook::new();
        let msg = GossipMessage::PeerAddrUpdate {
            node_id: "me".to_string(),
            entry: addr_entry("me", 1_000, "10.254.9.9:49737"),
        };
        assert!(apply_peer_addr_message(&mut book, &msg, "me").is_none());
        assert!(book.is_empty(), "own echoed announcement must not enter the book");
    }

    #[test]
    fn apply_peer_addr_updates_on_newer_and_ignores_stale() {
        let mut book = AddrBook::new();
        let older = GossipMessage::PeerAddrUpdate {
            node_id: "peer-a".to_string(),
            entry: addr_entry("peer-a", 1_000, "10.254.1.1:49737"),
        };
        let newer = GossipMessage::PeerAddrUpdate {
            node_id: "peer-a".to_string(),
            entry: addr_entry("peer-a", 2_000, "10.254.1.2:49737"),
        };
        assert!(apply_peer_addr_message(&mut book, &older, "me").is_some());
        // Newer announcement wins (LWW).
        assert!(apply_peer_addr_message(&mut book, &newer, "me").is_some());
        assert_eq!(book["peer-a"].direct_addrs[0].to_string(), "10.254.1.2:49737");
        // Replaying the older one is Unchanged -> None, and does not regress.
        assert!(apply_peer_addr_message(&mut book, &older, "me").is_none());
        assert_eq!(book["peer-a"].direct_addrs[0].to_string(), "10.254.1.2:49737");
    }

    #[test]
    fn apply_peer_addr_ignores_non_peer_addr_messages() {
        let mut book = AddrBook::new();
        let msg = GossipMessage::SubnetClaimRelease {
            cidr: "10.42.1.0/24".to_string(),
            hlc: HLC { wall_clock: 1, counter: 0, node_id: "x".to_string() },
        };
        assert!(apply_peer_addr_message(&mut book, &msg, "me").is_none());
        assert!(book.is_empty());
    }

    #[test]
    fn load_addr_book_missing_or_corrupt_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_addr_book(&dir.path().join("nope.state")).is_empty());
        let path = dir.path().join("addrbook.state");
        std::fs::write(&path, b"not a valid postcard payload").unwrap();
        assert!(load_addr_book(&path).is_empty());
    }

    #[test]
    fn persist_then_load_addr_book_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("addrbook.state");
        let mut book = AddrBook::new();
        book.insert("peer-a".to_string(), addr_entry("peer-a", 100, "10.254.1.1:49737"));
        book.insert("peer-b".to_string(), addr_entry("peer-b", 200, "10.254.2.2:49737"));

        persist_addr_book(&book, &path);
        assert!(path.exists(), "parent dir is created and file written");
        let loaded = load_addr_book(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded["peer-a"].announced_at.wall_clock, 100);
        assert_eq!(loaded["peer-b"].direct_addrs[0].to_string(), "10.254.2.2:49737");
    }

    // --- service directory (7jb) ---

    fn svc_entry(hostname: &str, port: u16, wall_clock: u64, node_id: &str) -> ServiceEntry {
        use std::net::{IpAddr, Ipv4Addr};
        ServiceEntry {
            hostname: hostname.to_string(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)),
            port,
            protocol: "_ipp._tcp".to_string(),
            txt: std::collections::BTreeMap::new(),
            host_mac: [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
            updated_at: HLC {
                wall_clock,
                counter: 0,
                node_id: node_id.to_string(),
            },
        }
    }

    fn svc_msg(name: &str, entry: ServiceEntry) -> GossipMessage {
        GossipMessage::ServiceUpdate {
            name: name.to_string(),
            entry,
        }
    }

    #[test]
    fn apply_service_inserts_new_and_returns_it() {
        let mut book = ServiceBook::new();
        let msg = svc_msg("printer._ipp._tcp", svc_entry("printer", 631, 100, "node-a"));
        let learned = apply_service_message(&mut book, &msg);
        assert!(learned.is_some());
        assert_eq!(learned.unwrap().0, "printer._ipp._tcp");
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn apply_service_updates_on_newer_and_ignores_stale() {
        let mut book = ServiceBook::new();
        let older = svc_msg("printer._ipp._tcp", svc_entry("printer", 631, 100, "node-a"));
        let newer = svc_msg("printer._ipp._tcp", svc_entry("printer", 9100, 200, "node-a"));
        assert!(apply_service_message(&mut book, &older).is_some());
        assert!(apply_service_message(&mut book, &newer).is_some());
        assert_eq!(book["printer._ipp._tcp"].port, 9100);
        // Re-delivering the older message must not roll the record back.
        assert!(apply_service_message(&mut book, &older).is_none());
        assert_eq!(book["printer._ipp._tcp"].port, 9100);
    }

    #[test]
    fn apply_service_ignores_non_service_messages() {
        let mut book = ServiceBook::new();
        let msg = GossipMessage::LeaseRelease {
            mac: [0; 6],
            hlc: HLC { wall_clock: 1, counter: 0, node_id: "x".to_string() },
        };
        assert!(apply_service_message(&mut book, &msg).is_none());
        assert!(book.is_empty());
    }

    #[test]
    fn load_service_book_missing_or_corrupt_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_service_book(&dir.path().join("nope.state")).is_empty());
        let path = dir.path().join("services.state");
        std::fs::write(&path, b"not a valid postcard payload").unwrap();
        assert!(load_service_book(&path).is_empty());
    }

    #[test]
    fn persist_then_load_service_book_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("services.state");
        let mut book = ServiceBook::new();
        book.insert("printer._ipp._tcp".to_string(), svc_entry("printer", 631, 100, "node-a"));
        book.insert("nas._smb._tcp".to_string(), svc_entry("nas", 445, 200, "node-b"));

        persist_service_book(&book, &path);
        assert!(path.exists(), "parent dir is created and file written");
        let loaded = load_service_book(&path);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded["printer._ipp._tcp"].port, 631);
        assert_eq!(loaded["nas._smb._tcp"].updated_at.wall_clock, 200);
    }

    // --- directory.json projection (mjolnir-mesh-avs) -------------------------

    fn user_entry(username: &str, display_name: &str, wall_clock: u64) -> UserEntry {
        UserEntry {
            username: username.to_string(),
            display_name: display_name.to_string(),
            registered_by: "self".to_string(),
            attrs: std::collections::BTreeMap::new(),
            updated_at: HLC { wall_clock, counter: 0, node_id: "self".to_string() },
        }
    }

    #[test]
    fn build_directory_snapshot_has_version_and_node_identity() {
        let claims = HashMap::new();
        let addr_book = AddrBook::new();
        let user_book = UserBook::new();
        let service_book = ServiceBook::new();

        let snapshot = build_directory_snapshot(
            &claims,
            &addr_book,
            &user_book,
            &service_book,
            "self",
            "10.254.1.1".parse().unwrap(),
        );

        assert_eq!(snapshot.version, DIRECTORY_SCHEMA_VERSION);
        assert_eq!(snapshot.node.node_id, "self");
        assert_eq!(snapshot.node.backhaul_addr, "10.254.1.1");
        // No claim recorded yet for "self" — subnet is unknown during warmup.
        assert_eq!(snapshot.node.subnet, None);
        assert!(snapshot.neighbors.is_empty());
        assert!(snapshot.identities.is_empty());
        assert!(snapshot.services.is_empty());
    }

    #[test]
    fn build_directory_snapshot_projects_neighbors_identities_and_services() {
        let mut claims = HashMap::new();
        claims.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "self", 100));
        claims.insert("10.42.2.0/24".to_string(), claim("10.42.2.0/24", "peer-a", 100));

        let mut addr_book = AddrBook::new();
        addr_book.insert("self".to_string(), addr_entry("self", 100, "10.254.1.1:49737"));
        addr_book.insert("peer-a".to_string(), addr_entry("peer-a", 100, "10.254.2.2:49737"));

        let mut user_book = UserBook::new();
        user_book.insert("ada".to_string(), user_entry("ada", "Ada Lovelace", 100));

        let mut service_book = ServiceBook::new();
        service_book.insert(
            "printer._ipp._tcp".to_string(),
            svc_entry("printer", 631, 100, "peer-a"),
        );

        let snapshot = build_directory_snapshot(
            &claims,
            &addr_book,
            &user_book,
            &service_book,
            "self",
            "10.254.1.1".parse().unwrap(),
        );

        // Valid JSON with the schema version field (AC).
        let json = serde_json::to_string(&snapshot).expect("snapshot must serialize as JSON");
        assert!(json.contains("\"version\":1"));

        // "You are here": self's own claimed /24, not a peer's.
        assert_eq!(snapshot.node.subnet.as_deref(), Some("10.42.1.0/24"));

        // Neighbors exclude self and join AddrBook with the neighbor's own claim.
        assert_eq!(snapshot.neighbors.len(), 1);
        let neighbor = &snapshot.neighbors[0];
        assert_eq!(neighbor.node_id, "peer-a");
        assert_eq!(neighbor.addrs, vec!["10.254.2.2:49737".to_string()]);
        assert_eq!(neighbor.subnet.as_deref(), Some("10.42.2.0/24"));

        // Identities come straight from the user book.
        assert_eq!(snapshot.identities.len(), 1);
        assert_eq!(snapshot.identities[0].username, "ada");
        assert_eq!(snapshot.identities[0].display_name, "Ada Lovelace");

        // Services are keyed by the ServiceBook map key, not entry.hostname.
        assert_eq!(snapshot.services.len(), 1);
        assert_eq!(snapshot.services[0].name, "printer._ipp._tcp");
        assert_eq!(snapshot.services[0].port, 631);
        assert_eq!(snapshot.services[0].protocol, "_ipp._tcp");
    }

    #[test]
    fn persist_directory_writes_valid_json_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("directory.json");
        // A stray .tmp sibling from a previous crash must not break a fresh write.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path.with_extension("tmp"), b"leftover garbage").unwrap();

        let snapshot = build_directory_snapshot(
            &HashMap::new(),
            &AddrBook::new(),
            &UserBook::new(),
            &ServiceBook::new(),
            "self",
            "10.254.1.1".parse().unwrap(),
        );
        persist_directory(&snapshot, &path);

        assert!(path.exists(), "parent dir is created and file written");
        let text = std::fs::read_to_string(&path).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(decoded["version"], 1);
        assert_eq!(decoded["node"]["node_id"], "self");
    }

    // --- identity-submission spool ingest (mjolnir-mesh-p6u) -------------------

    #[test]
    fn spool_submission_maps_to_user_entry_with_label() {
        let sub = SpoolSubmission {
            pubkey: "abcdef0123456789".to_string(),
            sig: "deadbeef".to_string(),
            challenge: "cafef00d".to_string(),
            label: Some("Ada".to_string()),
        };
        let entry = spool_submission_to_user_entry(&sub, "router-a");

        assert_eq!(entry.username, "abcdef0123456789", "pubkey is the stable identity key");
        assert_eq!(entry.display_name, "Ada", "label wins over the derived short form");
        assert_eq!(entry.registered_by, "router-a");
        assert_eq!(entry.attrs.get("pubkey"), Some(&"abcdef0123456789".to_string()));
        assert_eq!(entry.updated_at.node_id, "router-a", "stamped with a fresh HLC");
    }

    #[test]
    fn spool_submission_without_label_uses_short_pubkey() {
        let sub = SpoolSubmission {
            pubkey: "abcdef0123456789".to_string(),
            sig: "deadbeef".to_string(),
            challenge: "cafef00d".to_string(),
            label: None,
        };
        let entry = spool_submission_to_user_entry(&sub, "router-a");
        assert_eq!(entry.display_name, "abcdef01…");
    }

    #[test]
    fn spool_submission_blank_label_falls_back_to_short_pubkey() {
        let sub = SpoolSubmission {
            pubkey: "abcdef0123456789".to_string(),
            sig: "deadbeef".to_string(),
            challenge: "cafef00d".to_string(),
            label: Some("   ".to_string()),
        };
        let entry = spool_submission_to_user_entry(&sub, "router-a");
        assert_eq!(entry.display_name, "abcdef01…");
    }

    #[test]
    fn spool_json_parses_into_expected_submission() {
        let json = r#"{"pubkey":"abcdef0123456789","sig":"deadbeef","challenge":"cafef00d","label":"Ada"}"#;
        let sub: SpoolSubmission = serde_json::from_str(json).expect("valid submission JSON");
        assert_eq!(sub.pubkey, "abcdef0123456789");
        assert_eq!(sub.label.as_deref(), Some("Ada"));

        let entry = spool_submission_to_user_entry(&sub, "router-a");
        assert_eq!(entry.username, "abcdef0123456789");
        assert_eq!(entry.display_name, "Ada");
        assert_eq!(entry.registered_by, "router-a");
    }

    #[test]
    fn spool_json_without_label_field_parses_via_serde_default() {
        // mjolnir-hello's `label` field is optional in the wire format.
        let json = r#"{"pubkey":"abcdef0123456789","sig":"deadbeef","challenge":"cafef00d"}"#;
        let sub: SpoolSubmission = serde_json::from_str(json).expect("label is optional");
        assert_eq!(sub.label, None);
    }

    #[test]
    fn ingest_identity_spool_merges_and_deletes_valid_submission() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abcdef0123456789.json");
        std::fs::write(
            &path,
            r#"{"pubkey":"abcdef0123456789","sig":"deadbeef","challenge":"cafef00d","label":"Ada"}"#,
        )
        .unwrap();

        let user_book: Arc<Mutex<UserBook>> = Arc::new(Mutex::new(UserBook::new()));
        ingest_identity_spool(dir.path(), &user_book, "router-a");

        let book = user_book.lock().unwrap();
        assert_eq!(book.len(), 1);
        let entry = &book["abcdef0123456789"];
        assert_eq!(entry.display_name, "Ada");
        assert_eq!(entry.registered_by, "router-a");
        drop(book);

        assert!(!path.exists(), "ingested spool file must be removed");
    }

    #[test]
    fn ingest_identity_spool_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abcdef0123456789.json");
        let write_submission = || {
            std::fs::write(
                &path,
                r#"{"pubkey":"abcdef0123456789","sig":"deadbeef","challenge":"cafef00d"}"#,
            )
            .unwrap();
        };
        let user_book: Arc<Mutex<UserBook>> = Arc::new(Mutex::new(UserBook::new()));

        write_submission();
        ingest_identity_spool(dir.path(), &user_book, "router-a");
        // Re-ingesting the same submission (as if the delete had raced or the
        // file were resubmitted) must not error or duplicate the record.
        write_submission();
        ingest_identity_spool(dir.path(), &user_book, "router-a");

        let book = user_book.lock().unwrap();
        assert_eq!(book.len(), 1, "merge_user LWW keeps this idempotent");
    }

    #[test]
    fn ingest_identity_spool_quarantines_malformed_file_without_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, b"not valid json").unwrap();

        let user_book: Arc<Mutex<UserBook>> = Arc::new(Mutex::new(UserBook::new()));
        ingest_identity_spool(dir.path(), &user_book, "router-a");

        assert!(user_book.lock().unwrap().is_empty());
        assert!(!path.exists(), "malformed file is moved, not left in place");
        assert!(dir.path().join("bad.json.bad").exists(), "quarantined to a .bad sidecar");
    }

    #[test]
    fn ingest_identity_spool_missing_dir_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let user_book: Arc<Mutex<UserBook>> = Arc::new(Mutex::new(UserBook::new()));
        // Must not panic even though the spool dir was never created.
        ingest_identity_spool(&missing, &user_book, "router-a");
        assert!(user_book.lock().unwrap().is_empty());
    }
}
