// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for the license webhook endpoint.
//!
//! We generate a fresh Ed25519 keypair at test startup, inject the public key
//! into the provisioner's `KeyRegistry`, then exercise sign-then-verify
//! against the live HTTP endpoint. Each test gets its own temporary `SQLite`
//! database via tempfile so they cannot interfere via the persistent nonce
//! store.
//!
//! After Briefing-02c-3 the webhook handler runs the full provisioning
//! pipeline. Tests here mock the homeserver's `/_matrix/client/versions`
//! endpoint (so `MatrixRegistry::ping` succeeds) but NOT the `/register` or
//! `/createRoom` endpoints. A request that gets past verification therefore
//! reaches the provisioning service and fails the Tuwunel call with HTTP
//! 502 + `tuwunel_error`. That is the expected outcome for the
//! `webhook_accepts_valid_signature` and `webhook_rejects_replay` tests.

#![cfg(feature = "dev-keys")]

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use imogo_provisioner::{
    accounts::AccountsRepo,
    audit::AuditLog,
    config::{DbConfig, HomeserverConfig, ProvisioningConfig},
    db,
    http::{appservice::AppState, router},
    keys::{KeyRegistry, RegisteredKey},
    matrix::MatrixRegistry,
    nonce_store::NonceStore,
    provisioning::ProvisioningService,
    webhook::{
        HEADER_KEY_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP, WebhookVerifier,
        build_signing_string,
    },
};
use rand::rngs::OsRng;
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const TEST_KEY_ID: &str = "test-key-2026";

fn make_signing_key() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}

fn leak_static_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

async fn start_mock_homeserver() -> MockServer {
    // Mocks ONLY the /versions endpoint. Register/createRoom return 404,
    // which the provisioning layer maps to TuwunelError::Api -> HTTP 502.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "versions": ["v1.13"],
            "unstable_features": {}
        })))
        .mount(&server)
        .await;
    server
}

async fn build_state_with_key(public_key: VerifyingKey) -> (AppState, TempDir, MockServer) {
    let mock = start_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let tmp = tempfile::tempdir().expect("tmp");
    let db_cfg = DbConfig {
        path: tmp.path().join("test.db").to_string_lossy().into_owned(),
        max_connections: 2,
    };
    let pool = db::open_pool(&db_cfg).await.expect("db open");
    let nonce_store = NonceStore::new(pool.clone(), 600);
    let audit_log = AuditLog::new(pool.clone());
    let accounts = AccountsRepo::new(pool);

    let mut keys = KeyRegistry::default();
    keys.insert(RegisteredKey {
        key_id: leak_static_str(TEST_KEY_ID),
        key: public_key,
        note: leak_static_str("integration test key"),
    });
    let verifier = WebhookVerifier::new(keys, nonce_store, 300);

    let mut homeservers = BTreeMap::new();
    homeservers.insert(
        "b2b".to_string(),
        HomeserverConfig {
            url,
            server_name: "test.local".to_string(),
            appservice_id: "imogo-provisioner".to_string(),
            as_token: "test_as_token".to_string(),
            hs_token: "test_hs_token".to_string(),
            sender_localpart: "imogo-provisioner".to_string(),
        },
    );
    let registry = MatrixRegistry::build(&homeservers).await.expect("registry");

    let provisioning = ProvisioningService::new(
        accounts,
        audit_log.clone(),
        registry.clone(),
        ProvisioningConfig {
            b2b_homeserver: "b2b".to_string(),
            support_invitees: vec![],
            allowed_tiers: vec!["pro".to_string()],
        },
        reqwest::Client::new(),
    );

    (
        AppState {
            registry,
            webhook_verifier: verifier,
            audit_log,
            provisioning,
        },
        tmp,
        mock,
    )
}

async fn start_server(state: AppState) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = router::build(
        state.registry.clone(),
        state.webhook_verifier.clone(),
        state.audit_log.clone(),
        state.provisioning.clone(),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

fn body_hash_hex(body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(body);
    hex::encode(h.finalize())
}

fn now_unix() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    i64::try_from(secs).expect("Unix seconds fit in i64 until far past 2038")
}

fn sign(key: &SigningKey, method: &str, path: &str, ts: i64, nonce: &str, body: &[u8]) -> String {
    let signing_string =
        build_signing_string(method, path, &ts.to_string(), nonce, &body_hash_hex(body));
    let sig = key.sign(signing_string.as_bytes());
    STANDARD_NO_PAD.encode(sig.to_bytes())
}

/// Build a valid `LicenseActivatedPayload` JSON body.
fn valid_activation_body(license_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "event_type": "license.activated",
        "license_id": license_id,
        "tier": "pro",
        "customer": { "name": "Test User", "company": null, "email": null }
    }))
    .unwrap()
}

#[tokio::test]
async fn webhook_accepts_valid_signature() {
    let signing_key = make_signing_key();
    let (state, _tmp, _mock) = build_state_with_key(signing_key.verifying_key()).await;
    let addr = start_server(state).await;

    let body = valid_activation_body("lic-test-1");
    let ts = now_unix();
    let nonce = "n-0001-aaaa";
    let signature = sign(&signing_key, "POST", "/webhook/license", ts, nonce, &body);

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/webhook/license"))
        .header(HEADER_TIMESTAMP, ts.to_string())
        .header(HEADER_NONCE, nonce)
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_KEY_ID, TEST_KEY_ID)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request");

    // Mock homeserver does not implement /register, so the provisioning
    // layer reports a Tuwunel error after verification has already succeeded.
    assert_eq!(resp.status(), 502);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "tuwunel_error");
}

