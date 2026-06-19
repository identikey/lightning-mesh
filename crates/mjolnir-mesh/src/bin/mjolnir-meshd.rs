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

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bytes::Bytes;
use clap::{Parser, Subcommand};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// ALPN for the P0 mesh connectivity probe. Bumped per protocol revision.
const MESH_ALPN: &[u8] = b"mjolnir/mesh/v0";

/// Datagram payload used to prove an end-to-end round-trip.
const PING: &[u8] = b"mjolnir-ping";

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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let endpoint = build_endpoint(cli.secret_file.as_deref(), cli.no_relay, cli.bind).await?;

    match cli.command {
        Command::Id => {
            wait_until_addressable(&endpoint, cli.no_relay).await;
            print_identity(&endpoint)?;
        }
        Command::Listen => run_listen(endpoint, cli.no_relay).await?,
        Command::Connect { addr } => run_connect(endpoint, &addr).await?,
    }
    Ok(())
}

/// Build an iroh endpoint with a persisted (or ephemeral) identity. Relays are
/// on by default (they provide NAT traversal off-LAN); `--no-relay` forces
/// direct/LAN-only, and `--bind` pins the socket address.
async fn build_endpoint(
    secret_file: Option<&Path>,
    no_relay: bool,
    bind: Option<SocketAddr>,
) -> Result<Endpoint> {
    let secret = load_or_create_secret(secret_file)?;
    let mut builder = Endpoint::builder().secret_key(secret);
    if no_relay {
        builder = builder.relay_mode(RelayMode::Disabled);
    }
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
    let addr = decode_addr(addr_blob).context("decoding peer address blob")?;
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
        let secret = SecretKey::generate(&mut rand::rng());
        std::fs::write(p, encode_secret_hex(&secret))
            .with_context(|| format!("writing secret file {}", p.display()))?;
        info!(path = %p.display(), id = %secret.public(), "generated new node identity");
        return Ok(secret);
    }

    if let Ok(env) = std::env::var("IROH_SECRET") {
        return env.parse::<SecretKey>().context("parsing IROH_SECRET");
    }

    warn!("no --secret-file or IROH_SECRET set; using an ephemeral identity");
    Ok(SecretKey::generate(&mut rand::rng()))
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
