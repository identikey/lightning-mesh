//! M4: `deploy` — the whole flow from a converged router to a running mesh
//! container, replacing the manual teardown→upload→add→start→watch dance.
//!
//! Per router: converge the network (M3 apply) → scp the container tar → add +
//! start the container with the right `cmd` (`tun-listen` for a listener,
//! `tun-connect <blob>` for a connector) → wait for it to run and tail the
//! startup log for the reachability line. For `--all`, listeners deploy first
//! so a connector can read its peer's live address blob.
//!
//! root-dir lives on internal flash (the boards have no USB but ~86 MB free);
//! the container is tagged `comment="mjolnir-meshd"` so redeploys are idempotent.
//!
//! Prerequisite that CANNOT be scripted: device-mode containers must be enabled
//! (physical reset-button hold at boot). `deploy` checks it and fails with a
//! clear message rather than a cryptic `/container/add` error.

use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

use crate::apply;
use crate::inventory::{Inventory, Role, Router};
use crate::routeros;
use crate::ssh::Ssh;

/// Comment tag identifying the mesh container (for idempotent redeploy).
const CONTAINER_COMMENT: &str = "mjolnir-meshd";
/// RouterOS container env-list name holding the node's IROH_SECRET.
const ENV_LIST: &str = "mjolnir-env";

/// Get (or generate) a stable per-router node secret (64-hex = 32 bytes), so
/// the mesh identity — and therefore the address blob — survives container
/// restarts and redeploys. Without this meshd falls back to an ephemeral key
/// and the blob changes every run, which no connector can target. Stored under
/// a gitignored `secrets/` dir beside the inventory; passed to the container as
/// the `IROH_SECRET` env var.
pub fn ensure_secret(secrets_dir: &Path, router: &str) -> Result<String> {
    let path = secrets_dir.join(format!("{router}.secret"));
    if path.exists() {
        let s = std::fs::read_to_string(&path)
            .with_context(|| format!("reading secret {}", path.display()))?
            .trim()
            .to_string();
        if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(s);
        }
        bail!("secret {} is not 64 hex chars", path.display());
    }
    let mut buf = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .context("reading /dev/urandom for a new node secret")?;
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(secrets_dir)
        .with_context(|| format!("creating {}", secrets_dir.display()))?;
    std::fs::write(&path, format!("{hex}\n"))
        .with_context(|| format!("writing secret {}", path.display()))?;
    info!(
        "generated persistent node secret for {router} at {}",
        path.display()
    );
    Ok(hex)
}

#[derive(Debug, Clone)]
pub struct DeployOpts {
    /// Container DNS (the masquerade gives it egress; it still needs a resolver).
    pub dns: String,
    /// root-dir for the container's extracted rootfs (internal flash dir).
    pub root_dir: String,
    /// How long to wait for extraction / running / a blob before giving up.
    pub timeout: Duration,
}

impl Default for DeployOpts {
    fn default() -> Self {
        Self {
            dns: "1.1.1.1".into(),
            root_dir: "mjolnir".into(),
            timeout: Duration::from_secs(90),
        }
    }
}

/// Verify device-mode containers are enabled — the one un-scriptable physical
/// prerequisite. Fails loudly rather than letting `/container/add` error
/// cryptically.
pub async fn check_container_capable(ssh: &Ssh) -> Result<()> {
    let out = ssh.run(":put [/system/device-mode/get container]").await?;
    if out.trim() == "true" {
        Ok(())
    } else {
        bail!(
            "device-mode containers not enabled (got {:?}). This requires a \
             physical reset-button hold at boot — it CANNOT be done over SSH. \
             See docs/deploy/mikrotik-routeros-container.md.",
            out.trim()
        )
    }
}

/// Container run-state. RouterOS has no `status` property and no status column
/// — state is the boolean `stopped` field (true = stopped/ready, false =
/// running), shown as the `S` flag in `/container/print`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Stopped,
    Running,
}

