// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for the persistent nonce store.

use imogo_provisioner::{config::DbConfig, db, nonce_store::NonceStore};

async fn fresh_store(ttl_secs: i64) -> (NonceStore, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("tmp");
    let cfg = DbConfig {
        path: tmp.path().join("nonce.db").to_string_lossy().into_owned(),
        max_connections: 2,
    };
    let pool = db::open_pool(&cfg).await.expect("open");
    (NonceStore::new(pool, ttl_secs), tmp)
}

#[tokio::test]
async fn first_insert_returns_true() {
    let (store, _tmp) = fresh_store(600).await;
    assert!(store.try_insert("nonce1", "key1").await.unwrap());
    assert_eq!(store.count().await.unwrap(), 1);
}

#[tokio::test]
async fn second_insert_of_same_nonce_returns_false() {
    let (store, _tmp) = fresh_store(600).await;
    assert!(store.try_insert("nonce-x", "key1").await.unwrap());
    assert!(!store.try_insert("nonce-x", "key1").await.unwrap());
    assert_eq!(store.count().await.unwrap(), 1);
}

#[tokio::test]
async fn different_nonces_coexist() {
    let (store, _tmp) = fresh_store(600).await;
    assert!(store.try_insert("a", "k").await.unwrap());
    assert!(store.try_insert("b", "k").await.unwrap());
    assert!(store.try_insert("c", "k").await.unwrap());
    assert_eq!(store.count().await.unwrap(), 3);
}

#[tokio::test]
async fn expired_nonces_get_collected() {
    let (store, _tmp) = fresh_store(1).await;
    assert!(store.try_insert("short-lived", "k").await.unwrap());
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    // The next try_insert garbage-collects expired entries.
    assert!(store.try_insert("new", "k").await.unwrap());
    assert!(!store.contains("short-lived").await.unwrap());
}
