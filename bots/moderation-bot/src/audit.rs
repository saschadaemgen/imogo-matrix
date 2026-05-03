// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Append-only audit log with SHA-256 hash chain.
//!
//! Hash input (concatenated bytes, no separator):
//!
//! ```text
//! prev_hash || timestamp(8 bytes BE i64) || room_id || actor || action
//!           || target_user_id || target_event_id || payload_json
//! ```
//!
//! `room_id`, `target_user_id`, `target_event_id` are the empty string when
//! absent. `payload_json` is the canonical `serde_json` `to_string` result of
//! the payload value (NOT pretty-printed).
//!
//! Genesis hash is `"0".repeat(64)`.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::error::ModError;

/// Genesis hash for the empty chain.
#[must_use]
pub fn genesis_hash() -> String {
    "0".repeat(64)
}

/// Audit entry to be appended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unix seconds at the time of append.
    pub timestamp: i64,
    /// Optional room id.
    pub room_id: Option<String>,
    /// User id of the actor (the bot itself for system actions).
    pub actor_user_id: String,
    /// Stable label like `room_activated`, `user_banned`, `auto_moderation_redact`.
    pub action: String,
    /// Optional target user id.
    pub target_user_id: Option<String>,
    /// Optional target event id (for redact/pin operations).
    pub target_event_id: Option<String>,
    /// Free-form payload as a JSON value.
    pub payload: serde_json::Value,
}

impl AuditEntry {
    /// Construct an entry, defaulting timestamp to `Utc::now()`.
    #[must_use]
    pub fn now(
        room_id: Option<String>,
        actor_user_id: String,
        action: String,
        target_user_id: Option<String>,
        target_event_id: Option<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            timestamp: Utc::now().timestamp(),
            room_id,
            actor_user_id,
            action,
            target_user_id,
            target_event_id,
            payload,
        }
    }
}

/// Compute the SHA-256 hash of an entry plus the previous hash.
#[must_use]
pub fn compute_hash(entry: &AuditEntry, prev_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(entry.timestamp.to_be_bytes());
    hasher.update(entry.room_id.as_deref().unwrap_or("").as_bytes());
    hasher.update(entry.actor_user_id.as_bytes());
    hasher.update(entry.action.as_bytes());
    hasher.update(entry.target_user_id.as_deref().unwrap_or("").as_bytes());
    hasher.update(entry.target_event_id.as_deref().unwrap_or("").as_bytes());
    let payload_str = serde_json::to_string(&entry.payload).unwrap_or_default();
    hasher.update(payload_str.as_bytes());
    hex::encode(hasher.finalize())
}

/// Append `entry` to the audit log. Returns the assigned id.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn append(pool: &SqlitePool, entry: AuditEntry) -> Result<i64, ModError> {
    let mut tx = pool.begin().await?;

    let prev_hash: String =
        sqlx::query_scalar("SELECT hash FROM moderation_audit_log ORDER BY id DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_else(genesis_hash);

    let hash = compute_hash(&entry, &prev_hash);
    let payload_json = serde_json::to_string(&entry.payload).unwrap_or_default();

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO moderation_audit_log \
            (timestamp, room_id, actor_user_id, action, target_user_id, \
             target_event_id, payload_json, prev_hash, hash) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         RETURNING id",
    )
    .bind(entry.timestamp)
    .bind(&entry.room_id)
    .bind(&entry.actor_user_id)
    .bind(&entry.action)
    .bind(&entry.target_user_id)
    .bind(&entry.target_event_id)
    .bind(&payload_json)
    .bind(&prev_hash)
    .bind(&hash)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(id)
}

/// Walk the audit log and verify every entry's hash chains correctly.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors. Returns
/// `Ok(false)` if the chain is broken.
pub async fn verify_chain(pool: &SqlitePool) -> Result<bool, ModError> {
    let rows = sqlx::query(
        "SELECT timestamp, room_id, actor_user_id, action, target_user_id, \
                target_event_id, payload_json, prev_hash, hash \
         FROM moderation_audit_log \
         ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut expected_prev = genesis_hash();
    for row in rows {
        let timestamp: i64 = row.try_get("timestamp")?;
        let room_id: Option<String> = row.try_get("room_id")?;
        let actor_user_id: String = row.try_get("actor_user_id")?;
        let action: String = row.try_get("action")?;
        let target_user_id: Option<String> = row.try_get("target_user_id")?;
        let target_event_id: Option<String> = row.try_get("target_event_id")?;
        let payload_json: String = row.try_get("payload_json")?;
        let prev_hash: String = row.try_get("prev_hash")?;
        let stored_hash: String = row.try_get("hash")?;

        if prev_hash != expected_prev {
            return Ok(false);
        }

        let payload: serde_json::Value = serde_json::from_str(&payload_json).unwrap_or_default();
        let entry = AuditEntry {
            timestamp,
            room_id,
            actor_user_id,
            action,
            target_user_id,
            target_event_id,
            payload,
        };
        let recomputed = compute_hash(&entry, &prev_hash);

        if recomputed != stored_hash {
            return Ok(false);
        }
        expected_prev = stored_hash;
    }

    Ok(true)
}

/// Number of entries in the audit log.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
#[allow(clippy::len_without_is_empty)]
pub async fn len(pool: &SqlitePool) -> Result<i64, ModError> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM moderation_audit_log")
        .fetch_one(pool)
        .await?;
    Ok(n)
}
