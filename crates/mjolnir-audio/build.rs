//! Build-time enforcement of the libopus version requirement.
//!
//! mjolnir-audio relies on libopus 1.5+ for two production features:
//!
//! * **Deep PLC (FARGAN)** — neural packet-loss concealment that
//!   replaces the classical LPC-extrapolation heuristic. Activates
//!   automatically at decoder complexity ≥ 5, which is the default in
//!   libopus ≥ 1.5.
//! * **DRED (Deep Redundancy)** — sender-side redundancy VAE for
//!   surviving burst loss up to ~1 second. Required for the planned
//!   burst-loss tier; encoder-side ctl is `OPUS_SET_DRED_DURATION`.
//!
//! On libopus < 1.5 the decoder silently falls back to heuristic PLC
//! and DRED is unavailable, so we fail the build rather than silently
//! ship a degraded runtime. Distro hint: `apt install libopus-dev` on
//! Debian/Ubuntu, `brew install opus` on macOS, or build from source.
//!
//! The detected version is exposed to the crate as
//! `env!("MJOLNIR_LIBOPUS_VERSION")` for startup logging.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    let output = Command::new("pkg-config")
        .args(["--modversion", "opus"])
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "pkg-config required to detect libopus version: {e}. \
                 Install pkg-config and libopus-dev (or equivalent) \
                 with libopus >= 1.5 headers."
            )
        });

    if !output.status.success() {
        panic!(
            "libopus not found via pkg-config (stderr: {}). \
             Install libopus >= 1.5 development headers \
             (apt install libopus-dev, brew install opus, etc.).",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let version = String::from_utf8(output.stdout)
        .expect("pkg-config returned non-UTF8 version string");
    let version = version.trim().to_string();

    let parts: Vec<u32> = version
        .split('.')
        .take(2)
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);

    if (major, minor) < (1, 5) {
        panic!(
            "libopus {} found, but mjolnir-audio requires >= 1.5 for \
             deep neural PLC (FARGAN) and DRED support. Upgrade the \
             system libopus development headers.",
            version
        );
    }

    println!("cargo:rustc-env=MJOLNIR_LIBOPUS_VERSION={}", version);
}
