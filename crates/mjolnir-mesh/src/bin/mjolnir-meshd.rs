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
use iroh::endpoint::presets;
use iroh::endpoint::Connection;
use iroh_mdns_address_lookup::MdnsAddressLookup;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey};
use iroh_gossip::api::{Event, GossipReceiver, GossipSender};
use iroh_gossip::{Gossip, TopicId};
use mjolnir_mesh::tun::{spawn_tunnel, DatagramConn, EncapError, Tunnel};
use mjolnir_mesh::babel::{
    render_babeld_conf, write_atomic_if_changed, BabelConfigInputs, BabelSupervisor,
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
        /// babeld binary to supervise (PATH name or absolute path).
        #[arg(long, default_value = "babeld")]
        babeld: PathBuf,
        /// The container's gateway to the router (veth peer). When this node
        /// claims a /24, meshd installs a kernel route for it via this gateway,
        /// so babeld can redistribute the /24 and inbound mesh traffic reaches
        /// the router for local-client delivery. Matches container-net.rsc.
        #[arg(long, default_value = "172.20.0.1")]
        client_gateway: Ipv4Addr,
        /// The container interface on the shared L2 segment (the veth facing the
        /// other mesh nodes). meshd self-assigns this node's derived IPv4 backhaul
        /// address here so peers discover + connect directly over the LAN, no DHCP.
        #[arg(long, default_value = "eth0")]
        backhaul_iface: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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

    // The deployed `mesh` daemon defaults to LAN mode (offline, mDNS, no relay),
    // since the same-site mesh has no internet. Opt into internet/relay mode with
    // `--internet` or by passing `--relay`. The lower-level test commands
    // (listen/connect/id) keep their explicit-`--lan` behaviour unchanged.
    let mesh_mode = matches!(cli.command, Command::Mesh { .. });
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
    if let Command::Mesh { backhaul_iface, .. } = &cli.command {
        assign_backhaul_addr(backhaul_iface, &secret.public().to_string()).await;
    }
    let endpoint = build_endpoint(secret, no_relay, cli.bind, lan, &cli.relay).await?;

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
            babeld,
            client_gateway,
            // backhaul_iface was used before bind in `main`.
            backhaul_iface: _,
        } => {
            run_mesh(
                endpoint,
                no_relay,
                roster,
                peer,
                babel_config,
                babeld,
                client_gateway,
            )
            .await?
        }
        Command::TunTest => unreachable!("handled above"),
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
/// dispatch loop ever reads it).
struct IrohGossipTransport {
    sender: GossipSender,
    receiver: tokio::sync::Mutex<GossipReceiver>,
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
                // Only `Received` carries an application payload; neighbor
                // up/down and lag notifications are control events we skip.
                Some(Ok(Event::Received(msg))) => return Ok(msg.content),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(GossipError::Transport(e.to_string())),
                None => return Err(GossipError::Closed),
            }
        }
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
async fn run_mesh(
    endpoint: Endpoint,
    no_relay: bool,
    roster_path: Option<PathBuf>,
    peer_args: Vec<String>,
    babel_config: PathBuf,
    babeld: PathBuf,
    client_gateway: Ipv4Addr,
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
    // Shared CRDT subnet-claim store (mjolnir-mesh-chn): cidr -> claim. Written
    // by the gossip apply loop and the local claim routine; babeld (83k) reads
    // it for the local subnet to redistribute.
    let claims: ClaimStore = Arc::new(Mutex::new(HashMap::new()));

    // CRDT gossip overlay (mjolnir-mesh-k8c): all mesh nodes join one fixed
    // topic and exchange CRDT updates best-effort, as a second protocol on the
    // same endpoint alongside the TUN data plane.
    let gossip = Gossip::builder().spawn(endpoint.clone());

    // Accept inbound tunnels (peers with a higher node id dial in) and gossip.
    let router = Router::builder(endpoint.clone())
        .accept(
            TUN_ALPN,
            TunnelHandler {
                self_id: self_id_str.clone(),
                registry: registry.clone(),
            },
        )
        .accept(iroh_gossip::ALPN, gossip.clone())
        .spawn();

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
    let (gossip_dispatch, claim_task) = match gossip.subscribe(mesh_topic_id(), bootstrap).await {
        Ok(topic) => {
            let (sender, receiver) = topic.split();
            let sync = Arc::new(GossipSync::new(IrohGossipTransport {
                sender,
                receiver: tokio::sync::Mutex::new(receiver),
            }));
            info!("gossip overlay joined (mesh CRDT topic)");

            // Signalled by the apply loop when a conflict costs us our claim.
            let (reclaim_tx, reclaim_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

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
                            if apply_subnet_message(&mut s, &msg, &me) {
                                drop(s);
                                let _ = reclaim_tx.send(());
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
                tokio::spawn(async move {
                    claim_manager(sync, store, me, client_gateway, reclaim_rx).await
                })
            };

            (Some(dispatch), Some(claim))
        }
        Err(e) => {
            warn!("gossip subscribe failed: {e}; continuing without CRDT overlay");
            (None, None)
        }
    };

    // babeld supervision (mjolnir-mesh-83k): a reconciler regenerates babeld.conf
    // from the live tunnel set (TunnelRegistry) plus our subnet claim (ClaimStore)
    // and starts/SIGHUPs babeld as they change. babeld absence is non-fatal.
    let babel_sup = Arc::new(BabelSupervisor::new(babel_config.clone(), babeld));
    let babel_task = {
        let sup = babel_sup.clone();
        let registry = registry.clone();
        let claims = claims.clone();
        let me = self_id_str.clone();
        tokio::spawn(
            async move { babel_reconciler(sup, registry, claims, me, babel_config).await },
        )
    };

    // Spawn one dialer task per peer we initiate to. Tie-break by node id so
    // exactly one side of each pair dials (the lexicographically-lower id) and
    // the other accepts — otherwise both ends would race to create the same
    // deterministic /31 interface. This mirrors `pick_link_31`'s ordering.
    let mut dialers = Vec::new();
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
        if self_id_str < peer.to_string() {
            let ep = endpoint.clone();
            let reg = registry.clone();
            let sid = self_id_str.clone();
            let label = entry.label.clone();
            dialers.push(tokio::spawn(async move {
                connector_loop(ep, addr, sid, reg, label).await;
            }));
        } else {
            info!(%peer, label = ?entry.label, "peer has the higher id — waiting for it to dial us");
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
    babel_task.abort();
    if let Err(e) = babel_sup.shutdown().await {
        warn!("babeld shutdown error: {e}");
    }
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

/// Subnet-claim warmup: after joining gossip, wait this long to learn existing
/// claims before publishing our own, so a fresh node doesn't stomp an
/// established claim. (Same-site local-peer detection — claim_cooldown — is a
/// separate, future concern.)
const CLAIM_WARMUP: Duration = Duration::from_secs(8);

/// Client-subnet size each router claims from the mesh space (10.42.0.0/16).
const CLIENT_PREFIX_LEN: u8 = 24;

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

/// Apply an inbound subnet CRDT message to the claim store. Returns `true` if
/// THIS node lost its own claim in a conflict and must re-claim. Pure over the
/// map (no I/O) so it's unit-tested below.
fn apply_subnet_message(
    store: &mut HashMap<String, SubnetClaim>,
    msg: &GossipMessage,
    self_id: &str,
) -> bool {
    match msg {
        GossipMessage::SubnetClaimUpdate { cidr, entry } => {
            match merge_subnet_claim(store.get(cidr), entry) {
                MergeResult::Inserted | MergeResult::Updated => {
                    store.insert(cidr.clone(), entry.clone());
                    false
                }
                MergeResult::Unchanged => false,
                MergeResult::Conflict { winner, loser } => {
                    let we_lost =
                        loser.owner_node_id == self_id && winner.owner_node_id != self_id;
                    store.insert(cidr.clone(), winner);
                    we_lost
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
            false
        }
        // Lease/DNS/Service CRDT messages are out of scope for the subnet claim.
        _ => false,
    }
}

/// Manage this node's subnet claim: after a warmup to learn existing claims,
/// pick a free /24 and publish it; re-claim whenever a conflict costs us ours.
async fn claim_manager<T: GossipTransport>(
    sync: Arc<GossipSync<T>>,
    store: ClaimStore,
    self_id: String,
    client_gateway: Ipv4Addr,
    mut reclaim_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    tokio::time::sleep(CLAIM_WARMUP).await;
    claim_and_publish(&sync, &store, &self_id, client_gateway).await;
    while reclaim_rx.recv().await.is_some() {
        // Brief pause so a conflict storm settles before we re-pick.
        tokio::time::sleep(Duration::from_secs(1)).await;
        info!("lost our subnet claim in a conflict — re-claiming");
        claim_and_publish(&sync, &store, &self_id, client_gateway).await;
    }
}

/// Pick a free /24 (avoiding known claims), record it, install its local route
/// (so babeld can redistribute it), and gossip the claim.
async fn claim_and_publish<T: GossipTransport>(
    sync: &GossipSync<T>,
    store: &ClaimStore,
    self_id: &str,
    client_gateway: Ipv4Addr,
) {
    let claimed: HashSet<Ipv4Net> = {
        let s = store.lock().expect("claim store poisoned");
        s.values()
            .filter_map(|c| match c.cidr {
                IpNet::V4(n) => Some(n),
                IpNet::V6(_) => None,
            })
            .collect()
    };
    let net = match alloc::pick_subnet_or_smaller(
        self_id,
        &claimed,
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

    // Install the kernel route for our /24 toward the router, so babeld has a
    // concrete route to redistribute and inbound mesh traffic for it reaches
    // the local LAN (mjolnir-mesh-df4).
    install_client_route(net, client_gateway).await;
}

/// Install a kernel route `subnet via gateway` in this (container) netns. Gives
/// babeld a concrete route to redistribute for our claimed /24 and routes
/// inbound mesh traffic for it back to the router for local-client delivery.
/// Idempotent in effect: an already-present route (EEXIST) is fine.
#[cfg(target_os = "linux")]
async fn install_client_route(subnet: Ipv4Net, gateway: Ipv4Addr) {
    use rtnetlink::{new_connection, RouteMessageBuilder};
    let (connection, handle, _) = match new_connection() {
        Ok(c) => c,
        Err(e) => {
            warn!(%subnet, "netlink connect for client route failed: {e}");
            return;
        }
    };
    tokio::spawn(connection);
    let route = RouteMessageBuilder::<Ipv4Addr>::new()
        .destination_prefix(subnet.network(), subnet.prefix_len())
        .gateway(gateway)
        .build();
    match handle.route().add(route).execute().await {
        Ok(()) => info!(%subnet, %gateway, "installed client subnet route"),
        Err(e) => warn!(%subnet, %gateway, "could not install client route (may already exist): {e}"),
    }
}

#[cfg(not(target_os = "linux"))]
async fn install_client_route(_subnet: Ipv4Net, _gateway: Ipv4Addr) {}

/// Self-assign this node's derived IPv4 backhaul address (`10.254.0.0/16`, host
/// from the node id) to the shared-segment interface, so every node has a stable,
/// collision-free, DHCP-free underlay address in one shared /16. Peers are then
/// on-link to each other and iroh/mDNS discover + connect directly over the LAN
/// (mjolnir-mesh-4pk). IPv4 (not an IPv6 ULA) because iroh surfaces private IPv4
/// as a connection candidate and announces it over mDNS, but not IPv6 ULAs — see
/// the `iroh-lan-backhaul-findings` memory. Best-effort: an unreachable interface
/// or an already-present address is logged, not fatal — the node still runs.
#[cfg(target_os = "linux")]
async fn assign_backhaul_addr(iface: &str, self_id: &str) {
    use rtnetlink::new_connection;

    let addr = mjolnir_mesh::tun::backhaul_addr(self_id);
    let prefix = mjolnir_mesh::tun::BACKHAUL_PREFIX_LEN;

    let (connection, handle, _) = match new_connection() {
        Ok(c) => c,
        Err(e) => {
            warn!(%addr, "netlink connect for backhaul address failed: {e}");
            return;
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
            return;
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
}

#[cfg(not(target_os = "linux"))]
async fn assign_backhaul_addr(_iface: &str, _self_id: &str) {}

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

/// Reconcile babeld against live mesh state. Every few seconds it renders
/// babeld.conf from the current tunnel interfaces ([`TunnelRegistry`]) and our
/// local subnet claim ([`ClaimStore`]); it starts babeld once there's a tunnel
/// to route over and SIGHUPs it whenever the rendered config changes. babeld
/// being absent or unstartable is non-fatal — routing is disabled but the TUN
/// data plane keeps running.
async fn babel_reconciler(
    sup: Arc<BabelSupervisor>,
    registry: TunnelRegistry,
    claims: ClaimStore,
    self_id: String,
    config_path: PathBuf,
) {
    if let Some(parent) = config_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!("could not create babeld config dir {}: {e}", parent.display());
    }

    let mut spawned = false;
    let mut babeld_unavailable = false;
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
        let inputs = BabelConfigInputs::new(local_subnet, &iface_refs);
        let conf = render_babeld_conf(&inputs);

        match write_atomic_if_changed(&config_path, &conf) {
            Ok(changed) => {
                if ifaces.is_empty() {
                    // babeld refuses to run with zero interfaces ("Eek... asked to
                    // run on no interfaces!") and exits. When no tunnel is up, keep
                    // babeld stopped rather than restart-looping it into an empty
                    // config; it starts again once a tunnel reappears.
                    if spawned {
                        warn!("no live tunnels — stopping babeld until one returns");
                        let _ = sup.shutdown().await;
                        spawned = false;
                    }
                } else if !spawned && !babeld_unavailable {
                    // Start babeld once there's at least one tunnel to route over.
                    match sup.spawn().await {
                        Ok(()) => {
                            spawned = true;
                            info!(config = %config_path.display(), ifaces = ifaces.len(), "babeld started");
                        }
                        Err(e) => {
                            babeld_unavailable = true;
                            warn!("could not start babeld (cross-site routing disabled): {e}");
                        }
                    }
                } else if spawned {
                    // babeld 1.13 exits on a SIGHUP reload in this container, so
                    // RESTART on config change rather than signal — and respawn it
                    // if it has died (the reconciler is also the keep-alive).
                    if sup.has_exited().await {
                        warn!("babeld exited — restarting");
                        if let Err(e) = sup.restart().await {
                            warn!("babeld restart failed: {e}");
                        }
                    } else if changed {
                        info!("babeld config changed — restarting babeld");
                        if let Err(e) = sup.restart().await {
                            warn!("babeld restart failed: {e}");
                        }
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
        let mut builder = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .secret_key(secret)
            .transport_config(tunnel_transport_config())
            .address_lookup(MdnsAddressLookup::builder());
        if let Some(addr) = bind {
            builder = builder.bind_addr(addr).context("invalid --bind address")?;
        }
        return builder.bind().await.context("failed to bind iroh endpoint");
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
        assert!(!reclaim);
        assert_eq!(store["10.42.1.0/24"].owner_node_id, "peer-b");
    }

    #[test]
    fn same_owner_newer_updates_no_reclaim() {
        let mut store = HashMap::new();
        store.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 100));
        let newer = claim("10.42.1.0/24", "peer-b", 200);
        let reclaim = apply_subnet_message(&mut store, &update(&newer), "self");
        assert!(!reclaim);
        assert_eq!(store["10.42.1.0/24"].claimed_at.wall_clock, 200);
    }

    #[test]
    fn older_claim_is_unchanged() {
        let mut store = HashMap::new();
        store.insert("10.42.1.0/24".to_string(), claim("10.42.1.0/24", "peer-b", 200));
        let older = claim("10.42.1.0/24", "peer-b", 100);
        let reclaim = apply_subnet_message(&mut store, &update(&older), "self");
        assert!(!reclaim);
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
        assert!(reclaim, "we should re-claim after losing our subnet");
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
        assert!(!reclaim);
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
        assert!(!reclaim);
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
}
