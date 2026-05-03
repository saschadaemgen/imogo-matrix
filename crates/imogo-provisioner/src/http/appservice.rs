// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Inbound application service endpoints called by the Matrix homeserver.
//!
//! These endpoints implement the Matrix Application Service API, version 1:
//! <https://spec.matrix.org/latest/application-service-api/>
//!
//! In Briefing-02b we only validate the inbound `hs_token` and acknowledge
//! the calls. Real handling of pushed transactions, user existence checks,
//! and room alias existence checks is added in subsequent briefings.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{
    audit::AuditLog, b2c::B2cService, capability::CapabilityVerifier, matrix::MatrixRegistry,
    provisioning::ProvisioningService, webhook::WebhookVerifier,
};

/// Shared application state passed to every handler.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Registry of configured Matrix homeservers.
    pub registry: MatrixRegistry,
    /// Verifier for inbound license-server webhook calls.
    pub webhook_verifier: WebhookVerifier,
    /// Append-only audit log shared by all handlers that mutate state.
    pub audit_log: AuditLog,
    /// License-event provisioning workflows.
    pub provisioning: ProvisioningService,
    /// B2C end-customer provisioning workflows.
    pub b2c: B2cService,
    /// Verifier for capability tokens (b2c API auth).
    pub capability_verifier: CapabilityVerifier,
}

/// Query parameters every AS endpoint receives. The homeserver always passes
/// its `access_token` (which equals the configured `hs_token`).
#[derive(Debug, Deserialize)]
pub struct HsTokenQuery {
    /// The homeserver-issued `hs_token`, sent as the `?access_token=` query.
    pub access_token: Option<String>,
}

/// Path parameters for the `/_matrix/app/v1/{hs_name}/transactions/{txn_id}` route.
#[derive(Debug, Deserialize)]
pub struct TransactionsPath {
    /// Logical homeserver name configured under `[matrix.homeservers.<name>]`.
    pub hs_name: String,
    /// Opaque transaction id picked by the homeserver.
    pub txn_id: String,
}

/// Path parameters for the `/_matrix/app/v1/{hs_name}/users/{user_id}` route.
#[derive(Debug, Deserialize)]
pub struct UsersPath {
    /// Logical homeserver name.
    pub hs_name: String,
    /// Full Matrix user id, e.g. `@alice:example.org`.
    pub user_id: String,
}

/// Path parameters for the `/_matrix/app/v1/{hs_name}/rooms/{room_alias}` route.
#[derive(Debug, Deserialize)]
pub struct RoomsPath {
    /// Logical homeserver name.
    pub hs_name: String,
    /// Room alias, e.g. `#help:example.org`.
    pub room_alias: String,
}

/// Empty success body. The AS API expects `{}` JSON for successful responses
/// to most endpoints.
#[derive(Debug, Serialize)]
pub struct EmptyAck {}

/// Common error body shape used by the AS API.
#[derive(Debug, Serialize)]
pub struct AsError {
    /// Matrix-compatible error code, prefixed `IMOGO.` for our own codes.
    pub errcode: &'static str,
    /// Human-readable explanation.
    pub error: String,
}

impl IntoResponse for AsError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
    }
}

/// Validate the `hs_token` for the given homeserver name. Returns Ok(()) if
/// valid, otherwise an error response ready to return.
fn check_hs_token(
    state: &AppState,
    hs_name: &str,
    presented: Option<&str>,
) -> Result<(), (StatusCode, Json<AsError>)> {
    let conn = state.registry.get(hs_name).ok_or_else(|| {
        warn!(hs_name, "unknown homeserver name in inbound AS call");
        (
            StatusCode::NOT_FOUND,
            Json(AsError {
                errcode: "IMOGO.UNKNOWN_HOMESERVER",
                error: format!("no homeserver registered under name '{hs_name}'"),
            }),
        )
    })?;

    let token = presented.ok_or_else(|| {
        warn!(hs_name, "missing access_token on inbound AS call");
        (
            StatusCode::FORBIDDEN,
            Json(AsError {
                errcode: "M_FORBIDDEN",
                error: "missing access_token".to_string(),
            }),
        )
    })?;

    if !conn.verify_hs_token(token) {
        warn!(hs_name, "invalid hs_token on inbound AS call");
        return Err((
            StatusCode::FORBIDDEN,
            Json(AsError {
                errcode: "M_FORBIDDEN",
                error: "invalid access_token".to_string(),
            }),
        ));
    }

    Ok(())
}