#[tokio::test]
async fn webhook_rejects_missing_signature() {
    let signing_key = make_signing_key();
    let (state, _tmp, _mock) = build_state_with_key(signing_key.verifying_key()).await;
    let addr = start_server(state).await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/webhook/license"))
        .body("{}")
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_rejects_tampered_body() {
    let signing_key = make_signing_key();
    let (state, _tmp, _mock) = build_state_with_key(signing_key.verifying_key()).await;
    let addr = start_server(state).await;

    let original = b"{\"type\":\"x\"}".to_vec();
    let tampered = b"{\"type\":\"y\"}".to_vec();
    let ts = now_unix();
    let nonce = "n-0002-bbbb";
    let signature = sign(
        &signing_key,
        "POST",
        "/webhook/license",
        ts,
        nonce,
        &original,
    );

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/webhook/license"))
        .header(HEADER_TIMESTAMP, ts.to_string())
        .header(HEADER_NONCE, nonce)
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_KEY_ID, TEST_KEY_ID)
        .body(tampered)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_rejects_old_timestamp() {
    let signing_key = make_signing_key();
    let (state, _tmp, _mock) = build_state_with_key(signing_key.verifying_key()).await;
    let addr = start_server(state).await;

    let body = b"{}".to_vec();
    let ts = now_unix() - 3600; // 1h old, well outside the 300s skew
    let nonce = "n-0003-cccc";
    let signature = sign(&signing_key, "POST", "/webhook/license", ts, nonce, &body);

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/webhook/license"))
        .header(HEADER_TIMESTAMP, ts.to_string())
        .header(HEADER_NONCE, nonce)
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_KEY_ID, TEST_KEY_ID)
        .body(body)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_rejects_replay() {
    let signing_key = make_signing_key();
    let (state, _tmp, _mock) = build_state_with_key(signing_key.verifying_key()).await;
    let addr = start_server(state).await;

    let body = valid_activation_body("lic-replay-1");
    let ts = now_unix();
    let nonce = "n-0004-dddd-replay";
    let signature = sign(&signing_key, "POST", "/webhook/license", ts, nonce, &body);

    let client = reqwest::Client::new();

    // First request: signature verified, nonce inserted, then Tuwunel fails
    // because the mock homeserver does not implement /register.
    let resp1 = client
        .post(format!("http://{addr}/webhook/license"))
        .header(HEADER_TIMESTAMP, ts.to_string())
        .header(HEADER_NONCE, nonce)
        .header(HEADER_SIGNATURE, signature.clone())
        .header(HEADER_KEY_ID, TEST_KEY_ID)
        .body(body.clone())
        .send()
        .await
        .expect("request 1");
    assert_eq!(resp1.status(), 502);

    // Second identical request: nonce already recorded -> replay rejected.
    let resp2 = client
        .post(format!("http://{addr}/webhook/license"))
        .header(HEADER_TIMESTAMP, ts.to_string())
        .header(HEADER_NONCE, nonce)
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_KEY_ID, TEST_KEY_ID)
        .body(body)
        .send()
        .await
        .expect("request 2");
    assert_eq!(resp2.status(), 401);
}

#[tokio::test]
async fn webhook_rejects_unknown_key_id() {
    let signing_key = make_signing_key();
    let (state, _tmp, _mock) = build_state_with_key(signing_key.verifying_key()).await;
    let addr = start_server(state).await;

    let body = b"{}".to_vec();
    let ts = now_unix();
    let nonce = "n-0005-eeee";
    let signature = sign(&signing_key, "POST", "/webhook/license", ts, nonce, &body);

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/webhook/license"))
        .header(HEADER_TIMESTAMP, ts.to_string())
        .header(HEADER_NONCE, nonce)
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_KEY_ID, "no-such-key")
        .body(body)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_rejects_wrong_signing_key() {
    // Generate two different keys; sign with one, verify with the other.
    let real_key = make_signing_key();
    let imposter_key = make_signing_key();
    let (state, _tmp, _mock) = build_state_with_key(real_key.verifying_key()).await;
    let addr = start_server(state).await;

    let body = b"{}".to_vec();
    let ts = now_unix();
    let nonce = "n-0006-ffff";
    let signature = sign(&imposter_key, "POST", "/webhook/license", ts, nonce, &body);

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/webhook/license"))
        .header(HEADER_TIMESTAMP, ts.to_string())
        .header(HEADER_NONCE, nonce)
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_KEY_ID, TEST_KEY_ID)
        .body(body)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn webhook_rejects_path_mismatch() {
    let signing_key = make_signing_key();
    let (state, _tmp, _mock) = build_state_with_key(signing_key.verifying_key()).await;
    let addr = start_server(state).await;

    let body = b"{}".to_vec();
    let ts = now_unix();
    let nonce = "n-0007-gggg";
    // Sign for one path, send to another.
    let signature = sign(&signing_key, "POST", "/some/other/path", ts, nonce, &body);

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/webhook/license"))
        .header(HEADER_TIMESTAMP, ts.to_string())
        .header(HEADER_NONCE, nonce)
        .header(HEADER_SIGNATURE, signature)
        .header(HEADER_KEY_ID, TEST_KEY_ID)
        .body(body)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 401);
}
