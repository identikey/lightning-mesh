//! Request routing and handlers.
//!
//! Routing is factored into a pure function (`route`) that maps a method +
//! path to a `RouteResponse`, independent of `tiny_http`'s request/response
//! types. This keeps the routing/handler logic unit-testable without binding
//! a real socket; `server.rs` is the thin adapter that drives `tiny_http` and
//! translates `RouteResponse` into wire responses.

use std::collections::HashMap;
use std::net::IpAddr;
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

/// Empty radio-telemetry doc (bead ng9), served whenever `radio.json` is
/// missing/unreadable and there is no last-good copy — e.g. a node with no
/// 802.11s mesh interface, where `mjolnir-meshd` writes no `radio.json` at all.
/// Same schema (v1) as a live snapshot, just with nulls and empty tables.
const EMPTY_RADIO: &str = r#"{"version":1,"backhaul_addr":null,"mesh_if":null,"mesh_mac":null,"channel":null,"freq_mhz":null,"collected_at_unix":null,"stations":[],"mpaths":[]}"#;

/// Last-good body of a daemon-written JSON file, keyed off the file's mtime so
/// a request only re-reads when it changed.
#[derive(Debug, Default)]
struct CachedFile {
    mtime: Option<SystemTime>,
    body: Option<String>,
}

/// Read (or serve cached) a daemon-written JSON file as a raw string, keyed on
/// mtime. On read failure, serves the last-good body — or `empty` when there
/// isn't one — without clobbering the cached mtime, so a transient stat/read
/// race can't drop a known-good snapshot. Shared by [`DirectoryCache`] and
/// [`RadioCache`]; the only difference between them is the `empty` fallback.
fn read_cached(inner: &Mutex<CachedFile>, path: &Path, empty: &str) -> String {
    let mtime = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok();

    let mut cached = inner.lock().expect("file cache poisoned");

    if (mtime.is_none() || mtime != cached.mtime)
        && let Ok(contents) = std::fs::read_to_string(path)
    {
        cached.mtime = mtime;
        cached.body = Some(contents);
    }

    cached.body.clone().unwrap_or_else(|| empty.to_string())
}

/// Cached last-good read of the daemon-written `directory.json`. Falls back to
/// (and remembers) [`EMPTY_DIRECTORY`] when the file is missing/unreadable and
/// there is no prior last-good snapshot.
#[derive(Debug, Default)]
pub struct DirectoryCache {
    inner: Mutex<CachedFile>,
}

impl DirectoryCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read (or serve cached) directory.json contents as a raw JSON string.
    fn read(&self, path: &Path) -> String {
        read_cached(&self.inner, path, EMPTY_DIRECTORY)
    }
}

/// Cached last-good read of the daemon-written `radio.json` (bead ng9). Mirrors
/// [`DirectoryCache`]; falls back to [`EMPTY_RADIO`] when the file is
/// missing/unreadable and there is no prior last-good snapshot.
#[derive(Debug, Default)]
pub struct RadioCache {
    inner: Mutex<CachedFile>,
}

