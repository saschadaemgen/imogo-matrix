// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Append-only audit log with SHA-256 hash chain.
//!
//! Every mutation in the provisioner produces one audit entry. Each entry's
//! `entry_hash` covers all of its own fields plus the previous entry's
//! `entry_hash`, so any tampering with old entries is detectable by walking
//! the chain.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use thiserror::Error;
use tracing::debug;

/// String hashed with SHA-256 to obtain the genesis `prev_hash`.
const GENESIS_SEED: &str = "imogo-matrix-audit-genesis-v1";

/// Errors raised by audit log operations.
#[derive(Debug, Error)]
#[allow(clippy::module_name_repetitions)]
pub enum AuditError {
    /// Underlying sqlx error.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    /// `prev_hash` of the current entry did not equal the previous entry's
    /// `entry_hash`.
    #[error("chain mismatch at id {id}: expected prev_hash {expected}, got {got}")]
    ChainMismatch {
        /// The id of the entry whose `prev_hash` did not match.
        id: i64,
        /// The expected previous-entry hash.
        expected: String,
        /// The actual stored `prev_hash`.
        got: String,
    },

    /// Recomputed `entry_hash` did not equal the stored value.
    #[error("entry hash mismatch at id {id}")]
    EntryHashMismatch {
        /// The id of the entry whose `entry_hash` no longer matches its
        /// recomputed value.
        id: i64,
    },
}

/// One row from the `audit_log` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct AuditEntry {
    /// Auto-increment row id (also the order in the chain).
    pub id: i64,
    /// Wall-clock time the entry was appended, UTC.
    pub created_at: DateTime<Utc>,
    /// Stable label like `webhook.license.received`.
    pub event_type: String,
    /// Producer of the entry, e.g. `license-server:dev-key-2026`.
    pub actor: String,
    /// Optional subject identifier the event refers to.
    pub subject: Option<String>,
    /// Free-form JSON payload.
    pub payload_json: String,
    /// `entry_hash` of the previous entry, or the genesis hash for id 1.
    pub prev_hash: String,
    /// SHA-256 hex of all of this entry's fields plus `prev_hash`.
    pub entry_hash: String,
}

/// New entry to be appended. The `id`, `created_at`, `prev_hash`, and
/// `entry_hash` are filled in by [`AuditLog::append`].
#[derive(Debug, Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct NewAuditEntry {
    /// Stable event-type label.
    pub event_type: String,
    /// Producer string.
    pub actor: String,
    /// Optional subject identifier.
    pub subject: Option<String>,
    /// Payload as a JSON string.
    pub payload_json: String,
}

/// Append-only audit log handle backed by `SQLite`.
#[derive(Clone)]
pub struct AuditLog {
    pool: SqlitePool,
}

impl std::fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditLog").finish_non_exhaustive()
    }
}

impl AuditLog {
    /// Wrap an existing `SqlitePool`.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Compute the seed genesis hash. Pure function, no IO.
    #[must_use]
    pub fn genesis_hash() -> String {
        let mut h = Sha256::new();
        h.update(GENESIS_SEED.as_bytes());
        hex::encode(h.finalize())
    }

