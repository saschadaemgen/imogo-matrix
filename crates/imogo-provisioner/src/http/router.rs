// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Top-level axum router for the provisioner HTTP API.

use axum::{
    Router,
    routing::{get, post, put},
};

use super::{
    appservice::{self, AppState},
    b2c as b2c_handler, health, webhook,
};
use crate::{
    audit::AuditLog, b2c::B2cService, capability::CapabilityVerifier, matrix::MatrixRegistry,
    provisioning::ProvisioningService, webhook::WebhookVerifier,
};

/// Build the full router with shared application state.
#[allow(clippy::too_many_arguments)]
pub fn build(
    registry: MatrixRegistry,
    webhook_verifier: WebhookVerifier,
    audit_log: AuditLog,
    provisioning: ProvisioningService,
    b2c: B2cService,
    capability_verifier: CapabilityVerifier,
) -> Router {
    let state = AppState {
        registry,
        webhook_verifier,
        audit_log,
        provisioning,
        b2c,
        capability_verifier,
    };

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
        .route("/webhook/license", post(webhook::license_webhook))
        .route("/v1/b2c/rooms", post(b2c_handler::create_room))
        .route("/v1/b2c/redeem", post(b2c_handler::redeem))
        .with_state(state)
}
