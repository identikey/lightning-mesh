//! Request routing and handlers.
//!
//! Routing is factored into a pure function (`route`) that maps a method +
//! path to a `RouteResponse`, independent of `tiny_http`'s request/response
//! types. This keeps the routing/handler logic unit-testable without binding
//! a real socket; `server.rs` is the thin adapter that drives `tiny_http` and
//! translates `RouteResponse` into wire responses.

use std::path::Path;

use crate::assets::{StaticAssets, INDEX_HTML};

#[derive(Debug, PartialEq, Eq)]
pub struct RouteResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl RouteResponse {
    fn json(status: u16, body: impl Into<String>) -> Self {
        RouteResponse { status, content_type: "application/json", body: body.into().into_bytes() }
    }

    fn html(status: u16, body: Vec<u8>) -> Self {
        RouteResponse { status, content_type: "text/html; charset=utf-8", body }
    }
}

/// `GET /api/health` — liveness for the deploy health-gate.
fn health() -> RouteResponse {
    RouteResponse::json(200, r#"{"status":"ok"}"#)
}

/// Serve the static bundle (embedded, or `--static-root` override for dev),
/// with SPA fallback to `index.html` for any path that isn't a known asset —
/// the SvelteKit app owns client-side routing.
fn serve_static(path: &str, static_root: Option<&Path>) -> RouteResponse {
    let asset_path = path.trim_start_matches('/');
    let asset_path = if asset_path.is_empty() { INDEX_HTML } else { asset_path };

    if let Some(root) = static_root {
        if let Ok(bytes) = std::fs::read(root.join(asset_path)) {
            return RouteResponse::html(200, bytes);
        }
        if let Ok(bytes) = std::fs::read(root.join(INDEX_HTML)) {
            return RouteResponse::html(200, bytes);
        }
        return RouteResponse::html(404, b"not found".to_vec());
    }

    if let Some(file) = StaticAssets::get(asset_path) {
        return RouteResponse::html(200, file.data.into_owned());
    }
    match StaticAssets::get(INDEX_HTML) {
        Some(file) => RouteResponse::html(200, file.data.into_owned()),
        None => RouteResponse::html(404, b"not found".to_vec()),
    }
}

/// Route a `(method, path)` pair to a response. `static_root` is the optional
/// on-disk override of the embedded bundle (`--static-root`, dev only).
pub fn route(method: &str, path: &str, static_root: Option<&Path>) -> RouteResponse {
    match (method, path) {
        ("GET", "/api/health") => health(),

        // --- S3 (mjolnir-mesh-11l): read-only mesh state ---------------
        // ("GET", "/api/directory") => ...,  // AddrBook + ServiceEntry projection from directory_file
        // ("GET", "/api/node") => ...,       // this node's own identity/summary

        // --- S4 (mjolnir-mesh-5zn): identity ceremony -------------------
        // ("GET", "/api/challenge") => ...,  // fresh nonce
        // ("POST", "/api/identity") => ...,  // spool a signed identity submission

        ("GET", _) => serve_static(path, static_root),
        _ => RouteResponse { status: 404, content_type: "text/plain", body: b"not found".to_vec() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_returns_ok_json() {
        let resp = route("GET", "/api/health", None);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/json");
        assert_eq!(resp.body, br#"{"status":"ok"}"#.to_vec());
    }

    #[test]
    fn root_serves_embedded_index() {
        let resp = route("GET", "/", None);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "text/html; charset=utf-8");
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Lightning Mesh"));
        assert!(body.contains("hello.mesh"));
    }

    #[test]
    fn unknown_path_falls_back_to_index_spa_style() {
        let resp = route("GET", "/some/client/route", None);
        assert_eq!(resp.status, 200);
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Lightning Mesh"));
    }

    #[test]
    fn unknown_method_is_not_found() {
        let resp = route("DELETE", "/api/health", None);
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn static_root_override_reads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<html>dev override</html>").unwrap();
        let resp = route("GET", "/", Some(dir.path()));
        assert_eq!(resp.status, 200);
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("dev override"));
    }
}
