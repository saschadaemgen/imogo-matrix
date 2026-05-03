// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for the audit-log hash chain.

use moderation_bot::{
    audit::{self, AuditEntry, compute_hash, genesis_hash},
    config::DatabaseConfig,
    db,
};
use serde_json::json;

async fn fresh_pool() -> (sqlx::SqlitePool, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = DatabaseConfig {
        path: tmp.path().join("audit.db").to_string_lossy().into_owned(),
    };
    let pool = db::open_pool(&cfg).await.unwrap();
    (pool, tmp)
}

#[test]
fn genesis_hash_is_64_zero_chars() {
    let g = genesis_hash();
    assert_eq!(g.len(), 64);
    assert!(g.chars().all(|c| c == '0'));
}

#[test]
fn compute_hash_is_deterministic() {
    let entry = AuditEntry {
        timestamp: 1_700_000_000,
        room_id: Some("!room:test".to_string()),
        actor_user_id: "@actor:test".to_string(),
        action: "user_kicked".to_string(),
        target_user_id: Some("@target:test".to_string()),
        target_event_id: None,
        payload: json!({"reason": "spam"}),
    };
    let prev = "a".repeat(64);
    let h1 = compute_hash(&entry, &prev);
    let h2 = compute_hash(&entry, &prev);
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64);
}

#[test]
fn different_entries_have_different_hashes() {
    let prev = "a".repeat(64);
    let mut e1 = AuditEntry {
        timestamp: 1,
        room_id: None,
        actor_user_id: "@a:t".to_string(),
        action: "x".to_string(),
        target_user_id: None,
        target_event_id: None,
        payload: json!({}),
    };
    let mut e2 = e1.clone();
    e2.action = "y".to_string();
    assert_ne!(compute_hash(&e1, &prev), compute_hash(&e2, &prev));

    e2 = e1.clone();
    e1.timestamp = 2;
    assert_ne!(compute_hash(&e1, &prev), compute_hash(&e2, &prev));
}

#[tokio::test]
async fn empty_chain_verifies() {
    let (pool, _tmp) = fresh_pool().await;
    assert!(audit::verify_chain(&pool).await.unwrap());
    assert_eq!(audit::len(&pool).await.unwrap(), 0);
}

#[tokio::test]
async fn appending_chains_correctly() {
    let (pool, _tmp) = fresh_pool().await;
    for i in 0..5 {
        let entry = AuditEntry::now(
            Some(format!("!room{i}:test")),
            "@bot:test".to_string(),
            format!("test_action_{i}"),
            None,
            None,
            json!({"i": i}),
        );
        audit::append(&pool, entry).await.unwrap();
    }
    assert_eq!(audit::len(&pool).await.unwrap(), 5);
    assert!(audit::verify_chain(&pool).await.unwrap());
}

#[tokio::test]
async fn tampering_breaks_chain() {
    let (pool, _tmp) = fresh_pool().await;
    audit::append(
        &pool,
        AuditEntry::now(
            Some("!r:t".to_string()),
            "@bot:test".to_string(),
            "first".to_string(),
            None,
            None,
            json!({}),
        ),
    )
    .await
    .unwrap();
    audit::append(
        &pool,
        AuditEntry::now(
            Some("!r:t".to_string()),
            "@bot:test".to_string(),
            "second".to_string(),
            None,
            None,
            json!({}),
        ),
    )
    .await
    .unwrap();

    assert!(audit::verify_chain(&pool).await.unwrap());

    // Tamper: change action of row 1.
    sqlx::query("UPDATE moderation_audit_log SET action = 'tampered' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();

    assert!(!audit::verify_chain(&pool).await.unwrap());
}
