//! Stamp the build with a unique source fingerprint, exposed as the
//! `MJOLNIR_BUILD` compile-time env (read via `env!` in the daemon banner).
//!
//! Why this exists (mjolnir-mesh-auu): two routers running the "same" mesh
//! must run the *same binary*. `CARGO_PKG_VERSION` is `0.1.0` for every build,
//! so it can't tell two different commits apart — which is exactly the
//! binary/version skew we suspected behind the tunnel death. A git short-SHA
//! (+`-dirty`) printed at startup makes sameness provable: identical banner
//! lines across nodes == identical source.
//!
//! Resolution order, so it works both on the host and inside the Docker
//! cross-build (where `.git` is dockerignored and unavailable):
//!   1. `MJOLNIR_BUILD` env, if set — the Dockerfile passes the host's git SHA
//!      in via a `--build-arg` (see deploy/mikrotik/build.sh + Dockerfile).
//!   2. `git rev-parse` on the host, with a `-dirty` suffix for a dirty tree.
//!   3. `"unknown"` fallback — never fail the build over a missing stamp.

use std::process::Command;

fn main() {
    // Rebuild the stamp when the override changes or HEAD moves.
    println!("cargo:rerun-if-env-changed=MJOLNIR_BUILD");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");

    let stamp = std::env::var("MJOLNIR_BUILD")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(git_stamp);

    println!("cargo:rustc-env=MJOLNIR_BUILD={stamp}");
}

/// `<short-sha>` or `<short-sha>-dirty`, or `"unknown"` if git is unavailable
/// (e.g. the Docker build, where `.git` is excluded — there the env override
/// from build.sh supplies the SHA instead).
fn git_stamp() -> String {
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let Some(sha) = sha else {
        return "unknown".to_string();
    };

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);

    if dirty { format!("{sha}-dirty") } else { sha }
}
