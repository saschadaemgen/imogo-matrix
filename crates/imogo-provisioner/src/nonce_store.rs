// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Persistent nonce cache backed by `SQLite`.
//!
//! Used by the webhook verifier to reject replays. Survives provisioner
//! restarts. Old entries are garbage collected on every insertion.

use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use thiserror::Error;
use tracing::trace;

/// Errors raised by the nonce store.
#[derive(Debug, Error)]
#[allow(clippy::module_name_repetitions)]
pub enum NonceStoreError {
    /// Underlying sqlx error.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Persistent nonce store. Cheap to clone; shares the underlying pool.
#[derive(Clone, Debug)]
pub struct NonceStore {
    pool: SqlitePool,
    ttl: Duration,
}

impl NonceStore {
    /// Construct a store using the given pool and TTL in seconds. TTL is
    /// clamped to at least 1 second.
    #[must_use]
    pub fn new(pool: SqlitePool, ttl_secs: i64) -> Self {
        Self {
            pool,
            ttl: Duration::seconds(ttl_secs.max(1)),
        }
    }

    /// Try to record a new nonce. Returns `true` if the nonce was fresh,
    /// `false` if it had already been seen (replay).
    ///
    /// Garbage collection of expired entries runs on every call so the table
    /// stays bounded.
    ///
    /// # Errors
    ///
    /// Returns [`NonceStoreError::Db`] on database errors.
    pub async fn try_insert(&self, nonce: &str, key_id: &str) -> Result<bool, NonceStoreError> {
        let now = Utc::now();
        let expires_at = now + self.ttl;
        let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let expires_str = expires_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let mut tx = self.pool.begin().await?;

        let deleted = sqlx::query("DELETE FROM webhook_nonces WHERE expires_at < ?")
            .bind(&now_str)
            .execute(&mut *tx)
            .await?
            .rows_affected();

        if deleted > 0 {
            trace!(deleted, "nonce gc removed expired entries");
        }

        let res = sqlx::query(
            "INSERT OR IGNORE INTO webhook_nonces (nonce, key_id, seen_at, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(nonce)
        .bind(key_id)
        .bind(&now_str)
        .bind(&expires_str)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(res.rows_affected() == 1)
    }

    /// True if the given nonce was previously recorded and has not yet
    /// expired.
    ///
    /// # Errors
    ///
    /// Returns [`NonceStoreError::Db`] on database errors.
    pub async fn contains(&self, nonce: &str) -> Result<bool, NonceStoreError> {
        let maybe_str: Option<String> =
            sqlx::query_scalar("SELECT expires_at FROM webhook_nonces WHERE nonce = ?")
                .bind(nonce)
                .fetch_optional(&self.pool)
                .await?;

        let Some(expires_str) = maybe_str else {
            return Ok(false);
        };

        let Ok(expires_at) = DateTime::parse_from_rfc3339(&expires_str) else {
            // A malformed timestamp is treated as expired so we do not let
            // through a possibly replayable request.
            return Ok(false);
        };

        Ok(expires_at.with_timezone(&Utc) > Utc::now())
    }

    /// Number of currently stored nonces (including expired but not yet
    /// gc'd entries). Mainly for tests and metrics.
    ///
    /// # Errors
    ///
    /// Returns [`NonceStoreError::Db`] on database errors.
    pub async fn count(&self) -> Result<i64, NonceStoreError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webhook_nonces")
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }
}
