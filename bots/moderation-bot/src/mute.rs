// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Mute, unmute, and auto-unmute coordination.
//!
//! A mute sets the target user's Power Level to -1 (a value less than the
//! room's `events_default`, which prevents sending in well-configured rooms)
//! and spawns a Tokio task that restores the previous PL after the requested
//! duration. The previous PL is recorded in the audit-log payload under the
//! key `previous_power_level`, together with `expires_at` (Unix seconds), so
//! that the restore step can survive a bot restart: at startup,
//! [`crate::audit::find_open_mutes`] returns every mute that has not yet been
//! unmuted, and [`schedule_recovery_tasks`] re-spawns the auto-unmute timers.

use std::time::Duration;

use chrono::Utc;
use matrix_sdk::{
    Client, Room,
    ruma::{Int, OwnedRoomId, OwnedUserId, UserId},
};
use serde_json::json;
use sqlx::SqlitePool;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::{
    audit::{self, AuditEntry, OpenMute},
    error::ModError,
    power_level,
};

/// Power level applied while a user is muted. Anything strictly less than the
/// room's `events_default` (typically 0) prevents the user from posting.
const MUTED_POWER_LEVEL: i64 = -1;

/// Apply a mute: record previous PL, set PL to -1, append `user_muted` audit
/// entry, and spawn the auto-unmute task. Returns `Ok(expires_at)` on success.
///
/// # Errors
///
/// Returns [`ModError::Matrix`] for matrix-sdk failures and [`ModError::Db`]
/// for audit-log persistence errors.
pub async fn apply_mute(
    pool: &SqlitePool,
    client: &Client,
    room: &Room,
    actor_user_id: &str,
    target_user_id: &UserId,
    duration_secs: u64,
    reason: Option<&str>,
) -> Result<i64, ModError> {
    let previous_pl = power_level::current_power_level(room, target_user_id).await?;
    let now = Utc::now().timestamp();
    let expires_at = now.saturating_add(i64::try_from(duration_secs).unwrap_or(i64::MAX));

    let muted_int = Int::try_from(MUTED_POWER_LEVEL).unwrap_or_else(|_| Int::from(0));
    room.update_power_levels(vec![(target_user_id, muted_int)])
        .await
        .map_err(|e| ModError::Matrix(e.to_string()))?;

    let room_id = room.room_id().to_string();
    audit::append(
        pool,
        AuditEntry::now(
            Some(room_id.clone()),
            actor_user_id.to_string(),
            "user_muted".to_string(),
            Some(target_user_id.to_string()),
            None,
            json!({
                "previous_power_level": previous_pl,
                "expires_at": expires_at,
                "duration_secs": duration_secs,
                "reason": reason,
            }),
        ),
    )
    .await?;

    spawn_auto_unmute(
        pool.clone(),
        client.clone(),
        room.room_id().to_owned(),
        target_user_id.to_owned(),
        previous_pl,
        duration_secs,
    );

    Ok(expires_at)
}

/// Apply a manual or automatic unmute: restore PL and append `user_unmuted`
/// audit entry. `auto` is `true` when called by the auto-unmute timer.
///
/// `previous_pl` is the level to restore. For manual unmutes called from the
/// command handler, the caller should look up the most recent open mute via
/// [`audit::find_open_mutes`] and pass the recorded `previous_power_level`;
/// if no open mute exists, fall back to 0.
///
/// # Errors
///
/// Returns [`ModError::Matrix`] for matrix-sdk failures and [`ModError::Db`]
/// for audit-log persistence errors.
pub async fn apply_unmute(
    pool: &SqlitePool,
    room: &Room,
    actor_user_id: &str,
    target_user_id: &UserId,
    previous_pl: i64,
    auto: bool,
) -> Result<(), ModError> {
    let restore_int = Int::try_from(previous_pl).unwrap_or_else(|_| Int::from(0));
    room.update_power_levels(vec![(target_user_id, restore_int)])
        .await
        .map_err(|e| ModError::Matrix(e.to_string()))?;

    audit::append(
        pool,
        AuditEntry::now(
            Some(room.room_id().to_string()),
            actor_user_id.to_string(),
            "user_unmuted".to_string(),
            Some(target_user_id.to_string()),
            None,
            json!({
                "restored_power_level": previous_pl,
                "auto": auto,
            }),
        ),
    )
    .await?;
    Ok(())
}