    /// Append a new entry. Returns the resulting [`AuditEntry`].
    ///
    /// The implementation:
    /// 1. Reads the latest entry's `entry_hash` (or genesis if empty).
    /// 2. Inserts a row with a placeholder `entry_hash` to obtain the id.
    /// 3. Computes the canonical `entry_hash` over all fields including id.
    /// 4. Updates the row with the real `entry_hash`.
    /// 5. Commits.
    ///
    /// All of this happens in one transaction. With a single appender (the
    /// usual case for the provisioner) this is race-free; concurrent
    /// appenders can in theory observe the same `prev_hash` and produce a
    /// chain split. A future iteration may serialise appends with a Mutex.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::Db`] on database errors.
    pub async fn append(&self, entry: NewAuditEntry) -> Result<AuditEntry, AuditError> {
        let mut tx = self.pool.begin().await?;

        let prev_hash: String =
            sqlx::query_scalar("SELECT entry_hash FROM audit_log ORDER BY id DESC LIMIT 1")
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or_else(Self::genesis_hash);

        let now = Utc::now();
        let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        // Insert with a placeholder hash, fetch the assigned id, then update.
        let placeholder = "PENDING";

        let id: i64 = sqlx::query_scalar(
            "INSERT INTO audit_log (created_at, event_type, actor, subject, payload_json, prev_hash, entry_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(&now_str)
        .bind(&entry.event_type)
        .bind(&entry.actor)
        .bind(&entry.subject)
        .bind(&entry.payload_json)
        .bind(&prev_hash)
        .bind(placeholder)
        .fetch_one(&mut *tx)
        .await?;

        let entry_hash = compute_entry_hash(
            id,
            &now_str,
            &entry.event_type,
            &entry.actor,
            entry.subject.as_deref(),
            &entry.payload_json,
            &prev_hash,
        );

        sqlx::query("UPDATE audit_log SET entry_hash = ? WHERE id = ?")
            .bind(&entry_hash)
            .bind(id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        let stored = AuditEntry {
            id,
            created_at: now,
            event_type: entry.event_type,
            actor: entry.actor,
            subject: entry.subject,
            payload_json: entry.payload_json,
            prev_hash,
            entry_hash: entry_hash.clone(),
        };

        debug!(
            id,
            event_type = stored.event_type.as_str(),
            "audit entry appended"
        );
        Ok(stored)
    }

    /// List entries in id order. For verification or admin display.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::Db`] on database errors.
    pub async fn list(&self) -> Result<Vec<AuditEntry>, AuditError> {
        let rows = sqlx::query_as::<_, AuditEntryRow>(
            "SELECT id, created_at, event_type, actor, subject, payload_json, prev_hash, entry_hash \
             FROM audit_log \
             ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(AuditEntryRow::into_entry).collect()
    }

    /// Walk the chain from the first entry, recomputing every hash. Returns
    /// `Ok(())` if the chain is intact. Returns the first inconsistency
    /// found.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::ChainMismatch`] or
    /// [`AuditError::EntryHashMismatch`] at the first inconsistency. Returns
    /// [`AuditError::Db`] on database errors.
    pub async fn verify_chain(&self) -> Result<(), AuditError> {
        let entries = self.list().await?;
        let mut expected_prev = Self::genesis_hash();

        for entry in entries {
            if entry.prev_hash != expected_prev {
                return Err(AuditError::ChainMismatch {
                    id: entry.id,
                    expected: expected_prev,
                    got: entry.prev_hash,
                });
            }

            let recomputed = compute_entry_hash(
                entry.id,
                &entry
                    .created_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                &entry.event_type,
                &entry.actor,
                entry.subject.as_deref(),
                &entry.payload_json,
                &entry.prev_hash,
            );

            if recomputed != entry.entry_hash {
                return Err(AuditError::EntryHashMismatch { id: entry.id });
            }

            expected_prev = entry.entry_hash;
        }

        Ok(())
    }

    /// Number of audit entries.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::Db`] on database errors.
    #[allow(clippy::len_without_is_empty)]
    pub async fn len(&self) -> Result<i64, AuditError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }
}

/// `SQLx` row binding helper.
#[derive(sqlx::FromRow)]
struct AuditEntryRow {
    id: i64,
    created_at: String,
    event_type: String,
    actor: String,
    subject: Option<String>,
    payload_json: String,
    prev_hash: String,
    entry_hash: String,
}

impl AuditEntryRow {
    fn into_entry(self) -> Result<AuditEntry, AuditError> {
        let created_at = DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|e| AuditError::Db(sqlx::Error::Decode(Box::new(e))))?
            .with_timezone(&Utc);
        Ok(AuditEntry {
            id: self.id,
            created_at,
            event_type: self.event_type,
            actor: self.actor,
            subject: self.subject,
            payload_json: self.payload_json,
            prev_hash: self.prev_hash,
            entry_hash: self.entry_hash,
        })
    }
}

/// Build the canonical hash input. Each variable-length field is prefixed
/// with its byte length as 8-byte little-endian u64 to prevent ambiguity.
#[must_use]
pub fn compute_entry_hash(
    id: i64,
    created_at: &str,
    event_type: &str,
    actor: &str,
    subject: Option<&str>,
    payload_json: &str,
    prev_hash: &str,
) -> String {
    let mut h = Sha256::new();
    h.update(id.to_le_bytes());
    push_field(&mut h, created_at.as_bytes());
    push_field(&mut h, event_type.as_bytes());
    push_field(&mut h, actor.as_bytes());
    push_field(&mut h, subject.unwrap_or("").as_bytes());
    push_field(&mut h, payload_json.as_bytes());
    push_field(&mut h, prev_hash.as_bytes());
    hex::encode(h.finalize())
}

fn push_field(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}