/// `PUT /_matrix/app/v1/{hs_name}/transactions/{txn_id}`
///
/// In Briefing-02b we only acknowledge. Briefing-02c+ will dispatch events.
///
/// # Errors
///
/// Returns `(StatusCode, Json<AsError>)` if `hs_name` is unknown (404) or
/// the `access_token` query parameter is missing or wrong (403).
#[allow(clippy::unused_async)]
pub async fn transactions(
    State(state): State<AppState>,
    Path(TransactionsPath { hs_name, txn_id }): Path<TransactionsPath>,
    Query(q): Query<HsTokenQuery>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<EmptyAck>, (StatusCode, Json<AsError>)> {
    check_hs_token(&state, &hs_name, q.access_token.as_deref())?;
    let event_count = body
        .get("events")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    info!(
        hs_name = hs_name.as_str(),
        txn_id = txn_id.as_str(),
        event_count,
        "received transaction (acknowledged, no dispatch yet)"
    );
    Ok(Json(EmptyAck {}))
}

/// `GET /_matrix/app/v1/{hs_name}/users/{user_id}`
///
/// In Briefing-02b we always claim the user does not exist. In 02c we will
/// keep the same behaviour, because the provisioner creates users explicitly
/// via the admin API rather than implicitly through namespace claims.
///
/// # Errors
///
/// Always returns `Err((404, ...))` with the `IMOGO.NO_IMPLICIT_USERS`
/// errcode when the `hs_token` is valid; `(404, ...)` for unknown
/// `hs_name`; `(403, ...)` for missing or invalid token. The Err variant
/// is the documented success path for this AS endpoint shape.
#[allow(clippy::unused_async)]
pub async fn user_exists(
    State(state): State<AppState>,
    Path(UsersPath { hs_name, user_id }): Path<UsersPath>,
    Query(q): Query<HsTokenQuery>,
) -> Result<(StatusCode, Json<AsError>), (StatusCode, Json<AsError>)> {
    check_hs_token(&state, &hs_name, q.access_token.as_deref())?;
    info!(
        hs_name = hs_name.as_str(),
        user_id = user_id.as_str(),
        "user existence check"
    );
    Err((
        StatusCode::NOT_FOUND,
        Json(AsError {
            errcode: "IMOGO.NO_IMPLICIT_USERS",
            error: "users are created explicitly by the provisioner".to_string(),
        }),
    ))
}

/// `GET /_matrix/app/v1/{hs_name}/rooms/{room_alias}`
///
/// In Briefing-02b we always claim the room does not exist. Same rationale as
/// `user_exists`.
///
/// # Errors
///
/// Always returns `Err((404, ...))` with the `IMOGO.NO_IMPLICIT_ROOMS`
/// errcode when the `hs_token` is valid; `(404, ...)` for unknown
/// `hs_name`; `(403, ...)` for missing or invalid token.
#[allow(clippy::unused_async)]
pub async fn room_exists(
    State(state): State<AppState>,
    Path(RoomsPath {
        hs_name,
        room_alias,
    }): Path<RoomsPath>,
    Query(q): Query<HsTokenQuery>,
) -> Result<(StatusCode, Json<AsError>), (StatusCode, Json<AsError>)> {
    check_hs_token(&state, &hs_name, q.access_token.as_deref())?;
    info!(
        hs_name = hs_name.as_str(),
        room_alias = room_alias.as_str(),
        "room alias existence check"
    );
    Err((
        StatusCode::NOT_FOUND,
        Json(AsError {
            errcode: "IMOGO.NO_IMPLICIT_ROOMS",
            error: "rooms are created explicitly by the provisioner".to_string(),
        }),
    ))
}
