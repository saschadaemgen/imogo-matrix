// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! HTTP endpoints for B2C provisioning.
//!
//! - `POST /v1/b2c/rooms` (Bearer-token-protected) creates a room and QR
//!   token for an invoice.
//! - `POST /v1/b2c/redeem` (anonymous) registers a guest account and
//!   returns a single-use Matrix login token. Auth here is the QR token
//!   itself.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Serialize;
use tracing::{info, warn};

use super::appservice::AppState;
use crate::{
    b2c::{B2cError, CreateRoomRequest, RedeemRequest},
    capability::CapabilityError,
};

/// Generic API error payload.
#[derive(Debug, Serialize)]
pub struct ApiError {
    /// Stable machine-readable error label.
    pub error: &'static str,
    /// Optional human-readable diagnostic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `POST /v1/b2c/rooms`
///
/// Verify the bearer capability token, then create the room.
pub async fn create_room(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateRoomRequest>,
) -> impl IntoResponse {
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());

    let claims = match state
        .capability_verifier
        .verify(auth, "b2c.create_room")
        .await
    {
        Ok(c) => c,
        Err(e) => return capability_error_to_response(&e),
    };

    match state
        .b2c
        .create_room(&claims.sub, &claims.matrix_user_id, request)
        .await
    {
        Ok(resp) => {
            info!(license = claims.sub.as_str(), "b2c.create_room success");
            (StatusCode::CREATED, Json(resp)).into_response()
        }
        Err(e) => b2c_error_to_response(&e),
    }
}

/// `POST /v1/b2c/redeem`
///
/// Anonymous endpoint. Authentication is the QR token itself.
pub async fn redeem(
    State(state): State<AppState>,
    Json(request): Json<RedeemRequest>,
) -> impl IntoResponse {
    match state.b2c.redeem(request).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => b2c_error_to_response(&e),
    }
}

fn capability_error_to_response(e: &CapabilityError) -> axum::response::Response {
    warn!(error = %e, "capability token rejected");
    let code = match e {
        CapabilityError::BadAuthHeader => "bad_auth_header",
        CapabilityError::Decode(_) => "token_decode_error",
        CapabilityError::UnknownKeyId => "unknown_key_id",
        CapabilityError::Invalid(_) => "invalid_token",
        CapabilityError::Expired => "token_expired",
        CapabilityError::IatTooOld => "token_iat_too_old",
        CapabilityError::Replay => "token_replay",
        CapabilityError::MissingCapability(_) => "missing_capability",
        CapabilityError::Db(_) => "internal_error",
    };
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiError {
            error: code,
            detail: Some(e.to_string()),
        }),
    )
        .into_response()
}

fn b2c_error_to_response(e: &B2cError) -> axum::response::Response {
    warn!(error = %e, "b2c request failed");
    let (status, code) = match e {
        B2cError::InvalidInvoiceNumber
        | B2cError::InvalidInvoiceSubject
        | B2cError::TtlOutOfRange => (StatusCode::BAD_REQUEST, "validation_error"),
        B2cError::TokenNotFound | B2cError::TokenExpired => {
            (StatusCode::UNAUTHORIZED, "invalid_or_expired_token")
        }
        B2cError::GuestLimitExceeded => (StatusCode::CONFLICT, "guest_limit_exceeded"),
        B2cError::HomeserverNotRegistered(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "homeserver_not_registered",
        ),
        B2cError::Tuwunel(_) => (StatusCode::BAD_GATEWAY, "tuwunel_error"),
        B2cError::Audit(_) | B2cError::Db(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        }
    };
    (
        status,
        Json(ApiError {
            error: code,
            detail: Some(e.to_string()),
        }),
    )
        .into_response()
}
