// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for license lifecycle events.

use std::collections::BTreeMap;

use imogo_provisioner::{
    accounts::{AccountState, AccountsRepo},
    audit::AuditLog,
    config::{DbConfig, HomeserverConfig, ProvisioningConfig},
    db,
    matrix::MatrixRegistry,
    provisioning::{
        CustomerInfo, LicenseActivatedPayload, LicenseDeactivatedPayload, LicenseExpiredPayload,
        LicenseTierChangedPayload, ProvisioningError, ProvisioningService,
    },
};
use serde_json::json;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, path_regex},
};

async fn start_full_mock_homeserver() -> MockServer {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versions": ["v1.13"]})))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/register"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"user_id": "@user:test.local"})),
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
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"room_id": "!room:test.local"})),
        )
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/v3/rooms/.+/state/m\.room\.power_levels$",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"event_id": "$evt:test.local"})),
        )
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path_regex(
            r"^/_matrix/client/v3/rooms/.+/state/m\.room\.topic$",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"event_id": "$evt:test.local"})),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path_regex(r"^/_synapse/admin/v1/deactivate/.+$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id_server_unbind_result": "no-op"})),
        )
        .mount(&server)
        .await;

    server
}

async fn fresh_service() -> (
    ProvisioningService,
    AuditLog,
    AccountsRepo,
    MockServer,
    tempfile::TempDir,
) {
    let mock = start_full_mock_homeserver().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert(
        "b2b".to_string(),
        HomeserverConfig {
            url,
            server_name: "test.local".to_string(),
            appservice_id: "imogo-provisioner".to_string(),
            as_token: "test_as".to_string(),
            hs_token: "test_hs".to_string(),
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
        support_invitees: vec!["@support:test.local".to_string()],
        allowed_tiers: vec!["solo".into(), "pro".into()],
    };
    let service = ProvisioningService::new(
        accounts.clone(),
        audit.clone(),
        registry,
        cfg,
        reqwest::Client::new(),
    );

    (service, audit, accounts, mock, tmp)
}

fn act_payload(license_id: &str, tier: &str) -> LicenseActivatedPayload {
    LicenseActivatedPayload {
        event_type: "license.activated".to_string(),
        license_id: license_id.to_string(),
        tier: tier.to_string(),
        customer: CustomerInfo {
            name: "Test Customer".to_string(),
            company: None,
            email: None,
        },
    }
}

#[tokio::test]
async fn license_expired_transitions_to_read_only() {
    let (service, audit, accounts, _mock, _tmp) = fresh_service().await;

    service
        .handle_license_activated(act_payload("lic-1", "pro"))
        .await
        .unwrap();

    let outcome = service
        .handle_license_expired(LicenseExpiredPayload {
            event_type: "license.expired".to_string(),
            license_id: "lic-1".to_string(),
        })
        .await
        .unwrap();

    assert!(!outcome.already_in_target_state);
    assert_eq!(outcome.new_state, "read_only");

    let stored = accounts.find_by_license("lic-1").await.unwrap().unwrap();
    assert_eq!(stored.state, AccountState::ReadOnly);
    assert!(stored.expired_at.is_some());

    audit.verify_chain().await.unwrap();
}

#[tokio::test]
async fn license_expired_idempotent() {
    let (service, _audit, _accounts, _mock, _tmp) = fresh_service().await;

    service
        .handle_license_activated(act_payload("lic-2", "pro"))
        .await
        .unwrap();
    service
        .handle_license_expired(LicenseExpiredPayload {
            event_type: "license.expired".to_string(),
            license_id: "lic-2".to_string(),
        })
        .await
        .unwrap();

    let second = service
        .handle_license_expired(LicenseExpiredPayload {
            event_type: "license.expired".to_string(),
            license_id: "lic-2".to_string(),
        })
        .await
        .unwrap();

    assert!(second.already_in_target_state);
    assert_eq!(second.new_state, "read_only");
}

#[tokio::test]
async fn license_expired_for_unknown_license_returns_error() {
    let (service, _audit, _accounts, _mock, _tmp) = fresh_service().await;

    let result = service
        .handle_license_expired(LicenseExpiredPayload {
            event_type: "license.expired".to_string(),
            license_id: "nonexistent".to_string(),
        })
        .await;

    assert!(matches!(result, Err(ProvisioningError::AccountNotFound(_))));
}

#[tokio::test]
async fn license_deactivated_from_active_works() {
    let (service, _audit, accounts, _mock, _tmp) = fresh_service().await;

    service
        .handle_license_activated(act_payload("lic-3", "pro"))
        .await
        .unwrap();

    let outcome = service
        .handle_license_deactivated(LicenseDeactivatedPayload {
            event_type: "license.deactivated".to_string(),
            license_id: "lic-3".to_string(),
        })
        .await
        .unwrap();

    assert!(!outcome.already_in_target_state);
    assert_eq!(outcome.new_state, "deactivated");

    let stored = accounts.find_by_license("lic-3").await.unwrap().unwrap();
    assert_eq!(stored.state, AccountState::Deactivated);
    assert!(stored.deactivated_at.is_some());
}

#[tokio::test]
async fn license_deactivated_after_expired_works() {
    let (service, _audit, accounts, _mock, _tmp) = fresh_service().await;

    service
        .handle_license_activated(act_payload("lic-4", "pro"))
        .await
        .unwrap();
    service
        .handle_license_expired(LicenseExpiredPayload {
            event_type: "license.expired".to_string(),
            license_id: "lic-4".to_string(),
        })
        .await
        .unwrap();
    service
        .handle_license_deactivated(LicenseDeactivatedPayload {
            event_type: "license.deactivated".to_string(),
            license_id: "lic-4".to_string(),
        })
        .await
        .unwrap();

    let stored = accounts.find_by_license("lic-4").await.unwrap().unwrap();
    assert_eq!(stored.state, AccountState::Deactivated);
}

#[tokio::test]
async fn tier_changed_updates_tier() {
    let (service, _audit, accounts, _mock, _tmp) = fresh_service().await;

    service
        .handle_license_activated(act_payload("lic-5", "solo"))
        .await
        .unwrap();

    let outcome = service
        .handle_license_tier_changed(LicenseTierChangedPayload {
            event_type: "license.tier_changed".to_string(),
            license_id: "lic-5".to_string(),
            new_tier: "pro".to_string(),
        })
        .await
        .unwrap();

    assert!(!outcome.already_in_target_state);
    assert_eq!(outcome.account.tier, "pro");

    let stored = accounts.find_by_license("lic-5").await.unwrap().unwrap();
    assert_eq!(stored.tier, "pro");
}

#[tokio::test]
async fn tier_changed_idempotent_when_same() {
    let (service, _audit, _accounts, _mock, _tmp) = fresh_service().await;

    service
        .handle_license_activated(act_payload("lic-6", "pro"))
        .await
        .unwrap();

    let outcome = service
        .handle_license_tier_changed(LicenseTierChangedPayload {
            event_type: "license.tier_changed".to_string(),
            license_id: "lic-6".to_string(),
            new_tier: "pro".to_string(),
        })
        .await
        .unwrap();

    assert!(outcome.already_in_target_state);
}

#[tokio::test]
async fn full_lifecycle_audit_chain_is_intact() {
    let (service, audit, _accounts, _mock, _tmp) = fresh_service().await;

    service
        .handle_license_activated(act_payload("lic-7", "solo"))
        .await
        .unwrap();
    service
        .handle_license_tier_changed(LicenseTierChangedPayload {
            event_type: "license.tier_changed".to_string(),
            license_id: "lic-7".to_string(),
            new_tier: "pro".to_string(),
        })
        .await
        .unwrap();
    service
        .handle_license_expired(LicenseExpiredPayload {
            event_type: "license.expired".to_string(),
            license_id: "lic-7".to_string(),
        })
        .await
        .unwrap();
    service
        .handle_license_deactivated(LicenseDeactivatedPayload {
            event_type: "license.deactivated".to_string(),
            license_id: "lic-7".to_string(),
        })
        .await
        .unwrap();

    audit.verify_chain().await.unwrap();
    let n = audit.len().await.unwrap();
    assert!(
        n >= 8,
        "expected at least 8 audit entries from full lifecycle, got {n}"
    );
}
