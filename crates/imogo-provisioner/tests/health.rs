// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration test: spin up the HTTP server on an ephemeral port and
//! verify the /healthz and /readyz endpoints.

use std::net::SocketAddr;

use imogo_provisioner::{
    config::{Config, HttpConfig, LogConfig},
    http,
};

#[tokio::test]
async fn healthz_returns_ok() {
    let config = Config {
        http: HttpConfig {
            listen: "127.0.0.1:0".parse().unwrap(),
            request_timeout_secs: 5,
        },
        log: LogConfig {
            filter: "off".to_string(),
            json: false,
        },
    };

    // Start server on a free ephemeral port. We bind here directly so we
    // can learn the chosen port before serving.
    let listener = tokio::net::TcpListener::bind(config.http.listen)
        .await
        .expect("bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().expect("local addr");

    let app = imogo_provisioner::http::router::build();

    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    // Give the server a tick to be ready.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let url = format!("http://{addr}/healthz");
    let resp = reqwest::get(&url).await.expect("request");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["status"], "ok");
    assert!(body["version"].as_str().is_some());

    server.abort();

    // Suppress unused import warning when http module is not directly used.
    let _ = http::run;
}

#[tokio::test]
async fn readyz_returns_ok() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr: SocketAddr = listener.local_addr().expect("local addr");

    let app = imogo_provisioner::http::router::build();

    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let url = format!("http://{addr}/readyz");
    let resp = reqwest::get(&url).await.expect("request");
    assert_eq!(resp.status(), 200);

    server.abort();
}