impl RadioCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read (or serve cached) radio.json contents as a raw JSON string.
    fn read(&self, path: &Path) -> String {
        read_cached(&self.inner, path, EMPTY_RADIO)
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

/// Subdirectory of the identity spool that holds name-claim submissions
/// (bead mjolnir-mesh-lex). Kept OUT of the top-level spool dir on purpose:
/// meshd's identity sweep globs `spool_dir/*.json` non-recursively, so a
/// sibling `names/` subdir is invisible to it — the name-claim sweep (a
/// separate meshd stage, bead 71x) reads `spool_dir/names/*.json` instead.
const NAME_SPOOL_SUBDIR: &str = "names";

/// Domain-separation prefix for the bytes a name-claim signs. The identity
/// ceremony signs the raw nonce; a name claim signs a *distinct* preimage that
/// binds the nonce to the claimed name+port, so (a) a signature captured from
/// the identity ceremony can never be replayed as a name claim, and (b) a MITM
/// cannot swap the name/port under an otherwise-valid signature. The client
/// must sign exactly [`name_claim_signing_message`]'s bytes.
const NAME_CLAIM_DOMAIN: &str = "mjolnir-name-claim:v1";

#[derive(Debug, Deserialize)]
struct NameClaimRequest {
    pubkey: String,
    sig: String,
    challenge: String,
    /// The flat `.mesh` name to claim (single DNS label, e.g. `walkie-talkie`).
    name: String,
    /// Port the claimant listens on; folded into the signed message. Absent →
    /// `0` (an A-only claim, no SRV).
    #[serde(default)]
    port: Option<u16>,
    /// Target address the name should resolve to. Self-reported by the claimant
    /// (a server knows its own lease IP); NOT covered by the signature, so it is
    /// node-vouched, not key-authenticated — meshd sanity-checks it at ingest.
    /// Absent for a browser client that can't determine its own LAN IP; meshd
    /// then falls back to the request's source address (bead 71x). Kept out of
    /// the signed message so the shipped ceremony's preimage is unchanged.
    #[serde(default)]
    ip: Option<IpAddr>,
}

/// What meshd's name-claim sweep (bead 71x) ingests. Unlike [`IdentityRecord`],
/// this carries the `sig` as a LOAD-BEARING field: the name lane is trustless,
/// so the signature is gossiped into the CRDT and every node re-verifies it
/// against `pubkey` over [`name_claim_signing_message`] — a node cannot forge a
/// key-owned claim the way it could a `/users` record.
#[derive(Debug, Serialize)]
struct NameClaimRecord<'a> {
    pubkey: &'a str,
    sig: &'a str,
    challenge: &'a str,
    name: &'a str,
    port: u16,
    /// Self-reported target IP (see [`NameClaimRequest::ip`]); `null` when the
    /// claimant didn't supply one and meshd should use the source address.
    ip: Option<IpAddr>,
}

/// The exact bytes a name claim signs: the domain prefix, the hex challenge,
/// the (already-normalized) name, and the port, newline-separated. Shared by
/// the client (to sign) and the server (to verify) — they MUST agree byte for
/// byte, so the name is required to be pre-normalized (see [`submit_name_claim`])
/// rather than normalized here, keeping the browser from having to reproduce
/// meshd's normalization to know what it signed.
fn name_claim_signing_message(challenge_hex: &str, name: &str, port: u16) -> Vec<u8> {
    format!("{NAME_CLAIM_DOMAIN}\n{challenge_hex}\n{name}\n{port}").into_bytes()
}

#[derive(Debug, PartialEq, Eq)]
pub struct RouteResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// When set, the adapter emits `Access-Control-Allow-Origin: *`. The
    /// browser topology view (bead ng9) aggregates `GET /api/*` from OTHER
    /// nodes' overlay addresses, so those reads must be cross-origin-readable.
    pub cors: bool,
}

impl RouteResponse {
    fn json(status: u16, body: impl Into<String>) -> Self {
        RouteResponse {
            status,
            content_type: "application/json",
            body: body.into().into_bytes(),
            cors: false,
        }
    }

