// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Webhook endpoint for license server calls.
//!
//! Pipeline (each step is a hard gate; failure short-circuits with the noted
//! HTTP status):
//!
//! 1. Verify signature/timestamp/nonce -> 401 on failure.
//! 2. Append `webhook.license.received` audit entry -> 500 on failure.
//! 3. Parse body as `LicenseActivatedPayload` -> 400 on failure.
//! 4. Check `event_type == "license.activated"` -> 400 otherwise.
//! 5. Hand off to [`ProvisioningService`](crate::provisioning::ProvisioningService);
//!    map [`ProvisioningError`] to the appropriate status code (400/500/502).

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
    provisioning::{ActivationOutcome, LicenseActivatedPayload, ProvisioningError},
    webhook::{
        HEADER_KEY_ID, HEADER_NONCE, HEADER_SIGNATURE, HEADER_TIMESTAMP, WebhookVerifyError,
    },
};

/// Hard cap on the payload bytes stored in the audit log.
const MAX_PAYLOAD_BYTES: usize = 16 * 1024;

/// Successful response payload returned with HTTP 201 (created) or 200 (already existed).
#[derive(Debug, Serialize)]
pub struct WebhookAck {
    /// `"activated"` for newly created accounts, `"existed"` for idempotent replays.
    pub status: &'static str,
    /// Echoes back the verified `key_id`.
    pub key_id: String,
    /// Echoes back the accepted nonce.
    pub nonce: String,
    /// Auto-increment id of the audit entry produced for this call.
    pub audit_id: i64,
    /// Account record and (for new accounts only) the initial password.
    pub outcome: ActivationOutcome,
}

/// Generic error payload.
#[derive(Debug, Serialize)]
pub struct WebhookError {
    /// Stable machine-readable error label.
    pub error: &'static str,
    /// Optional human-readable diagnostic, kept short.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `POST /webhook/license`
#[allow(clippy::too_many_lines)]
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

    // Step 1: verify
    let verify = state
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

    let verified = match verify {
        Ok(v) => v,
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
            return (
                StatusCode::UNAUTHORIZED,
                Json(WebhookError {
                    error: label,
                    detail: None,
                }),
            )
                .into_response();
        }
    };

    // Step 2: audit (truncated body, never the password).
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
    let audit_id = match audit_entry {
        Ok(e) => e.id,
        Err(e) => {
            warn!(error = %e, "audit append failed after verification");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(WebhookError {
                    error: "audit_failed",
                    detail: None,
                }),
            )
                .into_response();
        }
    };

    // Step 3: parse payload.
    let payload: LicenseActivatedPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "license payload parse error");
            return (
                StatusCode::BAD_REQUEST,
                Json(WebhookError {
                    error: "invalid_payload",
                    detail: Some(e.to_string()),
                }),
            )
                .into_response();
        }
    };

    // Step 4: event_type gate.
    if payload.event_type != "license.activated" {
        return (
            StatusCode::BAD_REQUEST,
            Json(WebhookError {
                error: "unsupported_event_type",
                detail: Some(payload.event_type.clone()),
            }),
        )
            .into_response();
    }

    // Step 5: provisioning.
    match state.provisioning.handle_license_activated(payload).await {
        Ok(outcome) => {
            let status_code = if outcome.already_existed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            info!(
                already_existed = outcome.already_existed,
                license_id = outcome.account.license_id.as_str(),
                matrix_user_id = outcome.account.matrix_user_id.as_str(),
                "activation handled"
            );
            (
                status_code,
                Json(WebhookAck {
                    status: if outcome.already_existed {
                        "existed"
                    } else {
                        "activated"
                    },
                    key_id: verified.key_id,
                    nonce: verified.nonce,
                    audit_id,
                    outcome,
                }),
            )
                .into_response()
        }
        Err(e) => {
            warn!(error = %e, "activation failed");
            let (status, code) = match e {
                ProvisioningError::MissingLicenseId
                | ProvisioningError::MissingCustomerName
                | ProvisioningError::InvalidTier(_) => {
                    (StatusCode::BAD_REQUEST, "validation_error")
                }
                ProvisioningError::HomeserverNotRegistered(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "homeserver_not_registered",
                ),
                ProvisioningError::Tuwunel(_) => (StatusCode::BAD_GATEWAY, "tuwunel_error"),
                ProvisioningError::Account(_) | ProvisioningError::Audit(_) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
                }
            };
            (
                status,
                Json(WebhookError {
                    error: code,
                    detail: Some(e.to_string()),
                }),
            )
                .into_response()
        }
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}
