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
mod plan;
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
    /// Path to the router inventory. When omitted, `deploy/mikrotik/routers.toml`
    /// is searched for upward from the current directory, so meshctl works from
    /// anywhere in the repo (not just the root).
    #[arg(long, global = true)]
    inventory: Option<PathBuf>,

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
    /// Run a RouterOS query and print the parsed records — the observe layer's
    /// introspection primitive. Exercises the exact script-gen + parse path the
    /// reconciler uses. E.g. `meshctl query router-1 /interface/veth`.
    Query {
        /// Router name.
        name: String,
        /// Menu path, e.g. `/interface/veth` or `/ip/firewall/nat`.
        path: String,
        /// Comma-separated fields to fetch (default: name,comment). NOTE: a
        /// field that doesn't exist on the menu makes RouterOS abort the loop.
        #[arg(long, value_delimiter = ',')]
        fields: Option<Vec<String>>,
        /// Optional `where` filter, e.g. `comment~"mjolnir"`.
        #[arg(long)]
        filter: Option<String>,
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
    let inv_path = resolve_inventory(cli.inventory)?;
    let inv = Inventory::load(&inv_path)?;

    match cli.command {
        Command::List => cmd_list(&inv),
        Command::Ping { name, all } => cmd_ping(&inv, name.as_deref(), all).await,
        Command::Query {
            name,
            path,
            fields,
            filter,
        } => cmd_query(&inv, &name, &path, fields, filter.as_deref()).await,
        Command::Bootstrap { name, all, pubkey } => {
            cmd_bootstrap(&inv, name.as_deref(), all, pubkey.as_deref()).await
        }
        Command::Plan { name, all } => cmd_plan(&inv, name.as_deref(), all).await,
        Command::Apply { .. } => not_yet("apply", "M3 (mjolnir-mesh-65e)"),
        Command::Deploy { .. } => not_yet("deploy", "M4 (mjolnir-mesh-2p1)"),
    }
}

fn not_yet(cmd: &str, milestone: &str) -> Result<()> {
    bail!("`{cmd}` is not implemented yet — landing in {milestone}");
}

/// Resolve the inventory path. An explicit `--inventory` is used verbatim (and
/// must exist); otherwise `deploy/mikrotik/routers.toml` is searched for upward
/// from the current directory so meshctl works anywhere in the repo.
fn resolve_inventory(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if !p.exists() {
            bail!("--inventory {} does not exist", p.display());
        }
        return Ok(p);
    }
    let cwd = std::env::current_dir().context("getting current directory")?;
    find_upward(&cwd, Path::new(inventory::DEFAULT_PATH)).with_context(|| {
        format!(
            "no {} found in {} or any parent directory (pass --inventory to point elsewhere)",
            inventory::DEFAULT_PATH,
            cwd.display()
        )
    })
}

/// Walk up from `start`, returning the first existing `dir/suffix`.
fn find_upward(start: &Path, suffix: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(suffix);
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
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

async fn cmd_query(
    inv: &Inventory,
    name: &str,
    path: &str,
    fields: Option<Vec<String>>,
    filter: Option<&str>,
) -> Result<()> {
    let r = inv
        .get(name)
        .with_context(|| format!("no router named {name:?} in the inventory"))?;
    let fields = fields.unwrap_or_else(|| vec!["name".into(), "comment".into()]);
    let field_refs: Vec<&str> = fields.iter().map(String::as_str).collect();

    let ssh = Ssh::new(r.ssh_target(inv));
    let records = routeros::query(&ssh, path, filter, &field_refs).await?;

    println!("{} {} record(s) under {path}", r.name, records.len());
    for (i, rec) in records.iter().enumerate() {
        let line = field_refs
            .iter()
            .map(|f| format!("{f}={}", rec.get(*f).map(String::as_str).unwrap_or("")))
            .collect::<Vec<_>>()
            .join("  ");
        println!("  [{i}] {line}");
    }
    Ok(())
}

async fn cmd_plan(inv: &Inventory, name: Option<&str>, all: bool) -> Result<()> {
    use plan::Status;
    let targets = select(inv, name, all)?;
    for r in targets {
        let ssh = Ssh::new(r.ssh_target(inv));
        let (entries, prunes) = plan::plan_router(&ssh, inv, r).await?;

        println!("\n{} plan ({} resources):", r.name, entries.len());
        let (mut conv, mut miss, mut drift, mut conf) = (0u32, 0u32, 0u32, 0u32);
        for e in &entries {
            let label = match &e.status {
                Status::Missing => {
                    miss += 1;
                    "MISSING".to_string()
                }
                Status::Converged => {
                    conv += 1;
                    "CONVERGED".to_string()
                }
                Status::Drifted(diffs) => {
                    drift += 1;
                    let detail = diffs
                        .iter()
                        .map(|d| format!("{}: want {:?} got {:?}", d.field, d.want, d.got))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("DRIFTED ({detail})")
                }
                Status::Conflict(n) => {
                    conf += 1;
                    format!("CONFLICT ({n} live matches)")
                }
            };
            println!("  {:<11} {:<26} {label}", e.kind, e.id);
        }
        for p in &prunes {
            println!("  {:<11} {:<26} PRUNE (leftover)", p.kind, p.comment);
        }
        println!(
            "summary: {conv} converged, {miss} missing, {drift} drifted, {conf} conflict, {} prune",
            prunes.len()
        );
    }
    // `plan` is observe-only; it always exits 0 regardless of drift.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_upward_locates_from_subdir() {
        let root = tempfile::tempdir().unwrap();
        let suffix = Path::new("deploy/mikrotik/routers.toml");
        let target = root.path().join(suffix);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "default_user = \"admin\"\n").unwrap();

        // Search from a nested dir well below the inventory.
        let deep = root.path().join("crates/mjolnir-meshctl/src");
        std::fs::create_dir_all(&deep).unwrap();

        let found = find_upward(&deep, suffix).unwrap();
        assert_eq!(found.canonicalize().unwrap(), target.canonicalize().unwrap());
    }

    #[test]
    fn find_upward_none_when_absent() {
        let root = tempfile::tempdir().unwrap();
        assert!(find_upward(root.path(), Path::new("deploy/mikrotik/routers.toml")).is_none());
    }
}
