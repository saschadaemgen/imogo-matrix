// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! End-to-end test for `audit::find_open_mutes` against a temp `SQLite`
//! database populated with three mute scenarios:
//!
//! 1. an active mute (no follow-up unmute, expires in the future)
//! 2. an expired mute (no follow-up unmute, but `expires_at` is in the past)
//! 3. a manually-cancelled mute (followed by a `user_unmuted` audit row)
//!
//! Only the first two should appear in `find_open_mutes`. The recovery code
//! in `mute::schedule_recovery_tasks` decides what to do with each
//! depending on whether the expiration is past or future.

use chrono::Utc;
use moderation_bot::{
    audit::{self, AuditEntry},
    config::DatabaseConfig,
    db,
};
use serde_json::json;

async fn fresh_pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = DatabaseConfig {
        path: tmp.path().join("mute.db").to_string_lossy().into_owned(),
    };
    let pool = db::open_pool(&cfg).await.unwrap();
    (pool, tmp)
}

#[tokio::test]
async fn find_open_mutes_skips_unmuted_and_returns_active_and_expired() {
    let (pool, _tmp) = fresh_pool().await;
    let now = Utc::now().timestamp();

    // 1. Active mute: expires in 1 hour, never unmuted.
    audit::append(
        &pool,
        AuditEntry::now(
            Some("!active:test".to_string()),
            "@bot:test".to_string(),
            "user_muted".to_string(),
            Some("@active:test".to_string()),
            None,
            json!({
                "previous_power_level": 0,
                "expires_at": now + 3600,
                "duration_secs": 3600,
            }),
        ),
    )
    .await
    .unwrap();

    // 2. Expired mute: expires_at already in the past, never unmuted.
    audit::append(
        &pool,
        AuditEntry::now(
            Some("!expired:test".to_string()),
            "@bot:test".to_string(),
            "user_muted".to_string(),
            Some("@expired:test".to_string()),
            None,
            json!({
                "previous_power_level": 25,
                "expires_at": now - 3600,
                "duration_secs": 60,
            }),
        ),
    )
    .await
    .unwrap();

    // 3. Cancelled mute: a `user_muted` followed by a `user_unmuted`.
    audit::append(
        &pool,
        AuditEntry::now(
            Some("!cancelled:test".to_string()),
            "@bot:test".to_string(),
            "user_muted".to_string(),
            Some("@cancelled:test".to_string()),
            None,
            json!({
                "previous_power_level": 0,
                "expires_at": now + 3600,
            }),
        ),
    )
    .await
    .unwrap();
    audit::append(
        &pool,
        AuditEntry::now(
            Some("!cancelled:test".to_string()),
            "@admin:test".to_string(),
            "user_unmuted".to_string(),
            Some("@cancelled:test".to_string()),
            None,
            json!({ "auto": false }),
        ),
    )
    .await
    .unwrap();

    let open = audit::find_open_mutes(&pool).await.unwrap();
    assert_eq!(open.len(), 2);

    let active = open
        .iter()
        .find(|m| m.target_user_id == "@active:test")
        .expect("active mute found");
    assert_eq!(active.room_id, "!active:test");
    assert_eq!(active.previous_power_level, 0);
    assert!(active.expires_at > now);

    let expired = open
        .iter()
        .find(|m| m.target_user_id == "@expired:test")
        .expect("expired mute found");
    assert_eq!(expired.room_id, "!expired:test");
    assert_eq!(expired.previous_power_level, 25);
    assert!(expired.expires_at < now);

    assert!(
        open.iter().all(|m| m.target_user_id != "@cancelled:test"),
        "cancelled mute must not appear"
    );
}

#[tokio::test]
async fn empty_audit_log_returns_no_open_mutes() {
    let (pool, _tmp) = fresh_pool().await;
    let open = audit::find_open_mutes(&pool).await.unwrap();
    assert!(open.is_empty());
}

#[tokio::test]
async fn multiple_mutes_for_same_user_only_count_unmute_after() {
    let (pool, _tmp) = fresh_pool().await;
    let now = Utc::now().timestamp();
    let room = "!multi:test".to_string();
    let target = "@bob:test".to_string();

    // First mute, then unmute, then second mute. Only the second should be open.
    audit::append(
        &pool,
        AuditEntry::now(
            Some(room.clone()),
            "@bot:test".to_string(),
            "user_muted".to_string(),
            Some(target.clone()),
            None,
            json!({ "previous_power_level": 0, "expires_at": now - 1000 }),
        ),
    )
    .await
    .unwrap();
    audit::append(
        &pool,
        AuditEntry::now(
            Some(room.clone()),
            "@admin:test".to_string(),
            "user_unmuted".to_string(),
            Some(target.clone()),
            None,
            json!({ "auto": false }),
        ),
    )
    .await
    .unwrap();
    audit::append(
        &pool,
        AuditEntry::now(
            Some(room.clone()),
            "@bot:test".to_string(),
            "user_muted".to_string(),
            Some(target.clone()),
            None,
            json!({ "previous_power_level": 50, "expires_at": now + 1000 }),
        ),
    )
    .await
    .unwrap();

    let open = audit::find_open_mutes(&pool).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].previous_power_level, 50);
    assert!(open[0].expires_at > now);
}
