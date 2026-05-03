// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Webhook endpoint for license server calls.
//!
//! After the [`WebhookVerifier`](crate::webhook::WebhookVerifier) accepts a
//! request, this handler appends one entry to the audit log and returns
//! 202 Accepted. The audit append is the single source of truth that the
//! webhook actually arrived; if it fails we return 500 so the license server
//! retries.

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
};
use serde::Serialize;
use tracing::{info, warn};

use super::appservice::AppState;
use crate::{
    audit::NewAuditEntry,
    webhook::{
        HEADER_KEY_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP, WebhookVerifyError,
    },
};

/// Hard cap on the payload bytes stored in the audit log.
const MAX_PAYLOAD_BYTES: usize = 16 * 1024;

/// Successful response payload.
#[derive(Debug, Serialize)]
pub struct WebhookAck {
    /// Always `"verified"` on success.
    pub status: &'static str,
    /// Echoes back the verified `key_id`.
    pub key_id: String,
    /// Echoes back the accepted nonce.
    pub nonce: String,
    /// Auto-increment id of the audit entry produced for this call.
    pub audit_id: i64,
}

/// Generic error payload returned with HTTP 401 (or 500 for `audit_failed`).
#[derive(Debug, Serialize)]
pub struct WebhookError {
    /// Stable machine-readable error label.
    pub error: &'static str,
}

/// `POST /webhook/license`
///
/// Verifies signature, timestamp, and nonce. On success appends an audit
/// entry and returns 202 Accepted. On verification failure returns 401
/// Unauthorized. On audit-write failure (post-verification) returns 500 so
/// the sender retries.
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
            let payload_truncated = if body.len() > MAX_PAYLOAD_BYTES {
                String::from_utf8_lossy(&body[..MAX_PAYLOAD_BYTES]).into_owned()
            } else {
                String::from_utf8_lossy(&body).into_owned()
            };

            let audit_entry = state
                .audit_log
                .append(NewAuditEntry {
                    event_type: "webhook.license.received".to_string(),
                    actor: format!("license-server:{}", verified.key_id),
                    subject: None,
                    payload_json: payload_truncated,
                })
                .await;

            match audit_entry {
                Ok(entry) => {
                    info!(
                        audit_id = entry.id,
                        key_id = verified.key_id.as_str(),
                        nonce = verified.nonce.as_str(),
                        "license webhook verified and audited"
                    );
                    (
                        StatusCode::ACCEPTED,
                        Json(WebhookAck {
                            status: "verified",
                            key_id: verified.key_id,
                            nonce: verified.nonce,
                            audit_id: entry.id,
                        }),
                    )
                        .into_response()
                }
                Err(e) => {
                    warn!(error = %e, "audit append failed after verification");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(WebhookError {
                            error: "audit_failed",
                        }),
                    )
                        .into_response()
                }
            }
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
                WebhookVerifyError::NonceStore(_) => "internal_error",
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
