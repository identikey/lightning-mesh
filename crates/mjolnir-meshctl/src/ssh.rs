//! SSH/SCP transport — the bootstrap + file-movement half of meshctl.
//!
//! Config reconciliation goes over the RouterOS REST API (added in M1), but two
//! things can only happen over SSH/SCP: the one-time bootstrap (importing the
//! operator key, enabling `www-ssl` so REST works at all) and uploading the
//! container image tar (REST can't push a file). This module is that channel.
//!
//! We shell out to the system `ssh`/`scp` rather than linking a Rust SSH stack:
//! it reuses the operator's ssh-agent / `~/.ssh/config` / known_hosts exactly
//! as the manual `ssh admin@router` workflow already does, with zero new
//! crypto surface.
//!
//! ## RouterOS quirks handled here
//! - `StrictHostKeyChecking=accept-new` — trust a fresh router on first contact
//!   without an interactive yes/no prompt (but still pin it afterward).
//! - `scp -O` — OpenSSH 9+ defaults `scp` to the SFTP protocol, which RouterOS
//!   does NOT implement; `-O` forces the legacy SCP protocol it expects.

use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

/// An SSH connection to one router.
#[derive(Debug, Clone)]
pub struct Ssh {
    /// `user@address`.
    target: String,
    connect_timeout_secs: u32,
    /// When true, pass `BatchMode=yes` so ssh fails fast instead of prompting
    /// (the right mode for automated plan/apply once keys are installed). When
    /// false, stdio is inherited so a first-run password prompt reaches the
    /// operator's terminal — used by bootstrap before key auth exists.
    batch: bool,
}

impl Ssh {
    /// A batch-mode connection (no prompts; fails if key auth isn't set up).
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            connect_timeout_secs: 10,
            batch: true,
        }
    }

    /// Allow interactive prompts (password, host-key confirmation) by inheriting
    /// the parent stdio. Use for the first bootstrap before the key is imported.
    pub fn interactive(mut self) -> Self {
        self.batch = false;
        self
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    /// Common `-o` options shared by ssh and scp.
    fn base_opts(&self) -> Vec<String> {
        let mut opts = vec![
            "-o".into(),
            "StrictHostKeyChecking=accept-new".into(),
            "-o".into(),
            format!("ConnectTimeout={}", self.connect_timeout_secs),
        ];
        if self.batch {
            opts.push("-o".into());
            opts.push("BatchMode=yes".into());
        }
        opts
    }

    /// Run a RouterOS command and return its stdout. Errors on a non-zero exit,
    /// surfacing stderr. Only available in batch mode (output capture and
    /// interactive prompts are mutually exclusive over one channel).
    pub async fn run(&self, remote_cmd: &str) -> Result<String> {
        if !self.batch {
            bail!("Ssh::run requires batch mode; use run_interactive for prompts");
        }
        let mut cmd = Command::new("ssh");
        cmd.args(self.base_opts())
            .arg(&self.target)
            .arg(remote_cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let out = cmd
            .output()
            .await
            .with_context(|| format!("spawning ssh to {}", self.target))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!(
                "ssh {} `{}` failed ({}): {}",
                self.target,
                remote_cmd,
                out.status,
                stderr.trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run a command with stdio inherited from the parent, so password / host-key
    /// prompts reach the terminal. Returns only success/failure (stdout is not
    /// captured). Used by bootstrap before key auth is in place.
    pub async fn run_interactive(&self, remote_cmd: &str) -> Result<()> {
        let status = Command::new("ssh")
            .args(self.base_opts())
            .arg(&self.target)
            .arg(remote_cmd)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .with_context(|| format!("spawning ssh to {}", self.target))?;
        if !status.success() {
            bail!("ssh {} `{}` failed ({})", self.target, remote_cmd, status);
        }
        Ok(())
    }

    /// Upload a local file to the router via `scp -O` (legacy SCP protocol —
    /// required for RouterOS, see the module note). `remote_dest` is a path on
    /// the router; empty string drops it in the home/root directory.
    pub async fn upload(&self, local: &Path, remote_dest: &str) -> Result<()> {
        if !local.exists() {
            bail!("local file does not exist: {}", local.display());
        }
        let dest = format!("{}:{}", self.target, remote_dest);
        let mut cmd = Command::new("scp");
        cmd.arg("-O").args(self.base_opts()).arg(local).arg(&dest);
        if self.batch {
            cmd.stdin(Stdio::null());
        }
        let status = cmd
            .status()
            .await
            .with_context(|| format!("spawning scp to {}", self.target))?;
        if !status.success() {
            bail!(
                "scp {} -> {} failed ({})",
                local.display(),
                dest,
                status
            );
        }
        Ok(())
    }
}
