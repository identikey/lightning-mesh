use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, Instant};
use tracing::{error, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error("spawn: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("babeld exited unexpectedly with status {0:?}")]
    Exited(std::process::ExitStatus),
    #[error("supervisor already running")]
    AlreadyRunning,
}

/// Manages a long-running `babeld` child process.
///
/// Methods on `BabelSupervisor` are `&self` and internally synchronized so
/// the same instance can be shared between the encap loop, the gossip
/// receiver, and the shutdown signal handler.
pub struct BabelSupervisor {
    config_path: PathBuf,
    babeld_path: PathBuf,
    pub(crate) child: Mutex<Option<Child>>,
    pending_sighup: Mutex<Option<Instant>>,
}

impl BabelSupervisor {
    /// `babeld_path` is the path to the `babeld` binary (e.g. `"babeld"` to
    /// rely on PATH, or `"/usr/sbin/babeld"` for an explicit path).
    pub fn new(config_path: PathBuf, babeld_path: PathBuf) -> Self {
        Self {
            config_path,
            babeld_path,
            child: Mutex::new(None),
            pending_sighup: Mutex::new(None),
        }
    }

    /// Spawn babeld. Fails with `AlreadyRunning` if a child is already alive.
    pub async fn spawn(&self) -> Result<(), SupervisorError> {
        let mut guard = self.child.lock().await;
        if guard.is_some() {
            return Err(SupervisorError::AlreadyRunning);
        }
        let child = Command::new(&self.babeld_path)
            .arg("-c")
            .arg(&self.config_path)
            // Foreground mode: don't fork
            .arg("-D")
            // Inherit stdio so babeld's own logs (neighbor/route exchange) flow
            // to our stdout — captured by the RouterOS container log. Piping
            // without draining deadlocks babeld once the pipe buffer fills.
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        info!(pid = child.id(), "spawned babeld");
        *guard = Some(child);
        Ok(())
    }

    /// Send SIGHUP to babeld so it re-reads its config file.
    /// Debounced: if called multiple times within 100ms, only one signal is sent.
    #[cfg(unix)]
    pub async fn sighup(&self) -> Result<(), SupervisorError> {
        const DEBOUNCE: Duration = Duration::from_millis(100);

        let now = Instant::now();
        {
            let mut pending = self.pending_sighup.lock().await;
            // Record the request; coalesce by holding off on actual signal delivery
            // until the debounce window passes with no further calls.
            *pending = Some(now);
        }
        // Wait the debounce window then check we're still the most recent request.
        sleep(DEBOUNCE).await;
        {
            let mut pending = self.pending_sighup.lock().await;
            match *pending {
                Some(latest) if latest <= now => {
                    *pending = None;
                }
                Some(_) => {
                    // A newer request superseded us; let it handle delivery.
                    return Ok(());
                }
                None => return Ok(()),
            }
        }
        // Actually deliver the signal.
        let guard = self.child.lock().await;
        if let Some(child) = guard.as_ref()
            && let Some(pid) = child.id()
        {
            let pid = nix::unistd::Pid::from_raw(pid as i32);
            if let Err(e) = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGHUP) {
                warn!(?e, "failed to send SIGHUP to babeld");
            }
        }
        Ok(())
    }

    #[cfg(not(unix))]
    pub async fn sighup(&self) -> Result<(), SupervisorError> {
        // Non-unix has no SIGHUP — restart instead. Out of scope for MVP (Linux only).
        Ok(())
    }

    /// SIGTERM, wait up to 2s, then SIGKILL if still alive.
    pub async fn shutdown(&self) -> Result<(), SupervisorError> {
        let mut guard = self.child.lock().await;
        let Some(mut child) = guard.take() else {
            return Ok(());
        };
        #[cfg(unix)]
        {
            if let Some(pid) = child.id() {
                let pid = nix::unistd::Pid::from_raw(pid as i32);
                let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
            }
        }
        let term = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        match term {
            Ok(Ok(_status)) => Ok(()),
            Ok(Err(e)) => Err(SupervisorError::Spawn(e)),
            Err(_) => {
                // Timeout — kill hard.
                let _ = child.kill().await;
                let _ = child.wait().await;
                Ok(())
            }
        }
    }

    /// Wait for the child to exit and report its status. Useful for the
    /// supervisor's crash-restart loop.
    pub async fn wait(&self) -> Result<std::process::ExitStatus, SupervisorError> {
        let mut guard = self.child.lock().await;
        match guard.as_mut() {
            Some(child) => Ok(child.wait().await?),
            None => Err(SupervisorError::AlreadyRunning), // misuse
        }
    }
}