    fn html(status: u16, body: Vec<u8>) -> Self {
        RouteResponse {
            status,
            content_type: "text/html; charset=utf-8",
            body,
            cors: false,
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
            cors: false,
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

/// `GET /api/radio` — serve `radio.json` verbatim (bead ng9): cached, last-good
/// on read failure, empty-but-valid radio doc if there's no last-good copy
/// (e.g. a node with no mesh interface).
fn radio(cache: &RadioCache, radio_file: &Path) -> RouteResponse {
    RouteResponse::json(200, cache.read(radio_file))
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

/// `POST /api/name-claim` — validate a signed claim to a flat `.mesh` name and,
/// on success, spool it for meshd's name-claim sweep (bead mjolnir-mesh-lex).
///
/// Mirrors [`submit_identity`] but binds the signature to the claim, not just
/// the nonce: the client signs [`name_claim_signing_message`]. The server holds
/// no key — it only verifies. The name must arrive already normalized to a
/// single lowercase DNS label (so client and server sign identical bytes) and
/// must not be a reserved well-known name. Authority over the name is the KEY,
/// arbitrated downstream (leased, first-writer-wins per key, reclaimable after
/// lapse — bead p43); this endpoint only authenticates the request.
fn submit_name_claim(body: &[u8], challenges: &ChallengeStore, spool_dir: &Path) -> RouteResponse {
    let req: NameClaimRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(_) => return bad_request("invalid request body"),
    };

    // Name must be a valid, single-label DNS name and not reserved. Require it
    // to be *already* normalized: the signed message contains `req.name`
    // verbatim, so silently normalizing here would verify a signature over
    // bytes the client never signed.
    match mjolnir_mesh::normalize_device_host(&req.name) {
        Ok(normalized) if normalized == req.name => {}
        _ => return bad_request("invalid name"),
    }
    if mjolnir_mesh::is_reserved_service_name(&req.name) {
        return bad_request("reserved name");
    }

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

    let port = req.port.unwrap_or(0);
    let signed_message = name_claim_signing_message(&req.challenge, &req.name, port);

    // Peek at the challenge (known + unexpired) before verifying, so a bad
    // signature doesn't burn a nonce the legitimate holder might retry —
    // identical discipline to `submit_identity`.
    {
        let store = challenges.lock().expect("challenge store poisoned");
        match store.get(&req.challenge) {
            Some(issued_at) if issued_at.elapsed() < CHALLENGE_TTL => {}
            _ => return bad_request("unknown or expired challenge"),
        }
    }

    if verifying_key.verify(&signed_message, &signature).is_err() {
        return bad_request("invalid signature");
    }

    // Consume the challenge (single-use) now that the claim is valid.
    let mut store = challenges.lock().expect("challenge store poisoned");
    if store.remove(&req.challenge).is_none() {
        return bad_request("unknown or expired challenge");
    }
    drop(store);

    let name_spool = spool_dir.join(NAME_SPOOL_SUBDIR);
    if let Err(err) = std::fs::create_dir_all(&name_spool) {
        tracing::error!(%err, "failed to create name spool dir");
        return RouteResponse::json(500, r#"{"error":"spool unavailable"}"#);
    }

    let record = NameClaimRecord {
        pubkey: &req.pubkey,
        sig: &req.sig,
        challenge: &req.challenge,
        name: &req.name,
        port,
        ip: req.ip,
    };
    let record_json = serde_json::to_string(&record).expect("record serializes");

    // Keyed by pubkey: one pending claim per key (the one-name-per-key rule is
    // enforced authoritatively at ingest; this just keeps a key's latest claim
    // from piling up spool files).
    let dest = name_spool.join(format!("{}.json", req.pubkey));
    if let Err(err) = std::fs::write(&dest, &record_json) {
        tracing::error!(%err, path = %dest.display(), "failed to write name-claim spool entry");
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
    radio_cache: &RadioCache,
    radio_file: &Path,
) -> RouteResponse {
    let mut resp = match (method, path) {
        ("GET", "/api/health") => health(),

        // --- S3 (mjolnir-mesh-11l): read-only mesh state ---------------
        ("GET", "/api/directory") => directory(directory_cache, directory_file),
        ("GET", "/api/node") => node(directory_cache, directory_file),

        // --- radio telemetry (mjolnir-mesh-ng9): live mesh-topology view --
        ("GET", "/api/radio") => radio(radio_cache, radio_file),

        // --- S4 (mjolnir-mesh-5zn): identity ceremony -------------------
        ("GET", "/api/challenge") => issue_challenge(challenges),
        ("POST", "/api/identity") => submit_identity(body, challenges, spool_dir),
        ("POST", "/api/name-claim") => submit_name_claim(body, challenges, spool_dir),

        ("GET", _) => serve_static(path, static_root),
        _ => RouteResponse {
            status: 404,
            content_type: "text/plain",
            body: b"not found".to_vec(),
            cors: false,
        },
    };

    // Every `GET /api/*` read is aggregated cross-origin by the browser
    // topology view (bead ng9), so it must carry `Access-Control-Allow-Origin`.
    // Static assets and same-origin POSTs don't need it.
    if method == "GET" && path.starts_with("/api/") {
        resp.cors = true;
    }
    resp
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
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
            &RadioCache::new(),
            Path::new("/nonexistent"),
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
            &RadioCache::new(),
            Path::new("/nonexistent"),
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
            &RadioCache::new(),
            Path::new("/nonexistent"),
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
            &RadioCache::new(),
            Path::new("/nonexistent"),
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, b"{}".to_vec());
    }

    const SAMPLE_RADIO: &str = r#"{
        "version": 1,
        "backhaul_addr": "10.254.12.214",
        "mesh_if": "phy1-mesh0",
        "mesh_mac": "82:af:ca:e7:ba:9d",
        "channel": 36,
        "freq_mhz": 5180,
        "collected_at_unix": 1751234567,
        "stations": [ { "mac": "82:af:ca:d9:85:af", "signal_dbm": -59, "expected_throughput_mbps": 887.703, "inactive_ms": 100 } ],
        "mpaths": [ { "dst": "82:af:ca:d9:85:af", "next_hop": "82:af:ca:d9:85:af", "metric": 14 } ]
    }"#;

    #[test]
    fn radio_endpoint_serves_file_contents() {
        let (challenges, spool) = no_state();
        let dir = tempfile::tempdir().unwrap();
        let radio_path = dir.path().join("radio.json");
        std::fs::write(&radio_path, SAMPLE_RADIO).unwrap();

        let resp = route(
            "GET",
            "/api/radio",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
            &RadioCache::new(),
            &radio_path,
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/json");
        let value: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["mesh_if"], "phy1-mesh0");
        assert_eq!(value["stations"][0]["signal_dbm"], -59);
        assert_eq!(value["mpaths"][0]["metric"], 14);
    }

    #[test]
    fn radio_endpoint_returns_empty_radio_when_file_missing() {
        let (challenges, spool) = no_state();
        let dir = tempfile::tempdir().unwrap();
        let missing_path = dir.path().join("does-not-exist.json");

        let resp = route(
            "GET",
            "/api/radio",
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
            &RadioCache::new(),
            &missing_path,
        );
        assert_eq!(resp.status, 200);
        // Empty-but-valid v1 radio doc.
        let value: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(value["version"], 1);
        assert!(value["mesh_if"].is_null());
        assert_eq!(value["stations"].as_array().unwrap().len(), 0);
        assert_eq!(value["mpaths"].as_array().unwrap().len(), 0);
    }

    /// Helper: route with throwaway state, returning the `RouteResponse`.
    fn route_for(method: &str, path: &str) -> RouteResponse {
        let (challenges, spool) = no_state();
        route(
            method,
            path,
            None,
            b"",
            &challenges,
            spool.path(),
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
            &RadioCache::new(),
            Path::new("/nonexistent"),
        )
    }

    #[test]
    fn get_api_responses_carry_cors() {
        // Every GET /api/* read is aggregated cross-origin by the browser
        // topology view (ng9), so it must advertise CORS.
        assert!(route_for("GET", "/api/radio").cors);
        assert!(route_for("GET", "/api/directory").cors);
        assert!(route_for("GET", "/api/node").cors);
        assert!(route_for("GET", "/api/health").cors);
        assert!(route_for("GET", "/api/challenge").cors);
    }

    #[test]
    fn non_api_and_post_do_not_carry_cors() {
        // Static assets are same-origin; POSTs aren't cross-origin reads.
        assert!(!route_for("GET", "/").cors);
        assert!(!route_for("GET", "/some/client/route").cors);
        assert!(!route_for("POST", "/api/identity").cors);
    }

    // --- name-claim ceremony (bead mjolnir-mesh-lex) ---------------------

    /// Fetch a fresh challenge from the shared store so a follow-up name-claim
    /// can consume it (the `route_for` helper makes a throwaway store per call).
    fn fresh_challenge(challenges: &ChallengeStore, spool: &Path) -> String {
        let resp = route(
            "GET",
            "/api/challenge",
            None,
            b"",
            challenges,
            spool,
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
            &RadioCache::new(),
            Path::new("/nonexistent"),
        );
        let json: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        json["challenge"].as_str().unwrap().to_string()
    }

    fn post_name_claim(challenges: &ChallengeStore, spool: &Path, body: &str) -> RouteResponse {
        route(
            "POST",
            "/api/name-claim",
            None,
            body.as_bytes(),
            challenges,
            spool,
            &DirectoryCache::new(),
            Path::new("/nonexistent"),
            &RadioCache::new(),
            Path::new("/nonexistent"),
        )
    }

    #[test]
    fn name_claim_valid_signature_spools_to_names_subdir_and_consumes_nonce() {
        let (challenges, spool) = no_state();
        let key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(key.verifying_key().as_bytes());

        let challenge_hex = fresh_challenge(&challenges, spool.path());
        let sig = key.sign(&name_claim_signing_message(&challenge_hex, "walkie-talkie", 3000));
        let sig_hex = HEXLOWER.encode(&sig.to_bytes());
        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}","name":"walkie-talkie","port":3000,"ip":"10.42.5.23"}}"#
        );

        let resp = post_name_claim(&challenges, spool.path(), &body);
        assert_eq!(resp.status, 200, "body: {}", String::from_utf8_lossy(&resp.body));

        // Spooled under names/ (NOT the top-level dir the identity sweep globs).
        let spooled = spool.path().join("names").join(format!("{pubkey_hex}.json"));
        assert!(spooled.exists(), "expected name-claim at {spooled:?}");
        let rec: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&spooled).unwrap()).unwrap();
        assert_eq!(rec["name"], "walkie-talkie");
        assert_eq!(rec["port"], 3000);
        assert_eq!(rec["ip"], "10.42.5.23", "self-reported target IP must be carried to meshd");
        assert_eq!(rec["sig"], sig_hex, "signature must be carried for mesh-wide verify");

        // Single-use nonce: replaying the exact same claim now fails.
        let replay = post_name_claim(&challenges, spool.path(), &body);
        assert_eq!(replay.status, 400);
    }

    #[test]
    fn name_claim_wrong_signature_is_rejected_and_spools_nothing() {
        let (challenges, spool) = no_state();
        let key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(key.verifying_key().as_bytes());

        let challenge_hex = fresh_challenge(&challenges, spool.path());
        // Sign the RIGHT name but claim a DIFFERENT one — a MITM rebinding the
        // name must not verify.
        let sig = key.sign(&name_claim_signing_message(&challenge_hex, "walkie-talkie", 3000));
        let sig_hex = HEXLOWER.encode(&sig.to_bytes());
        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}","name":"impostor","port":3000}}"#
        );

        let resp = post_name_claim(&challenges, spool.path(), &body);
        assert_eq!(resp.status, 400);
        assert!(!spool.path().join("names").join(format!("{pubkey_hex}.json")).exists());
    }

    #[test]
    fn name_claim_domain_separation_rejects_identity_style_signature() {
        // A signature over the RAW nonce (what POST /api/identity signs) must
        // NOT be replayable as a name claim — the domain prefix breaks it.
        let (challenges, spool) = no_state();
        let key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(key.verifying_key().as_bytes());

        let challenge_hex = fresh_challenge(&challenges, spool.path());
        let raw_nonce = HEXLOWER.decode(challenge_hex.as_bytes()).unwrap();
        let sig = key.sign(&raw_nonce); // identity-ceremony style
        let sig_hex = HEXLOWER.encode(&sig.to_bytes());
        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}","name":"walkie-talkie","port":3000}}"#
        );

        let resp = post_name_claim(&challenges, spool.path(), &body);
        assert_eq!(resp.status, 400, "identity signature must not satisfy a name claim");
    }

    #[test]
    fn name_claim_reserved_name_is_rejected() {
        let (challenges, spool) = no_state();
        let key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(key.verifying_key().as_bytes());
        let challenge_hex = fresh_challenge(&challenges, spool.path());
        let sig = key.sign(&name_claim_signing_message(&challenge_hex, "hello", 0));
        let sig_hex = HEXLOWER.encode(&sig.to_bytes());
        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}","name":"hello"}}"#
        );
        assert_eq!(post_name_claim(&challenges, spool.path(), &body).status, 400);
    }

    #[test]
    fn name_claim_non_normalized_name_is_rejected() {
        // Uppercase / dotted / boundary-hyphen names are rejected before any
        // signature check, so the client and server always sign identical bytes.
        for bad in ["Walkie", "a.b", "-nope", "has_underscore"] {
            let (challenges, spool) = no_state();
            let key = test_keypair();
            let pubkey_hex = HEXLOWER.encode(key.verifying_key().as_bytes());
            let challenge_hex = fresh_challenge(&challenges, spool.path());
            let sig = key.sign(&name_claim_signing_message(&challenge_hex, bad, 0));
            let sig_hex = HEXLOWER.encode(&sig.to_bytes());
            let body = format!(
                r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}","name":"{bad}"}}"#
            );
            assert_eq!(
                post_name_claim(&challenges, spool.path(), &body).status,
                400,
                "name {bad:?} should be rejected"
            );
        }
    }
}
