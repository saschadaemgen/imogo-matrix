// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Webhook endpoint for license server calls.
//!
//! In Briefing-02c-1 the endpoint validates signature, timestamp, and nonce.
//! Verified requests are logged and acknowledged with HTTP 202.
//! Briefing-02c-3 will dispatch verified requests to the business logic.

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::appservice::AppState;
use crate::webhook::{
    HEADER_KEY_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP, WebhookVerifyError,
};

/// Successful response payload.
#[derive(Debug, Serialize)]
pub struct WebhookAck {
    /// Always `"verified"` on success.
    pub status: &'static str,
    /// Echoes back the verified `key_id`.
    pub key_id: String,
    /// Echoes back the accepted nonce.
    pub nonce: String,
}

/// Generic error payload returned with HTTP 401.
#[derive(Debug, Serialize)]
pub struct WebhookError {
    /// Stable machine-readable error label.
    pub error: &'static str,
}

/// Optional payload structure. The actual schema is defined per event type
/// and lives in 02c-3. Here we accept any JSON.
#[derive(Debug, Deserialize)]
pub struct AnyEvent {
    /// Event type discriminator. Unused in 02c-1.
    #[allow(dead_code)]
    #[serde(rename = "type")]
    pub event_type: Option<String>,
}

/// `POST /webhook/license`
///
/// Verifies signature, timestamp, and nonce. On success returns 202 Accepted.
/// On any verification failure returns 401 Unauthorized.
pub async fn license_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    let timestamp = header_str(&headers, HEADER_TIMESTAMP);
    let nonce = header_str(&headers, HEADER_NONCE);
    let signature = header_str(&headers, HEADER_SIGNATURE);
    let key_id = header_str(&headers, HEADER_KEY_ID);

    let path_and_query = uri
        .path_and_query()
        .map_or_else(|| uri.path(), axum::http::uri::PathAndQuery::as_str);

    let result = state
        .webhook_verifier
        .verify(
            "POST",
            path_and_query,
            timestamp,
            nonce,
            signature,
            key_id,
            &body,
        )
        .await;

    match result {
        Ok(verified) => {
            info!(
                key_id = verified.key_id.as_str(),
                nonce = verified.nonce.as_str(),
                ts = verified.timestamp_unix_seconds,
                body_len = body.len(),
                "license webhook verified (no business logic yet)"
            );
            (
                StatusCode::ACCEPTED,
                Json(WebhookAck {
                    status: "verified",
                    key_id: verified.key_id,
                    nonce: verified.nonce,
                }),
            )
                .into_response()
        }
        Err(e) => {
            warn!(error = %e, "license webhook rejected");
            let label = match e {
                WebhookVerifyError::MissingHeader(_) => "missing_header",
                WebhookVerifyError::MalformedHeader { .. } => "malformed_header",
                WebhookVerifyError::TimestampOutOfRange => "timestamp_out_of_range",
                WebhookVerifyError::NonceReplay => "nonce_replay",
                WebhookVerifyError::UnknownKeyId => "unknown_key_id",
                WebhookVerifyError::BadSignature => "bad_signature",
            };
            (
                StatusCode::UNAUTHORIZED,
                Json(WebhookError { error: label }),
            )
                .into_response()
        }
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}
