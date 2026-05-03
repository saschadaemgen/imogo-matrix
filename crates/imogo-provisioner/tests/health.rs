// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for health endpoints and the AS endpoints.

use std::{collections::BTreeMap, net::SocketAddr};

use imogo_provisioner::{
    accounts::AccountsRepo,
    audit::AuditLog,
    b2c::B2cService,
    capability::CapabilityVerifier,
    config::{B2cConfig, DbConfig, HomeserverConfig, ProvisioningConfig},
    db,
    http::{appservice::AppState, router},
    keys::{CapabilityKeyRegistry, KeyRegistry},
    matrix::MatrixRegistry,
    nonce_store::NonceStore,
    provisioning::ProvisioningService,
    webhook::WebhookVerifier,
};
use serde_json::json;
use tempfile::TempDir;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const TEST_AS_TOKEN: &str = "test_as_token_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TEST_HS_TOKEN: &str = "test_hs_token_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn start_mock_homeserver() -> MockServer {
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

fn test_homeserver_config(url: Url) -> HomeserverConfig {
    HomeserverConfig {
        url,
        server_name: "test.local".to_string(),
        appservice_id: "imogo-provisioner".to_string(),
        as_token: TEST_AS_TOKEN.to_string(),
        hs_token: TEST_HS_TOKEN.to_string(),
        sender_localpart: "imogo-provisioner".to_string(),
    }
}

async fn build_test_state(homeservers: BTreeMap<String, HomeserverConfig>) -> (AppState, TempDir) {
    let tmp = tempfile::tempdir().expect("tmp");
    let db_cfg = DbConfig {
        path: tmp.path().join("test.db").to_string_lossy().into_owned(),
        max_connections: 2,
    };
    let pool = db::open_pool(&db_cfg).await.expect("db open");
    let audit_log = AuditLog::new(pool.clone());
    let nonce_store = NonceStore::new(pool.clone(), 600);
    let accounts = AccountsRepo::new(pool.clone());
    let registry = MatrixRegistry::build(&homeservers).await.expect("registry");
    let verifier = WebhookVerifier::new(KeyRegistry::default(), nonce_store, 300);
    let provisioning = ProvisioningService::new(
        accounts,
        audit_log.clone(),
        registry.clone(),
        ProvisioningConfig::default(),
        reqwest::Client::new(),
    );
    let capability_verifier =
        CapabilityVerifier::new(CapabilityKeyRegistry::default(), pool.clone());
    let b2c = B2cService::new(
        pool,
        audit_log.clone(),
        registry.clone(),
        B2cConfig::default(),
        reqwest::Client::new(),
    );
    (
        AppState {
            registry,
            webhook_verifier: verifier,
            audit_log,
            provisioning,
            b2c,
            capability_verifier,
        },
        tmp,
    )
}

async fn start_provisioner(state: AppState) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().expect("local addr");

    let app = router::build(
        state.registry.clone(),
        state.webhook_verifier.clone(),
        state.audit_log.clone(),
        state.provisioning.clone(),
        state.b2c.clone(),
        state.capability_verifier.clone(),
    );

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (state, _tmp) = build_test_state(BTreeMap::new()).await;
    let addr = start_provisioner(state).await;

    let resp = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn readyz_with_no_homeservers_is_ok() {
    let (state, _tmp) = build_test_state(BTreeMap::new()).await;
    let addr = start_provisioner(state).await;

    let resp = reqwest::get(format!("http://{addr}/readyz")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["total_homeservers"], 0);
}

#[tokio::test]
async fn readyz_with_reachable_homeserver_is_ok() {
    let mock = start_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert("test".to_string(), test_homeserver_config(url));

    let (state, _tmp) = build_test_state(homeservers).await;
    let addr = start_provisioner(state).await;

    let resp = reqwest::get(format!("http://{addr}/readyz")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["total_homeservers"], 1);
    assert_eq!(body["healthy_homeservers"][0], "test");
}

#[tokio::test]
async fn transactions_endpoint_rejects_missing_token() {
    let mock = start_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert("test".to_string(), test_homeserver_config(url));

    let (state, _tmp) = build_test_state(homeservers).await;
    let addr = start_provisioner(state).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!(
            "http://{addr}/_matrix/app/v1/test/transactions/txn1"
        ))
        .json(&json!({"events": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn transactions_endpoint_rejects_wrong_token() {
    let mock = start_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert("test".to_string(), test_homeserver_config(url));

    let (state, _tmp) = build_test_state(homeservers).await;
    let addr = start_provisioner(state).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!(
            "http://{addr}/_matrix/app/v1/test/transactions/txn1?access_token=wrong"
        ))
        .json(&json!({"events": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[tokio::test]
async fn transactions_endpoint_accepts_correct_token() {
    let mock = start_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert("test".to_string(), test_homeserver_config(url));

    let (state, _tmp) = build_test_state(homeservers).await;
    let addr = start_provisioner(state).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!(
            "http://{addr}/_matrix/app/v1/test/transactions/txn1?access_token={TEST_HS_TOKEN}"
        ))
        .json(&json!({"events": [{"type": "m.room.message"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn transactions_endpoint_unknown_homeserver_returns_404() {
    let (state, _tmp) = build_test_state(BTreeMap::new()).await;
    let addr = start_provisioner(state).await;

    let client = reqwest::Client::new();
    let resp = client
        .put(format!(
            "http://{addr}/_matrix/app/v1/nonexistent/transactions/txn1?access_token={TEST_HS_TOKEN}"
        ))
        .json(&json!({"events": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn user_exists_returns_404_with_correct_token() {
    let mock = start_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert("test".to_string(), test_homeserver_config(url));

    let (state, _tmp) = build_test_state(homeservers).await;
    let addr = start_provisioner(state).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{addr}/_matrix/app/v1/test/users/@kunde_42:test.local?access_token={TEST_HS_TOKEN}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
