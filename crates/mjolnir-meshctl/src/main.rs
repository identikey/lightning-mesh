//! `meshctl` — operator-side reconciler for the mjolnir mesh router swarm.
//!
//! Runs on the operator's machine (NOT in the container) and drives MikroTik
//! RouterOS routers to a declared mesh state — modeled on the `Mjolnir.Forge`
//! reconciler. Transport is SSH-only: RouterOS's REST API is HTTP-basic-auth
//! only (no key/token auth), so instead of a password + www-ssl bootstrap we
//! reconcile over the passwordless SSH channel — observe via generated
//! `:foreach`/`:put` query scripts, apply via idempotent `:if find` snippets
//! (mirroring `deploy/mikrotik/container-net.rsc`). See bd memory
//! `meshctl-transport-decision-2026-06-21-ssh-only`.
//!
//! Milestones (beads mjolnir-mesh-xh5 and children):
//!   M0  list, ping            — inventory + SSH transport            [done]
//!   M1  bootstrap, query      — SSH key import + RouterOS query layer
//!   M2  plan                  — observe + diff (no mutation)
//!   M3  apply                 — converge config (comment-tag ownership)
//!   M4  deploy / --all        — tar upload + container add/start + swarm

// The inventory + SSH transport expose the full surface the later milestones
// consume (peer resolution, file upload, interactive bootstrap). Until M1–M4
// wire them in, those items read as dead code. Remove this once M4 lands.
#![allow(dead_code)]

mod inventory;
mod routeros;
mod ssh;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use tracing::warn;
use tracing_subscriber::EnvFilter;

use inventory::{Inventory, Router};
use ssh::Ssh;

