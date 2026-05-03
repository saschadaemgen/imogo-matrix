// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Database access layer.
//!
//! Wraps a `sqlx::SqlitePool` and runs migrations on startup. The same pool
//! is shared by the audit log and the persistent nonce cache.

use std::str::FromStr;

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tracing::info;

use crate::{config::DbConfig, error::Error};

/// Open the database, creating the file if missing, and run migrations.
///
/// # Errors
///
/// Returns [`Error::Db`] if the database cannot be opened or migrated.
pub async fn open_pool(cfg: &DbConfig) -> Result<SqlitePool, Error> {
    let opts = SqliteConnectOptions::from_str(&cfg.path)
        .map_err(|e| Error::Db(format!("invalid db path: {e}")))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(cfg.max_connections)
        .connect_with(opts)
        .await
        .map_err(|e| Error::Db(e.to_string()))?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| Error::Db(format!("migration error: {e}")))?;

    info!(path = cfg.path.as_str(), "database opened and migrated");
    Ok(pool)
}
