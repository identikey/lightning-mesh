//! Request routing and handlers.
//!
//! Routing is factored into a pure function (`route`) that maps a method +
//! path to a `RouteResponse`, independent of `tiny_http`'s request/response
//! types. This keeps the routing/handler logic unit-testable without binding
//! a real socket; `server.rs` is the thin adapter that drives `tiny_http` and
//! translates `RouteResponse` into wire responses.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use data_encoding::HEXLOWER;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::assets::{INDEX_HTML, StaticAssets};

/// How long an issued challenge remains redeemable.
const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);

/// In-memory store of issued, not-yet-consumed challenges: hex nonce ->
/// issue time. `mjolnir-hello` runs a single-threaded request loop, but the
/// `Mutex` keeps the store safely shareable if that ever changes.
pub type ChallengeStore = Mutex<HashMap<String, Instant>>;

pub fn new_challenge_store() -> ChallengeStore {
    Mutex::new(HashMap::new())
}

/// Empty directory projection, served whenever `directory.json` is missing
/// or unreadable and no last-good copy exists yet.
const EMPTY_DIRECTORY: &str =
    r#"{"version":1,"node":null,"neighbors":[],"identities":[],"services":[]}"#;

/// Cached last-good read of the daemon-written `directory.json`, keyed off
/// the file's mtime so a request only re-reads the file when it changed.
/// Falls back to (and remembers) [`EMPTY_DIRECTORY`] when the file is
/// missing/unreadable and there is no prior last-good snapshot.
#[derive(Debug, Default)]
pub struct DirectoryCache {
    inner: Mutex<CachedDirectory>,
}

#[derive(Debug, Default)]
struct CachedDirectory {
    mtime: Option<SystemTime>,
    body: Option<String>,
}

impl DirectoryCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read (or serve cached) directory.json contents as a raw JSON string.
    fn read(&self, path: &Path) -> String {
        let mtime = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok();

        let mut cached = self.inner.lock().expect("directory cache poisoned");

        // Re-read only if the mtime is unknown or has changed since the last
        // successful read. On read failure, fall through and serve the
        // last-good body (or the empty directory if there isn't one) without
        // clobbering the cached mtime — a transient stat/read race shouldn't
        // drop a known-good snapshot.
        if (mtime.is_none() || mtime != cached.mtime)
            && let Ok(contents) = std::fs::read_to_string(path)
        {
            cached.mtime = mtime;
            cached.body = Some(contents);
        }

        cached
            .body
            .clone()
            .unwrap_or_else(|| EMPTY_DIRECTORY.to_string())
    }
}

#[derive(Debug, Deserialize)]
struct IdentityRequest {
    pubkey: String,
    sig: String,
    challenge: String,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Serialize)]
struct IdentityRecord<'a> {
    pubkey: &'a str,
    sig: &'a str,
    challenge: &'a str,
    label: &'a Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RouteResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl RouteResponse {
    fn json(status: u16, body: impl Into<String>) -> Self {
        RouteResponse {
            status,
            content_type: "application/json",
            body: body.into().into_bytes(),
        }
    }

    fn html(status: u16, body: Vec<u8>) -> Self {
        RouteResponse {
            status,
            content_type: "text/html; charset=utf-8",
            body,
        }
    }

    /// Serve a static asset with a Content-Type derived from its path
    /// extension. Load-bearing for the SvelteKit bundle: a `.js` ES module
    /// served as `text/html` is refused by the browser, leaving a blank page.
    fn asset(status: u16, path: &str, body: Vec<u8>) -> Self {
        RouteResponse {
            status,
            content_type: content_type_for(path),
            body,
        }
    }
}

