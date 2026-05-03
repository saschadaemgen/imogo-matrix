// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! End-to-end matcher tests with a temp `SQLite` database.

use moderation_bot::{
    banned_words::{self, MatchMode, Severity, WordCache},
    config::DatabaseConfig,
    db,
};

async fn fresh_pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = DatabaseConfig {
        path: tmp.path().join("mod.db").to_string_lossy().into_owned(),
    };
    let pool = db::open_pool(&cfg).await.unwrap();
    (pool, tmp)
}

#[tokio::test]
async fn add_list_remove_round_trip() {
    let (pool, _tmp) = fresh_pool().await;

    banned_words::add(
        &pool,
        "spamtest",
        "@admin:test",
        MatchMode::Substring,
        Severity::Redact,
    )
    .await
    .unwrap();
    banned_words::add(
        &pool,
        "scheisse",
        "@admin:test",
        MatchMode::WholeWord,
        Severity::Warn,
    )
    .await
    .unwrap();

    let list = banned_words::list(&pool).await.unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|w| w.word == "spamtest"));
    assert!(list.iter().any(|w| w.word == "scheisse"));

    banned_words::remove(&pool, "spamtest").await.unwrap();
    let list = banned_words::list(&pool).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].word, "scheisse");
}

#[tokio::test]
async fn cache_refresh_picks_up_changes() {
    let (pool, _tmp) = fresh_pool().await;
    let cache = WordCache::new();

    cache.refresh(&pool).await.unwrap();
    assert!(cache.is_empty().await);

    banned_words::add(
        &pool,
        "foo",
        "@admin:test",
        MatchMode::Substring,
        Severity::Redact,
    )
    .await
    .unwrap();
    cache.refresh(&pool).await.unwrap();
    assert_eq!(cache.len().await, 1);

    let m = cache.first_match("hier ist FOObar drin").await;
    assert!(m.is_some());
    assert_eq!(m.unwrap().word, "foo");
}

#[tokio::test]
async fn add_is_idempotent_via_upsert() {
    let (pool, _tmp) = fresh_pool().await;

    banned_words::add(
        &pool,
        "foo",
        "@a:test",
        MatchMode::Substring,
        Severity::Redact,
    )
    .await
    .unwrap();
    banned_words::add(
        &pool,
        "foo",
        "@b:test",
        MatchMode::WholeWord,
        Severity::Kick,
    )
    .await
    .unwrap();

    let list = banned_words::list(&pool).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].match_mode, MatchMode::WholeWord);
    assert_eq!(list[0].severity, Severity::Kick);
}
