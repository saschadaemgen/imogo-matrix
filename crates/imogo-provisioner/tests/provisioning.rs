// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration test for the full license-activation flow against a mock
//! Tuwunel homeserver.

use std::collections::BTreeMap;

use imogo_provisioner::{
    accounts::AccountsRepo,
    audit::AuditLog,
    config::{DbConfig, HomeserverConfig, ProvisioningConfig},
    db,
    matrix::MatrixRegistry,
    provisioning::{CustomerInfo, LicenseActivatedPayload, ProvisioningService},
};
use serde_json::json;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, path_regex},
};

async fn start_mock_homeserver() -> MockServer {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "versions": ["v1.13"]
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/register"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"user_id": "@test:test.local", "home_server": "test.local"})),
        )
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path_regex(r"^/_matrix/client/v3/profile/.+/displayname$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/createRoom"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "room_id": "!supportroom:test.local",
            "room_alias": "#support-abcd1234:test.local"
        })))
        .mount(&server)
        .await;

    server
}

#[tokio::test]
async fn license_activation_creates_account() {
    let mock = start_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert(
        "b2b".to_string(),
        HomeserverConfig {
            url,
            server_name: "test.local".to_string(),
            appservice_id: "imogo-provisioner".to_string(),
            as_token: "test_as_token".to_string(),
            hs_token: "test_hs_token".to_string(),
            sender_localpart: "imogo-provisioner".to_string(),
        },
    );
    let registry = MatrixRegistry::build(&homeservers).await.unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let db_cfg = DbConfig {
        path: tmp.path().join("p.db").to_string_lossy().into_owned(),
        max_connections: 2,
    };
    let pool = db::open_pool(&db_cfg).await.unwrap();
    let audit = AuditLog::new(pool.clone());
    let accounts = AccountsRepo::new(pool);

    let cfg = ProvisioningConfig {
        b2b_homeserver: "b2b".to_string(),
        support_invitees: vec!["@support-team:test.local".to_string()],
        allowed_tiers: vec!["pro".to_string()],
    };

    let service = ProvisioningService::new(
        accounts,
        audit.clone(),
        registry,
        cfg,
        reqwest::Client::new(),
    );

    let payload = LicenseActivatedPayload {
        event_type: "license.activated".to_string(),
        license_id: "lic-2026-0042".to_string(),
        tier: "pro".to_string(),
        customer: CustomerInfo {
            name: "Max Mustermann".to_string(),
            company: Some("Mustermann GmbH".to_string()),
            email: None,
        },
    };

    let outcome = service
        .handle_license_activated(payload.clone())
        .await
        .unwrap();
    assert!(!outcome.already_existed);
    assert!(outcome.initial_password.is_some());
    assert_eq!(outcome.account.tier, "pro");
    assert!(outcome.account.matrix_user_id.starts_with('@'));
    assert!(outcome.account.matrix_user_id.ends_with(":test.local"));
    assert_eq!(outcome.account.support_room_id, "!supportroom:test.local");

    // Idempotency: same payload again returns existing without password.
    let outcome2 = service.handle_license_activated(payload).await.unwrap();
    assert!(outcome2.already_existed);
    assert!(outcome2.initial_password.is_none());
    assert_eq!(
        outcome2.account.matrix_user_id,
        outcome.account.matrix_user_id
    );

    // Audit chain still verifies.
    audit.verify_chain().await.unwrap();
}

#[tokio::test]
async fn license_activation_rejects_invalid_tier() {
    let mock = start_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert(
        "b2b".to_string(),
        HomeserverConfig {
            url,
            server_name: "test.local".to_string(),
            appservice_id: "imogo-provisioner".to_string(),
            as_token: "test".to_string(),
            hs_token: "test".to_string(),
            sender_localpart: "imogo-provisioner".to_string(),
        },
    );
    let registry = MatrixRegistry::build(&homeservers).await.unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let db_cfg = DbConfig {
        path: tmp.path().join("p.db").to_string_lossy().into_owned(),
        max_connections: 2,
    };
    let pool = db::open_pool(&db_cfg).await.unwrap();
    let audit = AuditLog::new(pool.clone());
    let accounts = AccountsRepo::new(pool);

    let cfg = ProvisioningConfig {
        b2b_homeserver: "b2b".to_string(),
        support_invitees: vec![],
        allowed_tiers: vec!["pro".to_string()],
    };

    let service = ProvisioningService::new(accounts, audit, registry, cfg, reqwest::Client::new());

    let payload = LicenseActivatedPayload {
        event_type: "license.activated".to_string(),
        license_id: "lic-bad".to_string(),
        tier: "free".to_string(),
        customer: CustomerInfo {
            name: "Test".to_string(),
            company: None,
            email: None,
        },
    };

    let result = service.handle_license_activated(payload).await;
    assert!(result.is_err());
}
