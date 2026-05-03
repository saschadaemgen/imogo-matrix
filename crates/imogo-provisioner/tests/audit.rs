// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for the audit log hash-chain.

use imogo_provisioner::{
    audit::{AuditLog, NewAuditEntry},
    config::DbConfig,
    db,
};
use sqlx::SqlitePool;

async fn fresh_pool() -> (SqlitePool, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tmp");
    let cfg = DbConfig {
        path: tmp.path().join("audit.db").to_string_lossy().into_owned(),
        max_connections: 2,
    };
    let pool = db::open_pool(&cfg).await.expect("open");
    (pool, tmp)
}

#[tokio::test]
async fn appending_entries_chains_correctly() {
    let (pool, _tmp) = fresh_pool().await;
    let log = AuditLog::new(pool);

    let e1 = log
        .append(NewAuditEntry {
            event_type: "test.first".to_string(),
            actor: "test".to_string(),
            subject: Some("subj-1".to_string()),
            payload_json: r#"{"x":1}"#.to_string(),
        })
        .await
        .expect("append 1");

    assert_eq!(e1.id, 1);
    assert_eq!(e1.prev_hash, AuditLog::genesis_hash());

    let e2 = log
        .append(NewAuditEntry {
            event_type: "test.second".to_string(),
            actor: "test".to_string(),
            subject: None,
            payload_json: r#"{"x":2}"#.to_string(),
        })
        .await
        .expect("append 2");

    assert_eq!(e2.id, 2);
    assert_eq!(e2.prev_hash, e1.entry_hash);
    assert_ne!(e2.entry_hash, e1.entry_hash);

    log.verify_chain().await.expect("chain verifies");
}

#[tokio::test]
async fn empty_chain_verifies() {
    let (pool, _tmp) = fresh_pool().await;
    let log = AuditLog::new(pool);
    log.verify_chain().await.expect("empty chain ok");
    assert_eq!(log.len().await.unwrap(), 0);
}

#[tokio::test]
async fn tampered_payload_breaks_chain() {
    let (pool, _tmp) = fresh_pool().await;
    let log = AuditLog::new(pool.clone());

    log.append(NewAuditEntry {
        event_type: "test".to_string(),
        actor: "a".to_string(),
        subject: None,
        payload_json: "original".to_string(),
    })
    .await
    .expect("append");

    sqlx::query("UPDATE audit_log SET payload_json = ? WHERE id = 1")
        .bind("tampered")
        .execute(&pool)
        .await
        .expect("tamper");

    let result = log.verify_chain().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn tampered_prev_hash_breaks_chain() {
    let (pool, _tmp) = fresh_pool().await;
    let log = AuditLog::new(pool.clone());

    log.append(NewAuditEntry {
        event_type: "a".to_string(),
        actor: "x".to_string(),
        subject: None,
        payload_json: "{}".to_string(),
    })
    .await
    .expect("e1");
    log.append(NewAuditEntry {
        event_type: "b".to_string(),
        actor: "x".to_string(),
        subject: None,
        payload_json: "{}".to_string(),
    })
    .await
    .expect("e2");

    sqlx::query("UPDATE audit_log SET prev_hash = ? WHERE id = 2")
        .bind("0".repeat(64))
        .execute(&pool)
        .await
        .expect("tamper");

    let result = log.verify_chain().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn many_entries_chain_well() {
    let (pool, _tmp) = fresh_pool().await;
    let log = AuditLog::new(pool);

    for i in 0..50 {
        log.append(NewAuditEntry {
            event_type: format!("event.{i}"),
            actor: "test".to_string(),
            subject: Some(format!("subj-{i}")),
            payload_json: format!(r#"{{"i":{i}}}"#),
        })
        .await
        .expect("append");
    }

    assert_eq!(log.len().await.unwrap(), 50);
    log.verify_chain().await.expect("verify");
}