/// Map a path's file extension to a Content-Type. Covers what the SvelteKit
/// static build emits; unknown extensions fall back to octet-stream.
fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") | Some("webmanifest") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `GET /api/health` — liveness for the deploy health-gate.
fn health() -> RouteResponse {
    RouteResponse::json(200, r#"{"status":"ok"}"#)
}

/// `GET /api/directory` — serve `directory.json` verbatim (cached, last-good
/// on read failure, empty-but-valid directory if there's no last-good copy).
fn directory(cache: &DirectoryCache, directory_file: &Path) -> RouteResponse {
    RouteResponse::json(200, cache.read(directory_file))
}

/// `GET /api/node` — extract just the `node` section of the directory, for
/// the "you are here" header. Missing/null node -> `{}`.
fn node(cache: &DirectoryCache, directory_file: &Path) -> RouteResponse {
    let body = cache.read(directory_file);
    let value: serde_json::Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(_) => return RouteResponse::json(200, "{}"),
    };
    let node = value
        .get("node")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let node = if node.is_null() {
        serde_json::json!({})
    } else {
        node
    };
    RouteResponse::json(200, node.to_string())
}

/// Serve the static bundle (embedded, or `--static-root` override for dev),
/// with SPA fallback to `index.html` for any path that isn't a known asset —
/// the SvelteKit app owns client-side routing.
fn serve_static(path: &str, static_root: Option<&Path>) -> RouteResponse {
    let asset_path = path.trim_start_matches('/');
    let asset_path = if asset_path.is_empty() {
        INDEX_HTML
    } else {
        asset_path
    };

    if let Some(root) = static_root {
        if let Ok(bytes) = std::fs::read(root.join(asset_path)) {
            return RouteResponse::asset(200, asset_path, bytes);
        }
        if let Ok(bytes) = std::fs::read(root.join(INDEX_HTML)) {
            return RouteResponse::html(200, bytes);
        }
        return RouteResponse::html(404, b"not found".to_vec());
    }

    if let Some(file) = StaticAssets::get(asset_path) {
        return RouteResponse::asset(200, asset_path, file.data.into_owned());
    }
    match StaticAssets::get(INDEX_HTML) {
        Some(file) => RouteResponse::html(200, file.data.into_owned()),
        None => RouteResponse::html(404, b"not found".to_vec()),
    }
}

