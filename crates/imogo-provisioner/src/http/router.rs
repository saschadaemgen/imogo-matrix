// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Top-level axum router for the provisioner HTTP API.

use axum::{
    Router,
    routing::{get, put},
};

use super::{
    appservice::{self, AppState},
    health,
};
use crate::matrix::MatrixRegistry;

/// Build the full router with shared application state.
pub fn build(registry: MatrixRegistry) -> Router {
    let state = AppState { registry };

    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route(
            "/_matrix/app/v1/{hs_name}/transactions/{txn_id}",
            put(appservice::transactions),
        )
        .route(
            "/_matrix/app/v1/{hs_name}/users/{user_id}",
            get(appservice::user_exists),
        )
        .route(
            "/_matrix/app/v1/{hs_name}/rooms/{room_alias}",
            get(appservice::room_exists),
        )
        .with_state(state)
}