/// Crash-restart loop with exponential backoff (1s → 2s → 4s … capped at 30s).
/// Resets the backoff after a clean run of >60s.
///
/// Runs until the future returned by `should_stop` resolves to true.
pub async fn run_with_restart<F, S>(
    sup: &BabelSupervisor,
    mut should_stop: S,
) -> Result<(), SupervisorError>
where
    S: FnMut() -> F,
    F: std::future::Future<Output = bool>,
{
    let mut backoff = Duration::from_secs(1);
    loop {
        if should_stop().await {
            sup.shutdown().await?;
            return Ok(());
        }
        let start = Instant::now();
        sup.spawn().await?;
        let result = sup.wait().await;
        match result {
            Ok(status) => {
                let ran = start.elapsed();
                if status.success() {
                    info!("babeld exited cleanly after {:?}", ran);
                } else {
                    warn!("babeld exited with {:?} after {:?}", status, ran);
                }
                if ran > Duration::from_secs(60) {
                    backoff = Duration::from_secs(1);
                } else {
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
            Err(e) => {
                error!(?e, "babeld supervisor error");
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
        if should_stop().await {
            return Ok(());
        }
        sleep(backoff).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn already_running_error_when_spawned_twice() {
        let sup = BabelSupervisor::new(
            std::path::PathBuf::from("/dev/null"),
            std::path::PathBuf::from("tail"),
        );
        // Manually inject a long-running process into the child mutex to test the
        // AlreadyRunning guard without needing babeld-compatible CLI args.
        {
            let cmd = tokio::process::Command::new("tail")
                .arg("-f")
                .arg("/dev/null")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn tail");
            *sup.child.lock().await = Some(cmd);
        }
        let r = sup.spawn().await;
        assert!(matches!(r, Err(SupervisorError::AlreadyRunning)));
        sup.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_noop_when_no_child() {
        let sup = BabelSupervisor::new(
            std::path::PathBuf::from("/dev/null"),
            std::path::PathBuf::from("babeld"),
        );
        // Should succeed cleanly with no child running.
        sup.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_kills_injected_child() {
        let sup = BabelSupervisor::new(
            std::path::PathBuf::from("/dev/null"),
            std::path::PathBuf::from("sleep"),
        );
        {
            let cmd = tokio::process::Command::new("sleep")
                .arg("30")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn sleep");
            *sup.child.lock().await = Some(cmd);
        }
        // shutdown() should SIGTERM then wait; sleep will exit
        sup.shutdown().await.unwrap();
        // After shutdown the slot must be empty
        assert!(sup.child.lock().await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "requires babeld in PATH; run with `cargo test -- --ignored`"]
    async fn real_babeld_spawn_shutdown() {
        use tempfile::tempdir;
        use crate::babel::render_babeld_conf;
        use crate::babel::BabelConfigInputs;

        let dir = tempdir().unwrap();
        let conf_path = dir.path().join("babeld.conf");
        let inputs = BabelConfigInputs::new(None, &[]);
        let conf = render_babeld_conf(&inputs);
        std::fs::write(&conf_path, &conf).unwrap();

        let sup = BabelSupervisor::new(conf_path, std::path::PathBuf::from("babeld"));
        sup.spawn().await.expect("spawn babeld");
        tokio::time::sleep(Duration::from_millis(500)).await;
        sup.shutdown().await.expect("shutdown babeld");
    }
}
