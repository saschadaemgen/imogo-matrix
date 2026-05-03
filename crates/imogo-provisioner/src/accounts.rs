// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Account record persistence: maps license IDs to stable Matrix identities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;

/// Errors raised by [`AccountsRepo`].
#[derive(Debug, Error)]
#[allow(clippy::module_name_repetitions)]
pub enum AccountError {
    /// Underlying sqlx error (including unique constraint violations).
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Stable account record: maps a license id to a Matrix identity and the
/// support room created at activation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct AccountRecord {
    /// Opaque license id assigned by the imogo cloud backend.
    pub license_id: String,
    /// Stable 26-char base32 lowercase Matrix localpart.
    pub matrix_uuid: String,
    /// Logical homeserver name (matches `[matrix.homeservers.<name>]`).
    pub matrix_homeserver: String,
    /// Fully qualified Matrix user id.
    pub matrix_user_id: String,
    /// Matrix room id (NOT alias) of the customer's support room.
    pub support_room_id: String,
    /// Display name set on the Matrix profile.
    pub display_name: String,
    /// Tier label as supplied by the license server.
    pub tier: String,
    /// When the record was created.
    pub created_at: DateTime<Utc>,
}

/// New account to insert. The `created_at` timestamp is filled in by the
/// repository.
#[derive(Debug, Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct NewAccount {
    /// Opaque license id.
    pub license_id: String,
    /// Stable Matrix localpart.
    pub matrix_uuid: String,
    /// Logical homeserver name.
    pub matrix_homeserver: String,
    /// Fully qualified Matrix user id.
    pub matrix_user_id: String,
    /// Matrix room id of the support room.
    pub support_room_id: String,
    /// Display name to record (also set on the profile separately).
    pub display_name: String,
    /// Tier label.
    pub tier: String,
}

/// Handle to the `accounts` table.
#[derive(Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct AccountsRepo {
    pool: SqlitePool,
}

impl std::fmt::Debug for AccountsRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountsRepo").finish_non_exhaustive()
    }
}

impl AccountsRepo {
    /// Wrap an existing `SqlitePool`.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Look up an account by license id. Returns `None` if no record exists.
    ///
    /// # Errors
    ///
    /// Returns [`AccountError::Db`] on database errors.
    pub async fn find_by_license(
        &self,
        license_id: &str,
    ) -> Result<Option<AccountRecord>, AccountError> {
        let row: Option<AccountRow> = sqlx::query_as(
            "SELECT license_id, matrix_uuid, matrix_homeserver, matrix_user_id, \
                    support_room_id, display_name, tier, created_at \
             FROM accounts WHERE license_id = ?",
        )
        .bind(license_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(AccountRow::into_record).transpose()
    }

    /// Insert a new account record. Fails with a unique-constraint violation
    /// if `license_id` or `matrix_user_id` already exist.
    ///
    /// # Errors
    ///
    /// Returns [`AccountError::Db`] on database errors, including unique
    /// constraint violations.
    pub async fn insert(&self, new: NewAccount) -> Result<AccountRecord, AccountError> {
        let now = Utc::now();
        let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        sqlx::query(
            "INSERT INTO accounts \
                (license_id, matrix_uuid, matrix_homeserver, matrix_user_id, \
                 support_room_id, display_name, tier, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&new.license_id)
        .bind(&new.matrix_uuid)
        .bind(&new.matrix_homeserver)
        .bind(&new.matrix_user_id)
        .bind(&new.support_room_id)
        .bind(&new.display_name)
        .bind(&new.tier)
        .bind(&now_str)
        .execute(&self.pool)
        .await?;

        Ok(AccountRecord {
            license_id: new.license_id,
            matrix_uuid: new.matrix_uuid,
            matrix_homeserver: new.matrix_homeserver,
            matrix_user_id: new.matrix_user_id,
            support_room_id: new.support_room_id,
            display_name: new.display_name,
            tier: new.tier,
            created_at: now,
        })
    }
}

/// `SQLx` row binding helper.
#[derive(sqlx::FromRow)]
struct AccountRow {
    license_id: String,
    matrix_uuid: String,
    matrix_homeserver: String,
    matrix_user_id: String,
    support_room_id: String,
    display_name: String,
    tier: String,
    created_at: String,
}

impl AccountRow {
    fn into_record(self) -> Result<AccountRecord, AccountError> {
        let created_at = DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|e| AccountError::Db(sqlx::Error::Decode(Box::new(e))))?
            .with_timezone(&Utc);
        Ok(AccountRecord {
            license_id: self.license_id,
            matrix_uuid: self.matrix_uuid,
            matrix_homeserver: self.matrix_homeserver,
            matrix_user_id: self.matrix_user_id,
            support_room_id: self.support_room_id,
            display_name: self.display_name,
            tier: self.tier,
            created_at,
        })
    }
}