/// `GET /api/challenge` — issue a fresh single-use nonce, hex-encoded.
fn issue_challenge(challenges: &ChallengeStore) -> RouteResponse {
    let mut nonce = [0u8; 32];
    rand::rng().fill_bytes(&mut nonce);
    let hex = HEXLOWER.encode(&nonce);

    let mut store = challenges.lock().expect("challenge store poisoned");
    // Opportunistically sweep expired entries so the map doesn't grow
    // unbounded.
    store.retain(|_, issued_at| issued_at.elapsed() < CHALLENGE_TTL);
    store.insert(hex.clone(), Instant::now());
    drop(store);

    RouteResponse::json(200, format!(r#"{{"challenge":"{hex}"}}"#))
}

fn bad_request(msg: &str) -> RouteResponse {
    RouteResponse::json(400, format!(r#"{{"error":"{msg}"}}"#))
}

/// `POST /api/identity` — validate a signed challenge response and, on
/// success, spool the submission for `mjolnir-meshd` to ingest. The server
/// holds no private key; it only verifies a signature made elsewhere.
fn submit_identity(body: &[u8], challenges: &ChallengeStore, spool_dir: &Path) -> RouteResponse {
    let req: IdentityRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(_) => return bad_request("invalid request body"),
    };

    let Ok(pubkey_bytes) = HEXLOWER.decode(req.pubkey.as_bytes()) else {
        return bad_request("invalid pubkey encoding");
    };
    let Ok(sig_bytes) = HEXLOWER.decode(req.sig.as_bytes()) else {
        return bad_request("invalid signature encoding");
    };
    let Ok(pubkey_arr): Result<[u8; 32], _> = pubkey_bytes.try_into() else {
        return bad_request("invalid pubkey length");
    };
    let Ok(sig_arr): Result<[u8; 64], _> = sig_bytes.try_into() else {
        return bad_request("invalid signature length");
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pubkey_arr) else {
        return bad_request("invalid pubkey");
    };
    let signature = Signature::from_bytes(&sig_arr);

    let Ok(challenge_bytes) = HEXLOWER.decode(req.challenge.as_bytes()) else {
        return bad_request("invalid challenge encoding");
    };

    // Peek at the challenge: known and unexpired. Only consumed once the
    // signature also checks out, so a bad signature doesn't burn a nonce the
    // legitimate holder might retry.
    {
        let store = challenges.lock().expect("challenge store poisoned");
        match store.get(&req.challenge) {
            Some(issued_at) if issued_at.elapsed() < CHALLENGE_TTL => {}
            _ => return bad_request("unknown or expired challenge"),
        }
    }

    if verifying_key.verify(&challenge_bytes, &signature).is_err() {
        return bad_request("invalid signature");
    }

    // Consume the challenge (single-use) now that the submission is valid.
    let mut store = challenges.lock().expect("challenge store poisoned");
    if store.remove(&req.challenge).is_none() {
        // Raced with another consumer (or it expired between the peek and
        // here) — reject rather than double-spool.
        return bad_request("unknown or expired challenge");
    }
    drop(store);

    if let Err(err) = std::fs::create_dir_all(spool_dir) {
        tracing::error!(%err, "failed to create spool dir");
        return RouteResponse::json(500, r#"{"error":"spool unavailable"}"#);
    }

    let record = IdentityRecord {
        pubkey: &req.pubkey,
        sig: &req.sig,
        challenge: &req.challenge,
        label: &req.label,
    };
    let record_json = serde_json::to_string(&record).expect("record serializes");

    let dest = spool_dir.join(format!("{}.json", req.pubkey));
    if let Err(err) = std::fs::write(&dest, &record_json) {
        tracing::error!(%err, path = %dest.display(), "failed to write identity spool entry");
        return RouteResponse::json(500, r#"{"error":"spool write failed"}"#);
    }

    RouteResponse::json(200, record_json)
}

/// Route a `(method, path)` pair to a response. `static_root` is the optional
/// on-disk override of the embedded bundle (`--static-root`, dev only).
/// `body` is the raw request body (only consulted for `POST` handlers).
/// `challenges` and `spool_dir` are the S4 identity-ceremony seams.
/// `directory_cache` and `directory_file` are the S3 read-only mesh-state
/// seams.
#[allow(clippy::too_many_arguments)]
pub fn route(
    method: &str,
    path: &str,
    static_root: Option<&Path>,
    body: &[u8],
    challenges: &ChallengeStore,
    spool_dir: &Path,
    directory_cache: &DirectoryCache,
    directory_file: &Path,
) -> RouteResponse {
    match (method, path) {
        ("GET", "/api/health") => health(),

        // --- S3 (mjolnir-mesh-11l): read-only mesh state ---------------
        ("GET", "/api/directory") => directory(directory_cache, directory_file),
        ("GET", "/api/node") => node(directory_cache, directory_file),

        // --- S4 (mjolnir-mesh-5zn): identity ceremony -------------------
        ("GET", "/api/challenge") => issue_challenge(challenges),
        ("POST", "/api/identity") => submit_identity(body, challenges, spool_dir),

        ("GET", _) => serve_static(path, static_root),
        _ => RouteResponse {
            status: 404,
            content_type: "text/plain",
            body: b"not found".to_vec(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// Empty challenge store + throwaway spool dir, for tests that don't
    /// exercise the identity ceremony.
    fn no_state() -> (ChallengeStore, tempfile::TempDir) {
        (new_challenge_store(), tempfile::tempdir().unwrap())
    }

    /// Fresh directory cache + a path to a (not-yet-existing) directory file
    /// under a throwaway dir, for tests that don't exercise S3 endpoints.
    fn no_directory() -> (DirectoryCache, tempfile::TempDir) {
        (DirectoryCache::new(), tempfile::tempdir().unwrap())
    }

    fn test_keypair() -> SigningKey {
        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    #[test]
    fn health_returns_ok_json() {
        let (challenges, spool) = no_state();
        let resp = route(
            "GET",
            "/api/health",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/json");
        assert_eq!(resp.body, br#"{"status":"ok"}"#.to_vec());
    }

    #[test]
    fn root_serves_embedded_index() {
        let (challenges, spool) = no_state();
        let resp = route(
            "GET",
            "/",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "text/html; charset=utf-8");
        // Assert the SPA shell, not specific copy — the embedded index is the
        // placeholder in a plain build and the SvelteKit shell after build:embed.
        let body = String::from_utf8(resp.body).unwrap().to_lowercase();
        assert!(body.contains("<!doctype html") || body.contains("<html"));
    }

    #[test]
    fn unknown_path_falls_back_to_index_spa_style() {
        let (challenges, spool) = no_state();
        let resp = route(
            "GET",
            "/some/client/route",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "text/html; charset=utf-8");
        let body = String::from_utf8(resp.body).unwrap().to_lowercase();
        assert!(body.contains("<!doctype html") || body.contains("<html"));
    }

    #[test]
    fn assets_get_correct_mime_not_html() {
        // Regression: a .js ES module served as text/html is refused by browsers
        // (blank page). serve_static must set Content-Type by extension.
        assert_eq!(
            content_type_for("/_app/immutable/entry/start.abc.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type_for("/_app/immutable/assets/0.abc.css"),
            "text/css; charset=utf-8"
        );
        assert_eq!(content_type_for("favicon.svg"), "image/svg+xml");
        assert_eq!(content_type_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type_for("robots.txt"), "text/plain; charset=utf-8");
    }

    #[test]
    fn unknown_method_is_not_found() {
        let (challenges, spool) = no_state();
        let resp = route(
            "DELETE",
            "/api/health",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn static_root_override_reads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<html>dev override</html>").unwrap();
        let (challenges, spool) = no_state();
        let resp = route(
            "GET",
            "/",
            Some(dir.path()),
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 200);
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("dev override"));
    }

    #[test]
    fn challenge_then_valid_signature_spools_and_consumes_nonce() {
        let (challenges, spool) = no_state();
        let signing_key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(signing_key.verifying_key().as_bytes());

        let challenge_resp = route(
            "GET",
            "/api/challenge",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(challenge_resp.status, 200);
        let challenge_json: serde_json::Value =
            serde_json::from_slice(&challenge_resp.body).unwrap();
        let challenge_hex = challenge_json["challenge"].as_str().unwrap().to_string();
        let challenge_bytes = HEXLOWER.decode(challenge_hex.as_bytes()).unwrap();

        let sig = signing_key.sign(&challenge_bytes);
        let sig_hex = HEXLOWER.encode(&sig.to_bytes());

        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}"}}"#
        );
        let resp = route(
            "POST",
            "/api/identity",
            None,
            body.as_bytes(),
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 200);

        let spooled_path = spool.path().join(format!("{pubkey_hex}.json"));
        assert!(
            spooled_path.exists(),
            "expected spooled record at {spooled_path:?}"
        );

        // Single-use: the same challenge must now be rejected.
        let replay = route(
            "POST",
            "/api/identity",
            None,
            body.as_bytes(),
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(replay.status, 400);
    }

    #[test]
    fn invalid_signature_is_rejected_and_spools_nothing() {
        let (challenges, spool) = no_state();
        let signing_key = test_keypair();
        let other_key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(signing_key.verifying_key().as_bytes());

        let challenge_resp = route(
            "GET",
            "/api/challenge",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        let challenge_json: serde_json::Value =
            serde_json::from_slice(&challenge_resp.body).unwrap();
        let challenge_hex = challenge_json["challenge"].as_str().unwrap().to_string();
        let challenge_bytes = HEXLOWER.decode(challenge_hex.as_bytes()).unwrap();

        // Sign with a *different* key than the claimed pubkey.
        let bad_sig = other_key.sign(&challenge_bytes);
        let sig_hex = HEXLOWER.encode(&bad_sig.to_bytes());

        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}"}}"#
        );
        let resp = route(
            "POST",
            "/api/identity",
            None,
            body.as_bytes(),
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 400);
        assert!(
            std::fs::read_dir(spool.path()).unwrap().next().is_none(),
            "spool should be empty"
        );
    }

    #[test]
    fn reused_challenge_is_rejected_second_time() {
        let (challenges, spool) = no_state();
        let signing_key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(signing_key.verifying_key().as_bytes());

        let challenge_resp = route(
            "GET",
            "/api/challenge",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        let challenge_json: serde_json::Value =
            serde_json::from_slice(&challenge_resp.body).unwrap();
        let challenge_hex = challenge_json["challenge"].as_str().unwrap().to_string();
        let challenge_bytes = HEXLOWER.decode(challenge_hex.as_bytes()).unwrap();

        let sig = signing_key.sign(&challenge_bytes);
        let sig_hex = HEXLOWER.encode(&sig.to_bytes());
        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}"}}"#
        );

        let first = route(
            "POST",
            "/api/identity",
            None,
            body.as_bytes(),
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(first.status, 200);

        let second = route(
            "POST",
            "/api/identity",
            None,
            body.as_bytes(),
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(second.status, 400);
    }

    #[test]
    fn unknown_challenge_is_rejected() {
        let (challenges, spool) = no_state();
        let signing_key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(signing_key.verifying_key().as_bytes());
        let fake_challenge = "00".repeat(32);
        let fake_challenge_bytes = HEXLOWER.decode(fake_challenge.as_bytes()).unwrap();

        let sig = signing_key.sign(&fake_challenge_bytes);
        let sig_hex = HEXLOWER.encode(&sig.to_bytes());
        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{fake_challenge}"}}"#
        );

        let resp = route(
            "POST",
            "/api/identity",
            None,
            body.as_bytes(),
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 400);
        assert!(std::fs::read_dir(spool.path()).unwrap().next().is_none());
    }

    const SAMPLE_DIRECTORY: &str = r#"{
        "version": 1,
        "node": { "node_id": "n1", "subnet": "10.42.1.0/24", "backhaul_addr": "10.254.1.1" },
        "neighbors": [ { "node_id": "n2", "addrs": ["10.254.1.2"], "subnet": null } ],
        "identities": [ { "username": "alice", "display_name": "Alice" } ],
        "services": [ { "name": "grafana", "ip": "10.42.1.5", "port": 3000, "protocol": "http" } ]
    }"#;

    #[test]
    fn directory_endpoint_serves_file_contents() {
        let (challenges, spool) = no_state();
        let (cache, dir) = no_directory();
        let directory_path = dir.path().join("directory.json");
        std::fs::write(&directory_path, SAMPLE_DIRECTORY).unwrap();

        let resp = route(
            "GET",
            "/api/directory",
            None,
            b"",
            &challenges,
            spool.path(),
            &cache,
            &directory_path,
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/json");
        let value: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(value["node"]["node_id"], "n1");
        assert_eq!(value["neighbors"][0]["node_id"], "n2");
        assert_eq!(value["identities"][0]["username"], "alice");
        assert_eq!(value["services"][0]["name"], "grafana");
    }

    #[test]
    fn directory_endpoint_returns_empty_directory_when_file_missing() {
        let (challenges, spool) = no_state();
        let (cache, dir) = no_directory();
        let missing_path = dir.path().join("does-not-exist.json");

        let resp = route(
            "GET",
            "/api/directory",
            None,
            b"",
            &challenges,
            spool.path(),
            &cache,
            &missing_path,
        );
        assert_eq!(resp.status, 200);
        assert_eq!(
            resp.body,
            br#"{"version":1,"node":null,"neighbors":[],"identities":[],"services":[]}"#.to_vec()
        );
    }

    #[test]
    fn node_endpoint_extracts_node_section() {
        let (challenges, spool) = no_state();
        let (cache, dir) = no_directory();
        let directory_path = dir.path().join("directory.json");
        std::fs::write(&directory_path, SAMPLE_DIRECTORY).unwrap();

        let resp = route(
            "GET",
            "/api/node",
            None,
            b"",
            &challenges,
            spool.path(),
            &cache,
            &directory_path,
        );
        assert_eq!(resp.status, 200);
        let value: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(value["node_id"], "n1");
        assert_eq!(value["backhaul_addr"], "10.254.1.1");
    }

    #[test]
    fn node_endpoint_returns_empty_object_when_node_missing() {
        let (challenges, spool) = no_state();
        let (cache, dir) = no_directory();
        let missing_path = dir.path().join("does-not-exist.json");

        let resp = route(
            "GET",
            "/api/node",
            None,
            b"",
            &challenges,
            spool.path(),
            &cache,
            &missing_path,
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"{}".to_vec());
    }
}
