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
    audit::AuditLog, config::Config, db, error::Error, keys::KeyRegistry, matrix::MatrixRegistry,
    nonce_store::NonceStore, webhook::WebhookVerifier,
};

/// Run the HTTP server until a shutdown signal is received. The Matrix
/// registry, the `SQLite` pool, and the key registry are all built before
/// the listener accepts connections so a misconfigured component fails fast.
///
/// # Errors
///
/// Returns [`Error::Db`] if the database cannot be opened or migrated,
/// [`Error::Matrix`] if a configured homeserver cannot be turned into a
/// `matrix-sdk` client, or [`Error::Io`] if the listener cannot bind or
/// `axum::serve` returns an I/O error.
pub async fn run(config: Config) -> Result<(), Error> {
    let pool = db::open_pool(&config.db).await?;
    let audit_log = AuditLog::new(pool.clone());
    let nonce_store = NonceStore::new(pool, config.webhook.nonce_ttl_secs);

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
    let webhook_verifier =
        WebhookVerifier::new(keys, nonce_store, config.webhook.max_timestamp_skew_secs);

    let app = router::build(registry, webhook_verifier, audit_log)
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