/// The mesh container's run-state, or `None` if it isn't present.
///
/// RouterOS only emits whichever flag is active: a running container has
/// `running=true` and *no* `stopped` field; a stopped one has `stopped=true`
/// and no `running`. `get` of the inactive flag returns empty (not an error),
/// so we query both and key off `running`.
async fn container_state(ssh: &Ssh) -> Result<Option<State>> {
    let recs = routeros::query(
        ssh,
        "/container",
        Some(&format!(r#"comment="{CONTAINER_COMMENT}""#)),
        &["running", "stopped"],
    )
    .await?;
    Ok(recs.first().map(|r| {
        if r.get("running").map(String::as_str) == Some("true") {
            State::Running
        } else {
            State::Stopped
        }
    }))
}

/// Poll until the container is present (post-add), or timeout.
async fn wait_present(ssh: &Ssh, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if container_state(ssh).await?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("container did not appear after add (extraction failed?)");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Poll until the container reaches `wanted`, or timeout.
async fn wait_for_state(ssh: &Ssh, wanted: State, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        match container_state(ssh).await? {
            Some(s) if s == wanted => return Ok(()),
            Some(s) => {
                if Instant::now() >= deadline {
                    bail!("timed out waiting for {wanted:?} (still {s:?})");
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            None => bail!("container disappeared while waiting for {wanted:?}"),
        }
    }
}

/// Remove an existing mesh container (stop then remove) so a redeploy is clean.
/// Best-effort, idempotent — a no-op when none exists.
async fn remove_existing(ssh: &Ssh) -> Result<()> {
    if container_state(ssh).await?.is_none() {
        return Ok(());
    }
    info!("removing existing {CONTAINER_COMMENT} container for redeploy");
    let _ = routeros::run_command(
        ssh,
        &format!(r#"/container/stop [find where comment="{CONTAINER_COMMENT}"]"#),
    )
    .await;
    // stop is async; give it a moment before remove.
    tokio::time::sleep(Duration::from_secs(3)).await;
    routeros::run_command(
        ssh,
        &format!(r#"/container/remove [find where comment="{CONTAINER_COMMENT}"]"#),
    )
    .await
    .context("removing existing container")?;
    Ok(())
}

/// The iroh node id (ed25519 public key, 64-hex) for a router's secret — the
/// same value meshd prints as `node id: …`. Lets us tie a connector to the
/// listener's identity without racing or mis-reading the cumulative log.
pub fn node_id(secret_hex: &str) -> Result<String> {
    if secret_hex.len() != 64 {
        bail!("secret must be 64 hex chars, got {}", secret_hex.len());
    }
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&secret_hex[i * 2..i * 2 + 2], 16)
            .context("secret is not valid hex")?;
    }
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    Ok(sk
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Read the address blob meshd printed for the node with `expected_id`,
/// polling until it appears or `timeout`. meshd prints `node id: <hex>`
/// immediately followed by `address: <blob>`, so we pair them and only accept
/// the blob whose preceding id matches — ignoring stale blobs from prior
/// (ephemeral) runs still in the cumulative log. Returns None on timeout.
pub async fn read_blob_for_id(
    ssh: &Ssh,
    expected_id: &str,
    timeout: Duration,
) -> Result<Option<String>> {
    let deadline = Instant::now() + timeout;
    loop {
        let out = ssh
            .run(r#":foreach l in=[/log/find where topics~"container"] do={:put [:tostr [/log/get $l message]]}"#)
            .await?;
        let mut last_id: Option<&str> = None;
        let mut blob: Option<String> = None;
        for line in out.lines() {
            if let Some(idx) = line.find("node id:") {
                last_id = Some(line[idx + "node id:".len()..].trim());
            } else if let Some(idx) = line.find("address:")
                && last_id == Some(expected_id)
            {
                blob = Some(line[idx + "address:".len()..].trim().to_string());
            }
        }
        if blob.is_some() {
            return Ok(blob);
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// The container `cmd` for a router given its role and (for connectors) the
/// peer's address blob.
pub fn container_cmd(role: Role, peer_blob: Option<&str>) -> Result<String> {
    match role {
        Role::Listener => Ok("tun-listen".into()),
        Role::Connector => {
            let blob = peer_blob.context(
                "connector has no peer blob — deploy its listener first, or set peer_blob",
            )?;
            Ok(format!("tun-connect {blob}"))
        }
    }
}

/// Deploy one router with an already-resolved container command.
pub async fn deploy_one(
    ssh: &Ssh,
    inv: &Inventory,
    r: &Router,
    cmd: &str,
    opts: &DeployOpts,
    secrets_dir: &Path,
) -> Result<()> {
    check_container_capable(ssh).await?;

    // 1. Converge the network (idempotent).
    let (changes, skipped) = apply::plan_changes(ssh, inv, r).await?;
    for s in &skipped {
        warn!("{}: skipping {} {} — {}", r.name, s.kind, s.id, s.why);
    }
    for c in &changes {
        apply::run_change(ssh, c).await?;
    }
    info!(
        "{}: network converged ({} change(s))",
        r.name,
        changes.len()
    );

    // 2. Upload the container tar.
    let tar = r
        .tar(inv)
        .with_context(|| format!("{}: no container tar in inventory", r.name))?;
    let tar_path = Path::new(tar);
    let tar_name = tar_path
        .file_name()
        .and_then(|s| s.to_str())
        .context("tar path has no file name")?;
    info!("{}: uploading {tar}", r.name);
    ssh.upload(tar_path, "").await.context("uploading tar")?;

    // 3. Persistent identity: stable IROH_SECRET as a container env var, so the
    //    address blob survives restarts. Re-set idempotently each deploy.
    // NB: the /container/envs menu names the list field `list` (not `name`,
    // despite older docs); /container/add references it as `envlist`.
    let secret = ensure_secret(secrets_dir, &r.name)?;
    let _ = routeros::run_command(
        ssh,
        &format!(r#"/container/envs/remove [find where list="{ENV_LIST}"]"#),
    )
    .await; // best-effort: no-op when none exist
    routeros::run_command(
        ssh,
        &format!(r#"/container/envs/add list="{ENV_LIST}" key="IROH_SECRET" value="{secret}""#),
    )
    .await
    .context("setting IROH_SECRET container env")?;

    // 4. Clean any prior container, then add + start.
    remove_existing(ssh).await?;
    let add = format!(
        r#"/container/add file="{tar_name}" interface=veth-mesh root-dir="{root}" dns={dns} cmd="{cmd}" envlist="{ENV_LIST}" comment="{CONTAINER_COMMENT}" logging=yes start-on-boot=yes"#,
        root = opts.root_dir,
        dns = opts.dns,
    );
    routeros::run_command(ssh, &add)
        .await
        .context("container add")?;
    info!("{}: container added; waiting for extraction…", r.name);
    wait_present(ssh, opts.timeout).await?;
    // This RouterOS exposes no "extracting" signal — only stopped/running. Give
    // the rootfs unpack a moment to settle before starting.
    tokio::time::sleep(Duration::from_secs(5)).await;

    routeros::run_command(
        ssh,
        &format!(r#"/container/start [find where comment="{CONTAINER_COMMENT}"]"#),
    )
    .await
    .context("container start")?;
    wait_for_state(ssh, State::Running, opts.timeout).await?;
    info!("{}: container running; cmd={cmd:?}", r.name);
    Ok(())
}

/// Tail the most recent container log lines and report whether reachability was
/// reached. Returns true if a "reachability OK" line is present.
pub async fn report_reachability(ssh: &Ssh, name: &str, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        let out = ssh
            .run(r#":foreach l in=[/log/find where topics~"container"] do={:put [:tostr [/log/get $l message]]}"#)
            .await?;
        // meshd's startup self-check logs "reachability OK" only if reachable at
        // that instant — but the relay handshake usually completes a few seconds
        // later, logging "home is now relay …". Either is a positive signal; the
        // startup "NOT REACHABLE" snapshot is NOT treated as terminal.
        let reachable = out.contains("reachability OK") || out.contains("home is now relay");
        if reachable {
            info!("{name}: reachable (relay acquired) ✓");
            return Ok(true);
        }
        if Instant::now() >= deadline {
            let tail: Vec<&str> = out.lines().rev().take(6).collect();
            warn!(
                "{name}: reachability not confirmed within timeout. Recent container log:\n  {}",
                tail.into_iter().rev().collect::<Vec<_>>().join("\n  ")
            );
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_for_listener_and_connector() {
        assert_eq!(container_cmd(Role::Listener, None).unwrap(), "tun-listen");
        assert_eq!(
            container_cmd(Role::Connector, Some("abc123")).unwrap(),
            "tun-connect abc123"
        );
        assert!(container_cmd(Role::Connector, None).is_err());
    }

    #[test]
    fn node_id_matches_ed25519_pubkey() {
        // Well-known vector: the ed25519 public key for the all-zero 32-byte
        // seed. Confirms our derivation matches what iroh/meshd computes.
        let zero = "0".repeat(64);
        assert_eq!(
            node_id(&zero).unwrap(),
            "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29"
        );
        // Deterministic + lowercase 64-hex.
        let id = node_id("11").err(); // wrong length
        assert!(id.is_some());
    }
}
