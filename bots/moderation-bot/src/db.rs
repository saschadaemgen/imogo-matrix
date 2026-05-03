// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Database access layer.

use std::str::FromStr;

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tracing::info;

use crate::{config::DatabaseConfig, error::ModError};

/// Open the database, creating the file if missing, and run migrations.
///
/// # Errors
///
/// Returns [`ModError::Db`] if the database cannot be opened or
/// [`ModError::Migrate`] if a migration fails.
pub async fn open_pool(cfg: &DatabaseConfig) -> Result<SqlitePool, ModError> {
    let opts = SqliteConnectOptions::from_str(&cfg.path)
        .map_err(ModError::Db)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    info!(path = cfg.path.as_str(), "database opened and migrated");
    Ok(pool)
}
