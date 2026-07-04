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
use std::time::{Duration, Instant};

use data_encoding::HEXLOWER;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::assets::{StaticAssets, INDEX_HTML};

/// How long an issued challenge remains redeemable.
const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);

/// In-memory store of issued, not-yet-consumed challenges: hex nonce ->
/// issue time. `mjolnir-hello` runs a single-threaded request loop, but the
/// `Mutex` keeps the store safely shareable if that ever changes.
pub type ChallengeStore = Mutex<HashMap<String, Instant>>;

pub fn new_challenge_store() -> ChallengeStore {
    Mutex::new(HashMap::new())
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
pub fn route(
    method: &str,
    path: &str,
    static_root: Option<&Path>,
    body: &[u8],
    challenges: &ChallengeStore,
    spool_dir: &Path,
) -> RouteResponse {
    match (method, path) {
        ("GET", "/api/health") => health(),

        // --- S3 (mjolnir-mesh-11l): read-only mesh state ---------------
        // ("GET", "/api/directory") => ...,  // AddrBook + ServiceEntry projection from directory_file
        // ("GET", "/api/node") => ...,       // this node's own identity/summary

        // --- S4 (mjolnir-mesh-5zn): identity ceremony -------------------
        ("GET", "/api/challenge") => issue_challenge(challenges),
        ("POST", "/api/identity") => submit_identity(body, challenges, spool_dir),

        ("GET", _) => serve_static(path, static_root),
        _ => RouteResponse { status: 404, content_type: "text/plain", body: b"not found".to_vec() },
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

    fn test_keypair() -> SigningKey {
        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    #[test]
    fn health_returns_ok_json() {
        let (challenges, spool) = no_state();
        let resp = route("GET", "/api/health", None, b"", &challenges, spool.path());
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/json");
        assert_eq!(resp.body, br#"{"status":"ok"}"#.to_vec());
    }

    #[test]
    fn root_serves_embedded_index() {
        let (challenges, spool) = no_state();
        let resp = route("GET", "/", None, b"", &challenges, spool.path());
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "text/html; charset=utf-8");
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Lightning Mesh"));
        assert!(body.contains("hello.mesh"));
    }

    #[test]
    fn unknown_path_falls_back_to_index_spa_style() {
        let (challenges, spool) = no_state();
        let resp = route("GET", "/some/client/route", None, b"", &challenges, spool.path());
        assert_eq!(resp.status, 200);
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("Lightning Mesh"));
    }

    #[test]
    fn unknown_method_is_not_found() {
        let (challenges, spool) = no_state();
        let resp = route("DELETE", "/api/health", None, b"", &challenges, spool.path());
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn static_root_override_reads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<html>dev override</html>").unwrap();
        let (challenges, spool) = no_state();
        let resp = route("GET", "/", Some(dir.path()), b"", &challenges, spool.path());
        assert_eq!(resp.status, 200);
        let body = String::from_utf8(resp.body).unwrap();
        assert!(body.contains("dev override"));
    }

    #[test]
    fn challenge_then_valid_signature_spools_and_consumes_nonce() {
        let (challenges, spool) = no_state();
        let signing_key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(signing_key.verifying_key().as_bytes());

        let challenge_resp =
            route("GET", "/api/challenge", None, b"", &challenges, spool.path());
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
        );
        assert_eq!(resp.status, 200);

        let spooled_path = spool.path().join(format!("{pubkey_hex}.json"));
        assert!(spooled_path.exists(), "expected spooled record at {spooled_path:?}");

        // Single-use: the same challenge must now be rejected.
        let replay = route(
            "POST",
            "/api/identity",
            None,
            body.as_bytes(),
            &challenges,
            spool.path(),
        );
        assert_eq!(replay.status, 400);
    }

    #[test]
    fn invalid_signature_is_rejected_and_spools_nothing() {
        let (challenges, spool) = no_state();
        let signing_key = test_keypair();
        let other_key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(signing_key.verifying_key().as_bytes());

        let challenge_resp =
            route("GET", "/api/challenge", None, b"", &challenges, spool.path());
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
        );
        assert_eq!(resp.status, 400);
        assert!(std::fs::read_dir(spool.path()).unwrap().next().is_none(), "spool should be empty");
    }

    #[test]
    fn reused_challenge_is_rejected_second_time() {
        let (challenges, spool) = no_state();
        let signing_key = test_keypair();
        let pubkey_hex = HEXLOWER.encode(signing_key.verifying_key().as_bytes());

        let challenge_resp =
            route("GET", "/api/challenge", None, b"", &challenges, spool.path());
        let challenge_json: serde_json::Value =
            serde_json::from_slice(&challenge_resp.body).unwrap();
        let challenge_hex = challenge_json["challenge"].as_str().unwrap().to_string();
        let challenge_bytes = HEXLOWER.decode(challenge_hex.as_bytes()).unwrap();

        let sig = signing_key.sign(&challenge_bytes);
        let sig_hex = HEXLOWER.encode(&sig.to_bytes());
        let body = format!(
            r#"{{"pubkey":"{pubkey_hex}","sig":"{sig_hex}","challenge":"{challenge_hex}"}}"#
        );

        let first = route("POST", "/api/identity", None, body.as_bytes(), &challenges, spool.path());
        assert_eq!(first.status, 200);

        let second = route("POST", "/api/identity", None, body.as_bytes(), &challenges, spool.path());
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

        let resp = route("POST", "/api/identity", None, body.as_bytes(), &challenges, spool.path());
        assert_eq!(resp.status, 400);
        assert!(std::fs::read_dir(spool.path()).unwrap().next().is_none());
    }
}
