// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for health endpoints and the AS endpoints, including
//! `hs_token` validation. A wiremock server stands in for a real homeserver.

use std::{collections::BTreeMap, net::SocketAddr};

use imogo_provisioner::{
    config::HomeserverConfig,
    http::{appservice::AppState, router},
    matrix::MatrixRegistry,
};
use serde_json::json;
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

async fn start_provisioner(registry: MatrixRegistry) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().expect("local addr");

    let app = router::build(registry);

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    addr
}

#[tokio::test]
async fn healthz_returns_ok() {
    let registry = MatrixRegistry::build(&BTreeMap::new())
        .await
        .expect("build empty registry");
    let addr = start_provisioner(registry).await;

    let resp = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn readyz_with_no_homeservers_is_ok() {
    let registry = MatrixRegistry::build(&BTreeMap::new())
        .await
        .expect("build empty registry");
    let addr = start_provisioner(registry).await;

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
    let registry = MatrixRegistry::build(&homeservers).await.expect("build");

    let addr = start_provisioner(registry).await;

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
    let registry = MatrixRegistry::build(&homeservers).await.expect("build");
    let addr = start_provisioner(registry).await;

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
    let registry = MatrixRegistry::build(&homeservers).await.expect("build");
    let addr = start_provisioner(registry).await;

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
    let registry = MatrixRegistry::build(&homeservers).await.expect("build");
    let addr = start_provisioner(registry).await;

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
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_object());
}

#[tokio::test]
async fn transactions_endpoint_unknown_homeserver_returns_404() {
    let registry = MatrixRegistry::build(&BTreeMap::new())
        .await
        .expect("build");
    let addr = start_provisioner(registry).await;

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
    let registry = MatrixRegistry::build(&homeservers).await.expect("build");
    let addr = start_provisioner(registry).await;

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

// AppState only used to verify the type compiles in isolation.
#[allow(dead_code)]
fn _appstate_compiles(_state: AppState) {}