#[derive(Parser)]
#[command(name = "meshctl", about = "Operator-side RouterOS reconciler for the mjolnir mesh swarm")]
struct Cli {
    /// Path to the router inventory.
    #[arg(long, global = true, default_value = inventory::DEFAULT_PATH)]
    inventory: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the parsed inventory (validates names, roles, peer references).
    List,
    /// Verify SSH reachability of one router (or `--all`) by asking RouterOS its
    /// identity. Confirms key auth + connectivity before plan/apply/deploy.
    Ping {
        /// Router name. Omit with `--all` to ping the whole swarm.
        name: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// One-time: install the operator SSH public key on the router for
    /// passwordless auth. Prompts for the router password once (the only
    /// interactive step in the whole workflow).
    Bootstrap {
        /// Router name. Omit with `--all` to bootstrap the whole swarm.
        name: Option<String>,
        #[arg(long)]
        all: bool,
        /// Public key to install. Defaults to ~/.ssh/id_ed25519.pub, then
        /// ~/.ssh/id_rsa.pub.
        #[arg(long)]
        pubkey: Option<PathBuf>,
    },
    /// [M2] Observe live state and report drift (no mutation).
    Plan {
        name: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// [M3] Converge the router to its declared config.
    Apply {
        name: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// [M4] Apply + upload tar + add/start the container + reachability check.
    Deploy {
        name: Option<String>,
        #[arg(long)]
        all: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // `{:#}` renders the full anyhow context chain on one line.
            eprintln!("meshctl: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let inv = Inventory::load(&cli.inventory)?;

    match cli.command {
        Command::List => cmd_list(&inv),
        Command::Ping { name, all } => cmd_ping(&inv, name.as_deref(), all).await,
        Command::Bootstrap { name, all, pubkey } => {
            cmd_bootstrap(&inv, name.as_deref(), all, pubkey.as_deref()).await
        }
        Command::Plan { .. } => not_yet("plan", "M2 (mjolnir-mesh-cax)"),
        Command::Apply { .. } => not_yet("apply", "M3 (mjolnir-mesh-65e)"),
        Command::Deploy { .. } => not_yet("deploy", "M4 (mjolnir-mesh-2p1)"),
    }
}

fn not_yet(cmd: &str, milestone: &str) -> Result<()> {
    bail!("`{cmd}` is not implemented yet — landing in {milestone}");
}

fn cmd_list(inv: &Inventory) -> Result<()> {
    if inv.routers.is_empty() {
        println!("(inventory is empty)");
        return Ok(());
    }
    println!(
        "{} router(s); default user={} subnet={}",
        inv.routers.len(),
        inv.default_user,
        inv.default_subnet
    );
    for r in &inv.routers {
        let peer = match (&r.peer, &r.peer_blob) {
            (Some(p), _) => format!(" peer={p}"),
            (None, Some(_)) => " peer=<blob>".to_string(),
            (None, None) => String::new(),
        };
        println!(
            "  {:<12} {:<18} {:<10} subnet={}{}",
            r.name,
            r.ssh_target(inv),
            r.role,
            r.subnet(inv),
            peer
        );
    }
    Ok(())
}

/// Resolve the set of routers a command targets from `(name, --all)`.
fn select<'a>(inv: &'a Inventory, name: Option<&str>, all: bool) -> Result<Vec<&'a Router>> {
    match (name, all) {
        (Some(_), true) => bail!("pass a router name OR --all, not both"),
        (None, false) => bail!("specify a router name, or --all"),
        (Some(n), false) => {
            let r = inv
                .get(n)
                .with_context(|| format!("no router named {n:?} in the inventory"))?;
            Ok(vec![r])
        }
        (None, true) => Ok(inv.routers.iter().collect()),
    }
}

async fn cmd_ping(inv: &Inventory, name: Option<&str>, all: bool) -> Result<()> {
    let targets = select(inv, name, all)?;
    let mut failures = 0;
    for r in targets {
        let ssh = Ssh::new(r.ssh_target(inv));
        // `:put` writes a bare value to stdout — the router's identity name —
        // which both proves the SSH command channel works and is human-friendly.
        match ssh.run(":put [/system/identity/get name]").await {
            Ok(out) => println!("  {:<12} OK   identity={}", r.name, out.trim()),
            Err(e) => {
                println!("  {:<12} FAIL {e:#}", r.name);
                failures += 1;
            }
        }
    }
    if failures > 0 {
        bail!("{failures} router(s) unreachable over SSH");
    }
    Ok(())
}

/// Locate the operator public key to install: explicit `--pubkey`, else the
/// usual `~/.ssh` defaults.
fn resolve_pubkey(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if !p.exists() {
            bail!("--pubkey {} does not exist", p.display());
        }
        return Ok(p.to_path_buf());
    }
    let home = std::env::var("HOME").context("HOME not set; pass --pubkey")?;
    for cand in ["id_ed25519.pub", "id_rsa.pub"] {
        let p = Path::new(&home).join(".ssh").join(cand);
        if p.exists() {
            return Ok(p);
        }
    }
    bail!("no SSH public key found in ~/.ssh (id_ed25519.pub / id_rsa.pub); pass --pubkey")
}

async fn cmd_bootstrap(
    inv: &Inventory,
    name: Option<&str>,
    all: bool,
    pubkey: Option<&Path>,
) -> Result<()> {
    let targets = select(inv, name, all)?;
    let pubkey = resolve_pubkey(pubkey)?;
    let remote_name = pubkey
        .file_name()
        .and_then(|s| s.to_str())
        .context("pubkey path has no file name")?
        .to_string();
    println!("installing {} on {} router(s)", pubkey.display(), targets.len());

    let mut failures = 0;
    for r in targets {
        println!("\n── {} ({}) ──", r.name, r.ssh_target(inv));
        match bootstrap_one(inv, r, &pubkey, &remote_name).await {
            Ok(()) => println!("  {} bootstrapped — passwordless SSH confirmed", r.name),
            Err(e) => {
                println!("  {} FAILED: {e:#}", r.name);
                failures += 1;
            }
        }
    }
    if failures > 0 {
        bail!("{failures} router(s) failed to bootstrap");
    }
    Ok(())
}

async fn bootstrap_one(
    inv: &Inventory,
    r: &Router,
    pubkey: &Path,
    remote_name: &str,
) -> Result<()> {
    let user = r.user(inv);
    let target = r.ssh_target(inv);

    // Interactive channel: the key isn't installed yet, so these two steps
    // authenticate by password (prompted once on the terminal).
    let interactive = Ssh::new(&target).interactive();
    println!("  uploading {remote_name} (you may be prompted for the router password)…");
    interactive
        .upload(pubkey, "")
        .await
        .context("uploading public key")?;
    println!("  importing key for user {user}…");
    interactive
        .run_interactive(&format!(
            "/user/ssh-keys/import public-key-file={remote_name} user={user}"
        ))
        .await
        .context("importing public key")?;

    // From here on, batch mode must succeed without a password — that *is* the
    // verification that the key took.
    let batch = Ssh::new(&target);
    let identity = batch
        .run(":put [/system/identity/get name]")
        .await
        .context("passwordless verification failed after key import")?;
    println!("  verified identity={} (no password)", identity.trim());

    // Tidy up the uploaded key file; import already copied it into the key
    // store, so the leftover in root/ is just clutter. Best-effort.
    if let Err(e) = batch
        .run(&format!("/file/remove [find where name=\"{remote_name}\"]"))
        .await
    {
        warn!("could not remove uploaded {remote_name} from router: {e:#}");
    }
    Ok(())
}
