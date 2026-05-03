// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Reload trigger for the banned-word cache.
//!
//! Briefing-04 keeps this thin: the cache is refreshed explicitly after
//! `ban-word add`/`remove` commands and at startup. A future iteration may
//! add a filesystem watcher analogous to the FAQ bot.

use sqlx::SqlitePool;

use crate::{banned_words::WordCache, error::ModError};

/// Refresh the banned-word cache from the database.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn refresh_banned_words(cache: &WordCache, pool: &SqlitePool) -> Result<(), ModError> {
    cache.refresh(pool).await
}
