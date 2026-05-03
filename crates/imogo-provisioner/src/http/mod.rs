// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! HTTP server module.

pub mod appservice;
pub mod health;
pub mod router;
pub mod webhook;

use std::time::Duration;

use axum::http::StatusCode;
use tokio::signal;
use tower_http::timeout::TimeoutLayer;
use tracing::info;

use crate::{
    config::Config, error::Error, keys::KeyRegistry, matrix::MatrixRegistry,
    webhook::WebhookVerifier,
};

/// Run the HTTP server until a shutdown signal is received. The Matrix
/// registry is built before the listener accepts connections so a misconfigured
/// homeserver fails fast.
///
/// # Errors
///
/// Returns [`Error::Matrix`] if any configured homeserver cannot be turned
/// into a `matrix-sdk` client, or [`Error::Io`] if the listener cannot bind
/// or `axum::serve` returns an I/O error.
pub async fn run(config: Config) -> Result<(), Error> {
    let registry = MatrixRegistry::build(&config.matrix.homeservers)
        .await
        .map_err(|e| Error::Matrix(e.to_string()))?;

    let healthy = registry.ping_all().await;
    info!(
        configured = config.matrix.homeservers.len(),
        healthy = healthy.len(),
        "matrix homeservers initialised"
    );

    let keys = KeyRegistry::with_compiled_in_keys();
    info!(
        registered_keys = keys.len(),
        "webhook key registry initialised"
    );
    let webhook_verifier = WebhookVerifier::new(
        keys,
        config.webhook.nonce_cache_capacity,
        config.webhook.max_timestamp_skew_secs,
    );

    let app = router::build(registry, webhook_verifier)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(config.http.request_timeout_secs),
        ));

    let listener = tokio::net::TcpListener::bind(config.http.listen).await?;
    info!(addr = %config.http.listen, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Wait for SIGTERM (Unix) or Ctrl-C (any platform).
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => info!("received Ctrl-C, shutting down"),
        () = terminate => info!("received SIGTERM, shutting down"),
    }
}
