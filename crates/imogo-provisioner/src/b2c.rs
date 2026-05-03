// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! B2C end-customer provisioning workflows.
//!
//! Two operations:
//! - [`B2cService::create_room`] is called by the imogo desktop app via
//!   capability-token-protected `POST /v1/b2c/rooms`. Creates a room on the
//!   open B2C homeserver, persists a `qr_token` with TTL, and returns the
//!   QR URL for printing on the invoice.
//! - [`B2cService::redeem`] is called by the public redeem page on
//!   `rechnung.imogo.de`. Looks up the `qr_token`, registers a numbered
//!   guest account, invites it into the room, and returns the access token
//!   so the customer's browser-based Matrix client can log in.

use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use thiserror::Error;
use tracing::{info, instrument};

use crate::{
    audit::{AuditError, AuditLog, NewAuditEntry},
    config::B2cConfig,
    matrix::MatrixRegistry,
    tuwunel::{PowerLevels, TuwunelClient, TuwunelError},
};

/// Errors raised by [`B2cService`].
#[derive(Debug, Error)]
#[allow(clippy::module_name_repetitions)]
pub enum B2cError {
    /// Invoice number empty after normalisation, or too long.
    #[error("invoice number invalid")]
    InvalidInvoiceNumber,

    /// Invoice subject empty or too long.
    #[error("invoice subject invalid")]
    InvalidInvoiceSubject,

    /// Requested TTL out of `[1, max_qr_token_ttl_days]`.
    #[error("ttl out of range")]
    TtlOutOfRange,

    /// Configured B2C homeserver is not registered.
    #[error("homeserver '{0}' not registered")]
    HomeserverNotRegistered(String),

    /// `next_guest_index` would exceed `guest_index_max`.
    #[error("guest limit exceeded")]
    GuestLimitExceeded,

    /// QR token expired.
    #[error("token expired")]
    TokenExpired,

    /// QR token not found in the database.
    #[error("token not found")]
    TokenNotFound,

    /// Tuwunel call failed.
    #[error("tuwunel error: {0}")]
    Tuwunel(#[from] TuwunelError),

    /// Audit append failed.
    #[error("audit error: {0}")]
    Audit(#[from] AuditError),

    /// Underlying sqlx error.
    #[error("db error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Request body for `POST /v1/b2c/rooms`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateRoomRequest {
    /// Invoice number (e.g. `"2026-0042"`).
    pub invoice_number: String,
    /// Subject line for the invoice (used in the room topic).
    pub invoice_subject: String,
    /// Topic that will be set on the support room.
    pub topic: String,
    /// Optional override of the default QR token TTL.
    #[serde(default)]
    pub qr_token_ttl_days: Option<i64>,
}

/// Response body for `POST /v1/b2c/rooms`.
#[derive(Debug, Clone, Serialize)]
pub struct CreateRoomResponse {
    /// Matrix room id (`!xxx:server`).
    pub room_id: String,
    /// Canonical room alias.
    pub room_alias: String,
    /// QR token (base64, no padding). Pass to the customer in the QR code.
    pub qr_token: String,
    /// Public URL to embed in the QR code.
    pub qr_url: String,
    /// Expiry of the QR token.
    pub expires_at: DateTime<Utc>,
}

/// Request body for `POST /v1/b2c/redeem`.
#[derive(Debug, Clone, Deserialize)]
pub struct RedeemRequest {
    /// QR token from the scan.
    pub qr_token: String,
}

/// Response body for `POST /v1/b2c/redeem`.
#[derive(Debug, Clone, Serialize)]
pub struct RedeemResponse {
    /// Base URL of the B2C homeserver.
    pub matrix_homeserver: String,
    /// The newly minted guest user id.
    pub matrix_user_id: String,
    /// Single-use access token for the guest's Matrix client.
    pub matrix_login_token: String,
    /// Matrix room id of the support room for this invoice.
    pub room_id: String,
    /// Canonical alias for the room.
    pub room_alias: String,
}

/// Service that orchestrates the B2C end-customer flow.
#[derive(Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct B2cService {
    pool: SqlitePool,
    audit: AuditLog,
    matrix: MatrixRegistry,
    cfg: B2cConfig,
    http: reqwest::Client,
}

impl std::fmt::Debug for B2cService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("B2cService")
            .field("b2c_homeserver", &self.cfg.b2c_homeserver)
            .finish_non_exhaustive()
    }
}

impl B2cService {
    /// Construct a new service.
    #[must_use]
    pub fn new(
        pool: SqlitePool,
        audit: AuditLog,
        matrix: MatrixRegistry,
        cfg: B2cConfig,
        http: reqwest::Client,
    ) -> Self {
        Self {
            pool,
            audit,
            matrix,
            cfg,
            http,
        }
    }