/// Spawn a background task that sleeps for `duration_secs` and then restores
/// `previous_pl`. The task captures owned `RoomId`/`UserId` and looks up the
/// `Room` from the `Client` at firing time, which is the cheapest way to
/// avoid keeping a `Room` reference alive for hours.
pub fn spawn_auto_unmute(
    pool: SqlitePool,
    client: Client,
    room_id: OwnedRoomId,
    target_user_id: OwnedUserId,
    previous_pl: i64,
    duration_secs: u64,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(duration_secs)).await;
        if let Err(e) =
            run_auto_unmute(&pool, &client, &room_id, &target_user_id, previous_pl).await
        {
            warn!(
                error = %e,
                room_id = %room_id,
                target = %target_user_id,
                "auto-unmute task failed"
            );
        }
    });
}

async fn run_auto_unmute(
    pool: &SqlitePool,
    client: &Client,
    room_id: &OwnedRoomId,
    target_user_id: &OwnedUserId,
    previous_pl: i64,
) -> Result<(), ModError> {
    let Some(room) = client.get_room(room_id) else {
        return Err(ModError::Matrix(format!(
            "auto-unmute: room {room_id} not in client cache"
        )));
    };
    let bot_user_id = client
        .user_id()
        .map_or_else(|| "bot".to_string(), ToString::to_string);
    apply_unmute(pool, &room, &bot_user_id, target_user_id, previous_pl, true).await?;
    info!(room_id = %room_id, target = %target_user_id, "auto-unmute applied");
    Ok(())
}

/// Re-spawn auto-unmute tasks for every still-open mute found in the audit
/// log. Mutes whose `expires_at` is already in the past are unmuted
/// immediately; future mutes get a Tokio sleep with the remaining duration.
///
/// # Errors
///
/// Returns [`ModError::Db`] on audit-log read errors.
pub async fn schedule_recovery_tasks(
    pool: &SqlitePool,
    client: &Client,
    open: Vec<OpenMute>,
) -> Result<(), ModError> {
    let now = Utc::now().timestamp();
    for mute in open {
        let Ok(target) = OwnedUserId::try_from(mute.target_user_id.clone()) else {
            warn!(target = %mute.target_user_id, "recovery: invalid user id, skipping");
            continue;
        };
        let Ok(room_id) = OwnedRoomId::try_from(mute.room_id.clone()) else {
            warn!(room_id = %mute.room_id, "recovery: invalid room id, skipping");
            continue;
        };
        if mute.expires_at <= now {
            // Already expired: try to unmute right now.
            if let Some(room) = client.get_room(&room_id) {
                let bot_user_id = client
                    .user_id()
                    .map_or_else(|| "bot".to_string(), ToString::to_string);
                if let Err(e) = apply_unmute(
                    pool,
                    &room,
                    &bot_user_id,
                    &target,
                    mute.previous_power_level,
                    true,
                )
                .await
                {
                    warn!(error = %e, "recovery: immediate unmute failed");
                }
            } else {
                warn!(room_id = %room_id, "recovery: room not in client cache, skip immediate unmute");
            }
        } else {
            let remaining = u64::try_from(mute.expires_at - now).unwrap_or(0);
            info!(
                room_id = %room_id,
                target = %target,
                remaining_secs = remaining,
                "recovery: rescheduling auto-unmute"
            );
            spawn_auto_unmute(
                pool.clone(),
                client.clone(),
                room_id,
                target,
                mute.previous_power_level,
                remaining,
            );
        }
    }
    Ok(())
}
