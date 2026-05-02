// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Top-level axum router for the provisioner HTTP API.

use axum::{Router, routing::get};

use super::health;

/// Build the full router. In subsequent briefings additional routes
/// (webhook, admin endpoints) will be added here.
pub fn build() -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
}