    /// Create a B2C room for a specific invoice.
    ///
    /// # Errors
    ///
    /// Returns [`B2cError`] for validation failures, homeserver errors, or
    /// persistence errors.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, request), fields(invoice = %request.invoice_number))]
    pub async fn create_room(
        &self,
        handwerker_license: &str,
        handwerker_user_id: &str,
        request: CreateRoomRequest,
    ) -> Result<CreateRoomResponse, B2cError> {
        let invoice_norm = normalise_invoice_number(&request.invoice_number);
        if invoice_norm.is_empty() || invoice_norm.len() > 64 {
            return Err(B2cError::InvalidInvoiceNumber);
        }
        if request.invoice_subject.trim().is_empty() || request.invoice_subject.len() > 200 {
            return Err(B2cError::InvalidInvoiceSubject);
        }

        let ttl_days = request
            .qr_token_ttl_days
            .unwrap_or(self.cfg.default_qr_token_ttl_days);
        if !(1..=self.cfg.max_qr_token_ttl_days).contains(&ttl_days) {
            return Err(B2cError::TtlOutOfRange);
        }

        let conn = self
            .matrix
            .get(&self.cfg.b2c_homeserver)
            .ok_or_else(|| B2cError::HomeserverNotRegistered(self.cfg.b2c_homeserver.clone()))?;

        let server_name = conn.config.server_name.clone();
        let qr_token = generate_qr_token();
        let now = Utc::now();
        let expires_at = now + Duration::days(ttl_days);
        let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let expires_str = expires_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let room_alias_localpart = format!("rechnung-{invoice_norm}");
        let room_alias_full = format!("#{room_alias_localpart}:{server_name}");

        // Power levels: only the provisioner bot is admin in v1. The
        // handwerker is invited later via federation from B2B (out of scope
        // for v1; documented as known limitation).
        let mut user_levels = std::collections::BTreeMap::new();
        user_levels.insert(
            format!("@{}:{server_name}", conn.config.sender_localpart),
            100,
        );

        let power_levels = PowerLevels {
            users: user_levels,
            users_default: 0,
            events_default: 50,
            state_default: 50,
            invite: 50,
            kick: 50,
            ban: 50,
            redact: 50,
        };

        let tuwunel = self.tuwunel_for(conn);

        let created = match tuwunel
            .create_room(&room_alias_localpart, &request.topic, &[], &power_levels)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                self.spawn_audit(
                    "b2c.room.create_failed",
                    Some(format!("license:{handwerker_license}")),
                    json!({
                        "handwerker_license": handwerker_license,
                        "invoice_number": invoice_norm,
                        "error": e.to_string(),
                    })
                    .to_string(),
                );
                return Err(B2cError::Tuwunel(e));
            }
        };

        sqlx::query(
            "INSERT INTO b2c_rooms \
                (qr_token, handwerker_license, handwerker_user_id, invoice_number, \
                 invoice_subject, room_id, room_alias, created_at, expires_at, next_guest_index) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
        )
        .bind(&qr_token)
        .bind(handwerker_license)
        .bind(handwerker_user_id)
        .bind(&invoice_norm)
        .bind(&request.invoice_subject)
        .bind(&created.room_id)
        .bind(&room_alias_full)
        .bind(&now_str)
        .bind(&expires_str)
        .execute(&self.pool)
        .await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "b2c.room.created".to_string(),
                actor: "system".to_string(),
                subject: Some(format!("room:{}", created.room_id)),
                payload_json: json!({
                    "handwerker_license": handwerker_license,
                    "handwerker_user_id": handwerker_user_id,
                    "invoice_number": invoice_norm,
                    "room_alias": room_alias_full,
                    "expires_at": expires_str,
                })
                .to_string(),
            })
            .await?;

        let qr_url = format!(
            "{}/{}?token={}",
            self.cfg.public_redeem_base_url, invoice_norm, qr_token
        );

        info!(
            handwerker_license,
            invoice = invoice_norm.as_str(),
            room_id = created.room_id.as_str(),
            "b2c room created"
        );

        Ok(CreateRoomResponse {
            room_id: created.room_id,
            room_alias: room_alias_full,
            qr_token,
            qr_url,
            expires_at,
        })
    }

    /// Redeem a QR token: create an anonymous guest account, invite into the
    /// pre-created room, return a Matrix login token.
    ///
    /// # Errors
    ///
    /// Returns [`B2cError::TokenNotFound`] if the QR token is unknown,
    /// [`B2cError::TokenExpired`] if it expired, or any persistence/Tuwunel
    /// error variant.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, request))]
    pub async fn redeem(&self, request: RedeemRequest) -> Result<RedeemResponse, B2cError> {
        let row: Option<RoomRow> = sqlx::query_as(
            "SELECT qr_token, handwerker_license, handwerker_user_id, invoice_number, \
                    invoice_subject, room_id, room_alias, created_at, expires_at, next_guest_index \
             FROM b2c_rooms WHERE qr_token = ?",
        )
        .bind(&request.qr_token)
        .fetch_optional(&self.pool)
        .await?;

        let Some(room) = row else {
            self.spawn_audit(
                "b2c.token.invalid",
                None,
                json!({"qr_token_prefix": prefix(&request.qr_token)}).to_string(),
            );
            return Err(B2cError::TokenNotFound);
        };

        let expires_at = DateTime::parse_from_rfc3339(&room.expires_at)
            .map_err(|e| B2cError::Db(sqlx::Error::Decode(Box::new(e))))?
            .with_timezone(&Utc);
        if expires_at < Utc::now() {
            self.audit
                .append(NewAuditEntry {
                    event_type: "b2c.token.expired".to_string(),
                    actor: "system".to_string(),
                    subject: Some(format!("room:{}", room.room_id)),
                    payload_json: json!({"qr_token_prefix": prefix(&request.qr_token)}).to_string(),
                })
                .await?;
            return Err(B2cError::TokenExpired);
        }

        if u32::try_from(room.next_guest_index).unwrap_or(u32::MAX) > self.cfg.guest_index_max {
            return Err(B2cError::GuestLimitExceeded);
        }

        let conn = self
            .matrix
            .get(&self.cfg.b2c_homeserver)
            .ok_or_else(|| B2cError::HomeserverNotRegistered(self.cfg.b2c_homeserver.clone()))?;
        let server_name = conn.config.server_name.clone();
        let homeserver_url = conn.config.url.as_str().trim_end_matches('/').to_string();
        let tuwunel = self.tuwunel_for(conn);

        let guest_localpart = format!("gast-{}-{:03}", room.invoice_number, room.next_guest_index);
        // The user_id is `@{guest_localpart}:{server_name}` once the
        // homeserver has registered the account; we use `registered.user_id`
        // returned by Tuwunel below as the authoritative value.
        let _ = &server_name;
        let initial_password = generate_password();

        // Step 1: register guest. The user_id we build locally always equals
        // the one Tuwunel returns (we set the localpart via `username`).
        let registered = tuwunel
            .register_user(&guest_localpart, &initial_password)
            .await
            .map_err(B2cError::Tuwunel)?;

        // Step 2: log in as the guest via the AS login flow to obtain a
        // single-use access token. We provide that token to the public
        // end-user; it is never persisted on our side.
        let login_token = tuwunel
            .login_appservice(&registered.user_id)
            .await
            .map_err(B2cError::Tuwunel)?;

        // Step 3: invite to the room (acting as the AS bot).
        tuwunel
            .invite_user(&room.room_id, &registered.user_id)
            .await
            .map_err(B2cError::Tuwunel)?;

        // Step 4: bump counter and persist guest record (transactional).
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE b2c_rooms SET next_guest_index = next_guest_index + 1 WHERE qr_token = ?",
        )
        .bind(&room.qr_token)
        .execute(&mut *tx)
        .await?;

        let now_str = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query(
            "INSERT INTO b2c_guests (matrix_user_id, qr_token, guest_index, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&registered.user_id)
        .bind(&room.qr_token)
        .bind(room.next_guest_index)
        .bind(&now_str)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "b2c.guest.joined".to_string(),
                actor: "system".to_string(),
                subject: Some(format!("guest:{}", registered.user_id)),
                payload_json: json!({
                    "room_id": room.room_id,
                    "guest_index": room.next_guest_index,
                    "qr_token_prefix": prefix(&room.qr_token),
                })
                .to_string(),
            })
            .await?;

        Ok(RedeemResponse {
            matrix_homeserver: homeserver_url,
            matrix_user_id: registered.user_id,
            matrix_login_token: login_token,
            room_id: room.room_id,
            room_alias: room.room_alias,
        })
    }

    fn tuwunel_for(&self, conn: &crate::matrix::HomeserverConnection) -> TuwunelClient {
        TuwunelClient::new(
            self.http.clone(),
            conn.config.url.as_str().trim_end_matches('/').to_string(),
            conn.config.as_token.clone(),
        )
    }

    /// Fire-and-forget audit entry. Used in error paths where awaiting the
    /// audit append would force the caller to handle a second error type.
    fn spawn_audit(&self, event_type: &'static str, subject: Option<String>, payload_json: String) {
        let audit = self.audit.clone();
        tokio::spawn(async move {
            let _ = audit
                .append(NewAuditEntry {
                    event_type: event_type.to_string(),
                    actor: "system".to_string(),
                    subject,
                    payload_json,
                })
                .await;
        });
    }
}

#[derive(sqlx::FromRow)]
struct RoomRow {
    qr_token: String,
    #[allow(dead_code)]
    handwerker_license: String,
    #[allow(dead_code)]
    handwerker_user_id: String,
    invoice_number: String,
    #[allow(dead_code)]
    invoice_subject: String,
    room_id: String,
    room_alias: String,
    #[allow(dead_code)]
    created_at: String,
    expires_at: String,
    next_guest_index: i64,
}

fn generate_qr_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    STANDARD_NO_PAD.encode(bytes)
}

fn generate_password() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    STANDARD_NO_PAD.encode(bytes)
}

fn prefix(token: &str) -> String {
    token[..8.min(token.len())].to_string()
}

/// Normalise an invoice number for use in Matrix room aliases.
/// Lowercase, replace anything outside `[a-z0-9-]` with `-`, collapse runs,
/// strip trailing dashes.
#[must_use]
pub fn normalise_invoice_number(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_dash = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}
