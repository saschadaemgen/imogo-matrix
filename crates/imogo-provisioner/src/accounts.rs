// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Account record persistence: maps license IDs to stable Matrix identities,
//! tracks lifecycle state.

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

/// Lifecycle state of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    /// Full access. The default for newly activated accounts.
    Active,
    /// Read-only after license expiry. Customer can read but not write.
    ReadOnly,
    /// Login is disabled and the account cannot post.
    Deactivated,
}

impl AccountState {
    /// Stable lower-snake-case label used in the database and the audit log.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::ReadOnly => "read_only",
            Self::Deactivated => "deactivated",
        }
    }
}

impl std::str::FromStr for AccountState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "read_only" => Ok(Self::ReadOnly),
            "deactivated" => Ok(Self::Deactivated),
            other => Err(format!("invalid account state: {other}")),
        }
    }
}

/// Stable account record: maps a license id to a Matrix identity, the
/// support room created at activation, and the current lifecycle state.
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
    /// Lifecycle state.
    pub state: AccountState,
    /// When the record was created.
    pub created_at: DateTime<Utc>,
    /// When the account transitioned to `read_only`, if at all.
    pub expired_at: Option<DateTime<Utc>>,
    /// When the account was deactivated, if at all.
    pub deactivated_at: Option<DateTime<Utc>>,
}

/// New account to insert. The `created_at` timestamp and lifecycle state
/// (`active`, no `expired_at`, no `deactivated_at`) are filled in by the
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
                    support_room_id, display_name, tier, state, created_at, \
                    expired_at, deactivated_at \
             FROM accounts WHERE license_id = ?",
        )
        .bind(license_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(AccountRow::into_record).transpose()
    }

    /// Insert a new account record. Fails with a unique-constraint violation
    /// if `license_id` or `matrix_user_id` already exist. State columns get
    /// their schema defaults (`state = 'active'`, `expired_at = NULL`,
    /// `deactivated_at = NULL`).
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
                 support_room_id, display_name, tier, created_at, state, \
                 expired_at, deactivated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', NULL, NULL)",
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
            state: AccountState::Active,
            created_at: now,
            expired_at: None,
            deactivated_at: None,
        })
    }

    /// Mark an account as expired (read-only).
    ///
    /// # Errors
    ///
    /// Returns [`AccountError::Db`] on database errors.
    pub async fn mark_expired(&self, license_id: &str) -> Result<(), AccountError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query("UPDATE accounts SET state = 'read_only', expired_at = ? WHERE license_id = ?")
            .bind(&now)
            .bind(license_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mark an account as deactivated.
    ///
    /// # Errors
    ///
    /// Returns [`AccountError::Db`] on database errors.
    pub async fn mark_deactivated(&self, license_id: &str) -> Result<(), AccountError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query(
            "UPDATE accounts SET state = 'deactivated', deactivated_at = ? WHERE license_id = ?",
        )
        .bind(&now)
        .bind(license_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Update the tier of an account.
    ///
    /// # Errors
    ///
    /// Returns [`AccountError::Db`] on database errors.
    pub async fn update_tier(&self, license_id: &str, tier: &str) -> Result<(), AccountError> {
        sqlx::query("UPDATE accounts SET tier = ? WHERE license_id = ?")
            .bind(tier)
            .bind(license_id)
            .execute(&self.pool)
            .await?;
        Ok(())
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
    state: String,
    created_at: String,
    expired_at: Option<String>,
    deactivated_at: Option<String>,
}

impl AccountRow {
    fn into_record(self) -> Result<AccountRecord, AccountError> {
        let parse_ts = |s: &str| -> Result<DateTime<Utc>, AccountError> {
            DateTime::parse_from_rfc3339(s)
                .map_err(|e| AccountError::Db(sqlx::Error::Decode(Box::new(e))))
                .map(|d| d.with_timezone(&Utc))
        };
        let created_at = parse_ts(&self.created_at)?;
        let expired_at = self.expired_at.as_deref().map(parse_ts).transpose()?;
        let deactivated_at = self.deactivated_at.as_deref().map(parse_ts).transpose()?;
        let state: AccountState = self
            .state
            .parse()
            .map_err(|e: String| AccountError::Db(sqlx::Error::Decode(e.into())))?;

        Ok(AccountRecord {
            license_id: self.license_id,
            matrix_uuid: self.matrix_uuid,
            matrix_homeserver: self.matrix_homeserver,
            matrix_user_id: self.matrix_user_id,
            support_room_id: self.support_room_id,
            display_name: self.display_name,
            tier: self.tier,
            state,
            created_at,
            expired_at,
            deactivated_at,
        })
    }
}
