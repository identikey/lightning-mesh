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
use routes::route;

fn main() {
    tracing_subscriber::fmt::init();
    let config = Config::parse();

    // Referenced by the S3/S4 handlers landing in mjolnir-mesh-11l / 5zn;
    // unused in this scaffold story beyond validating the flags parse.
    let _ = &config.directory_file;
    let _ = &config.spool_dir;

    let server = Server::http(&config.bind).unwrap_or_else(|err| {
        panic!("failed to bind {}: {err}", config.bind);
    });
    info!(bind = %config.bind, "mjolnir-hello listening");

    for request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let url = request.url().to_string();
        let resp = route(&method, &url, config.static_root.as_deref());

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
