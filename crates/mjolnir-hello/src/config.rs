//! CLI configuration.
//!
//! Dev defaults are safe for a laptop (`127.0.0.1:8080`); the OpenWrt procd
//! service (S7, bead mjolnir-mesh-eei) overrides `--bind` to the node's LAN
//! gateway IP:80. `--directory-file` and `--spool-dir` are the daemon seams
//! from docs/network-coordination/hello-mesh-service.md §1/§3: `mjolnir-meshd`
//! writes the former, `mjolnir-hello` writes into the latter for the daemon to
//! ingest (S4). Neither is read/written yet in this scaffold story.

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "mjolnir-hello",
    about = "hello.mesh front desk: static frontend + read-only mesh API"
)]
pub struct Config {
    /// Address to bind the HTTP server to. The node's procd service overrides
    /// this to the LAN gateway IP:80; the dev default is loopback-only.
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub bind: String,

    /// Path to the read-only directory projection that `mjolnir-meshd` writes
    /// (address book + subnet claims, and services once mjolnir-mesh-e21
    /// lands). Consumed by the S3 `/api/directory` and `/api/node` endpoints.
    #[arg(long, default_value = "/var/run/mjolnir/directory.json")]
    pub directory_file: PathBuf,

    /// Spool directory `mjolnir-hello` drops identity submissions into for
    /// `mjolnir-meshd` to validate and gossip. Consumed by the S4
    /// `POST /api/identity` endpoint.
    #[arg(long, default_value = "/var/run/mjolnir/pending")]
    pub spool_dir: PathBuf,

    /// Override the embedded static bundle with a directory on disk, for fast
    /// frontend dev iteration without rebuilding this binary.
    #[arg(long)]
    pub static_root: Option<PathBuf>,
}
