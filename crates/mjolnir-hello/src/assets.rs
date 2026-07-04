//! Embedded static bundle.
//!
//! The SvelteKit frontend build (adapter-static output) is synced into
//! `crates/mjolnir-hello/static/` by the frontend story; `rust-embed` compiles
//! that directory's contents into the binary so `mjolnir-hello` ships as one
//! self-contained artifact. A `--static-root` flag can override this with a
//! directory on disk for fast dev iteration on the frontend without rebuilding.

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
pub struct StaticAssets;

pub const INDEX_HTML: &str = "index.html";
