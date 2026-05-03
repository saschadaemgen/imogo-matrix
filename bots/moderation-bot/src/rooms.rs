// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Active rooms tracking. The bot only acts in rooms that have been
//! activated via `!mod aktivieren` or auto-discovered via the alias pattern.

use chrono::Utc;
use sqlx::SqlitePool;

use crate::error::ModError;

/// Insert (or replace) the activation record for `room_id`.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn activate(
    pool: &SqlitePool,
    room_id: &str,
    by: &str,
    note: Option<&str>,
) -> Result<(), ModError> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO moderation_active_rooms (room_id, activated_by, activated_at, note) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(room_id) DO UPDATE SET \
             activated_by = excluded.activated_by, \
             activated_at = excluded.activated_at, \
             note = excluded.note",
    )
    .bind(room_id)
    .bind(by)
    .bind(now)
    .bind(note)
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove the activation record for `room_id`. No-op if missing.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn deactivate(pool: &SqlitePool, room_id: &str) -> Result<(), ModError> {
    sqlx::query("DELETE FROM moderation_active_rooms WHERE room_id = ?")
        .bind(room_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Check whether `room_id` is currently active.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn is_active(pool: &SqlitePool, room_id: &str) -> Result<bool, ModError> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM moderation_active_rooms WHERE room_id = ?")
            .bind(room_id)
            .fetch_one(pool)
            .await?;
    Ok(n > 0)
}

/// Insert if missing. Returns `true` if a new row was created.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn insert_if_absent(
    pool: &SqlitePool,
    room_id: &str,
    by: &str,
    note: Option<&str>,
) -> Result<bool, ModError> {
    let now = Utc::now().timestamp();
    let res = sqlx::query(
        "INSERT OR IGNORE INTO moderation_active_rooms \
            (room_id, activated_by, activated_at, note) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(room_id)
    .bind(by)
    .bind(now)
    .bind(note)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}
