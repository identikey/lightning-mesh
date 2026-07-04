//! `mjolnir-hello` — the hello.mesh front desk server.
//!
//! Serves the embedded static frontend bundle and (starting with S3/S4) a
//! read-only JSON API + identity ingest over mesh state written by
//! `mjolnir-meshd`. This binary is standalone: it depends on the
//! `mjolnir-mesh` library with default features only (no `daemon` feature, so
//! no iroh) and runs fully independently of `mjolnir-meshd` — a node runs the
//! mesh fine without the front desk. See
//! docs/network-coordination/hello-mesh-service.md §3 and bead
//! mjolnir-mesh-bl2.

mod assets;
mod config;
mod routes;

use clap::Parser;
use tiny_http::{Header, Response, Server};
use tracing::info;

use config::Config;
use routes::{new_challenge_store, route, DirectoryCache};

fn main() {
    tracing_subscriber::fmt::init();
    let config = Config::parse();

    let challenges = new_challenge_store();
    let directory_cache = DirectoryCache::new();

    let server = Server::http(&config.bind).unwrap_or_else(|err| {
        panic!("failed to bind {}: {err}", config.bind);
    });
    info!(bind = %config.bind, "mjolnir-hello listening");

    for mut request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let url = request.url().to_string();

        let mut body = Vec::new();
        if let Err(err) = request.as_reader().read_to_end(&mut body) {
            tracing::warn!(%err, "failed to read request body");
        }

        let resp = route(
            &method,
            &url,
            config.static_root.as_deref(),
            &body,
            &challenges,
            &config.spool_dir,
            &directory_cache,
            &config.directory_file,
        );

        let content_type = Header::from_bytes(
            &b"Content-Type"[..],
            resp.content_type.as_bytes(),
        )
        .expect("valid content-type header");

        let response = Response::from_data(resp.body)
            .with_status_code(resp.status)
            .with_header(content_type);

        if let Err(err) = request.respond(response) {
            tracing::warn!(%err, "failed to respond to request");
        }
    }
}
