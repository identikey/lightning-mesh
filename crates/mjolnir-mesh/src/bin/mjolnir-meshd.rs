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
use std::sync::{Arc, Mutex};
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
    alloc, merge_subnet_claim, GossipError, GossipSync, GossipTransport, MergeResult, PeerEntry,
    PeerRoster, SubnetClaim, HLC,
};
use mjolnir_mesh::GossipMessage;
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
    tracing_subscriber::fmt()
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
    let l2_backhaul = match &cli.command {
        // Overlay mode: mjolnir0 carries the backhaul address, so don't put it on
        // the underlay iface too. (Overlay also ignores the l2 wired backhaul.)
        Command::Mesh { backhaul_iface, .. } if !overlay_mode => {
            assign_backhaul_addr(backhaul_iface, &secret.public().to_string()).await
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
            let ip = mjolnir_mesh::tun::backhaul_addr(&secret.public().to_string());
            // Pin a well-known port so peers can dial us at a fully-derived
            // address (backhaul_addr + MESH_IROH_PORT), no mDNS needed (0yb.1).
            Some(SocketAddr::new(std::net::IpAddr::V4(ip), MESH_IROH_PORT))
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
                overlay,
                gateway,
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
    overlay: bool,
    gateway: bool,
) -> Result<()> {
    let self_id = endpoint.id();
    let self_id_str = self_id.to_string();
    // NB: the derived IPv4 backhaul address was already assigned to the
    // shared-segment iface in `main`, before the endpoint was built, so iroh
    // picks it up at bind time and mDNS announces it (mjolnir-mesh-4pk).

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
        let (device, link) = spawn_overlay_tun(&self_id_str, OVERLAY_IFACE)
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
    let restored = load_claims(&claims_file);
    if !restored.is_empty() {
        info!(count = restored.len(), path = %claims_file.display(), "restored subnet claims from disk");
    }
    let claims: ClaimStore = Arc::new(Mutex::new(restored));

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
    if lan {
        match endpoint.address_lookup() {
            Ok(services) => {
                let derived = MemoryLookup::new();
                for id in &bootstrap {
                    let addr = SocketAddr::new(
                        std::net::IpAddr::V4(mjolnir_mesh::tun::backhaul_addr(&id.to_string())),
                        MESH_IROH_PORT,
                    );
                    derived.add_endpoint_info(EndpointAddr::new(*id).with_ip_addr(addr));
                    info!(peer = %id, %addr, "seeded derived peer address (no-discovery dialing)");
                }
                services.add(derived);
            }
            Err(e) => {
                warn!("address-lookup services unavailable — cannot seed derived peer addresses: {e}")
            }
        }
    }
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
                tokio::spawn(async move {
                    let result = sync
                        .run(move |msg| {
                            // Log peer claims received over gossip — proves CRDT
                            // convergence (a node seeing another's claim cross the mesh).
                            if let GossipMessage::SubnetClaimUpdate { cidr, entry } = &msg
                                && entry.owner_node_id != me
                            {
                                info!(%cidr, owner = %entry.owner_node_id, "gossip: received peer subnet claim");
                            }
                            let mut s = store.lock().expect("claim store poisoned");
                            if let Some(lost) = apply_subnet_message(&mut s, &msg, &me) {
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
                tokio::spawn(async move {
                    claim_manager(sync, store, me, client_iface, reclaim_rx, neigh_rx).await
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
                tokio::spawn(async move { anti_entropy_loop(sync, store, path).await })
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
                let ip = mjolnir_mesh::tun::backhaul_addr(&peer.to_string());
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
async fn claim_manager<T: GossipTransport>(
    sync: Arc<GossipSync<T>>,
    store: ClaimStore,
    self_id: String,
    client_iface: String,
    mut reclaim_rx: tokio::sync::mpsc::UnboundedReceiver<Ipv4Net>,
    neigh_rx: tokio::sync::watch::Receiver<usize>,
) {
    let has_own_claim = {
        let s = store.lock().expect("claim store poisoned");
        s.values().any(|c| c.owner_node_id == self_id)
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
    claim_and_publish(&sync, &store, &self_id, &client_iface).await;
    while let Some(lost) = reclaim_rx.recv().await {
        // Brief pause so a conflict storm settles before we re-pick.
        tokio::time::sleep(Duration::from_secs(1)).await;
        info!(subnet = %lost, "lost our subnet claim in a conflict — retracting its address and re-claiming");
        retract_client_addr(lost, &client_iface).await;
        claim_and_publish(&sync, &store, &self_id, &client_iface).await;
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
    let mut own: Vec<(Ipv4Net, SubnetClaim)> = store
        .values()
        .filter(|c| c.owner_node_id == self_id)
        .filter_map(|c| match c.cidr {
            IpNet::V4(n) => Some((n, c.clone())),
            IpNet::V6(_) => None,
        })
        .collect();
    let foreign: HashSet<Ipv4Net> = store
        .values()
        .filter(|c| c.owner_node_id != self_id)
        .filter_map(|c| match c.cidr {
            IpNet::V4(n) => Some(n),
            IpNet::V6(_) => None,
        })
        .collect();
    own.sort_by(|a, b| a.1.claimed_at.cmp(&b.1.claimed_at));
    let mut own = own.into_iter();
    let keep = own.next();
    let extras = own.map(|(n, _)| n).collect();
    (keep, extras, foreign)
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

/// Anti-entropy loop (mjolnir-mesh-s9v): every [`ANTI_ENTROPY_INTERVAL`],
/// re-broadcast every claim this node currently knows about — not just its
/// own — and rewrite the on-disk claims file. Re-broadcasting the full map
/// (rather than only our own claim, the weaker form `claim_and_publish`
/// already does) is what lets a late joiner, a node that missed a gossip
/// packet, or a node that just rebooted converge without any pull-based
/// reconciliation protocol.
async fn anti_entropy_loop<T: GossipTransport>(
    sync: Arc<GossipSync<T>>,
    store: ClaimStore,
    claims_file: PathBuf,
) {
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
    }
}

/// Assign this node's claimed /24 gateway address (`<net>.1/prefix`) to the local
/// client interface, giving babeld a concrete *connected* route to redistribute and
/// letting inbound mesh traffic for the /24 be delivered on-link. Replaces the old
/// container-gateway route hop (mjolnir-mesh-e4r): native OpenWrt has no veth
/// gateway — the router is itself on the client L2. Idempotent in effect: an
/// already-present address (EEXIST) is fine. Best-effort: a missing interface is
/// logged, not fatal.
#[cfg(target_os = "linux")]
async fn assign_client_addr(subnet: Ipv4Net, iface: &str) {
    use rtnetlink::new_connection;
    // The router takes `.1` of its claimed /24.
    let gw = Ipv4Addr::from(u32::from(subnet.network()) + 1);
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
    let gw = Ipv4Addr::from(u32::from(subnet.network()) + 1);
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
async fn assign_backhaul_addr(iface: &str, self_id: &str) -> Option<String> {
    use rtnetlink::new_connection;

    let addr = mjolnir_mesh::tun::backhaul_addr(self_id);
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
async fn assign_backhaul_addr(_iface: &str, _self_id: &str) -> Option<String> {
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
                .find(|claim| claim.owner_node_id == self_id)
                .and_then(|claim| match claim.cidr {
                    IpNet::V4(n) => Some(n),
                    IpNet::V6(_) => None,
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
                .find(|claim| claim.owner_node_id == self_id)
                .and_then(|claim| match claim.cidr {
                    IpNet::V4(n) => Some(n),
                    IpNet::V6(_) => None,
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
    let secret = load_or_create_secret(secret_file)?;
    let id = secret.public().to_string();
    let backhaul = mjolnir_mesh::tun::backhaul_addr(&id);
    let prefix = mjolnir_mesh::tun::BACKHAUL_PREFIX_LEN;

    println!("mjolnir-meshd status");
    println!("  build:    {}", env!("MJOLNIR_BUILD"));
    println!("  version:  {}", env!("CARGO_PKG_VERSION"));
    println!("  node id:  {id}");
    println!("  backhaul: {backhaul}/{prefix}  (derived from node id)");
    println!();
    print_system_status(backhaul).await;
    Ok(())
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
async fn print_system_status(backhaul: Ipv4Addr) {
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
        let has_backhaul = list.iter().any(|(a, _)| *a == backhaul);
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
    if !backhaul_seen {
        println!(
            "  WARNING: derived backhaul {backhaul} is not assigned on any interface \
             (daemon not running, or the backhaul interface is down)"
        );
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
async fn print_system_status(_backhaul: Ipv4Addr) {
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
}
