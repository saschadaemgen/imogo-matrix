// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for the B2C end-customer flow.

#![cfg(feature = "dev-keys")]

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;
use imogo_provisioner::{
    audit::AuditLog,
    b2c::{B2cService, CreateRoomRequest, RedeemRequest, normalise_invoice_number},
    capability::{CapabilityError, CapabilityVerifier},
    config::{B2cConfig, DbConfig, HomeserverConfig},
    db,
    keys::{CapabilityKeyRegistry, RegisteredKey},
    matrix::MatrixRegistry,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rand::rngs::OsRng;
use serde_json::json;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, path_regex},
};

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

async fn start_full_mock_b2c() -> MockServer {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/_matrix/client/versions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"versions": ["v1.13"]})))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/createRoom"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"room_id": "!b2croom:test.local"})),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/register"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"user_id": "@gast:test.local"})),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/_matrix/client/v3/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "syn_test_token",
            "user_id": "@gast:test.local"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path_regex(r"^/_matrix/client/v3/rooms/.+/invite$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    server
}

#[allow(clippy::type_complexity)]
async fn build_state() -> (
    B2cService,
    CapabilityVerifier,
    SigningKey,
    String,
    AuditLog,
    MockServer,
    tempfile::TempDir,
) {
    let mock = start_full_mock_b2c().await;
    let url = Url::parse(&mock.uri()).unwrap();

    let mut homeservers = BTreeMap::new();
    homeservers.insert(
        "b2c".to_string(),
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

    let signing_key = SigningKey::generate(&mut OsRng);
    let kid = "test-cap-key".to_string();
    let mut keys = CapabilityKeyRegistry::default();
    keys.insert(RegisteredKey {
        key_id: leak(&kid),
        key: signing_key.verifying_key(),
        note: leak("test"),
    });
    let cap_verifier = CapabilityVerifier::new(keys, pool.clone());

    let b2c = B2cService::new(
        pool,
        audit.clone(),
        registry,
        B2cConfig::default(),
        reqwest::Client::new(),
    );

    (b2c, cap_verifier, signing_key, kid, audit, mock, tmp)
}

fn issue_token(signing_key: &SigningKey, kid: &str, sub: &str, caps: &[&str]) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": "imogo-license-server",
        "sub": sub,
        "matrix_user_id": format!("@b2b-{sub}:imogo.de"),
        "caps": caps,
        "iat": now,
        "exp": now + 900,
        "jti": uuid::Uuid::new_v4().to_string(),
    });
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(kid.to_string());
    let der = ed25519_to_pkcs8_der(&signing_key.to_bytes());
    let encoding_key = EncodingKey::from_ed_der(&der);
    encode(&header, &claims, &encoding_key).unwrap()
}

/// Build a PKCS#8 v2 DER-encoded private key wrapper for an Ed25519 seed.
/// jsonwebtoken's `EncodingKey::from_ed_der` expects this format.
fn ed25519_to_pkcs8_der(seed: &[u8; 32]) -> Vec<u8> {
    // PKCS8 v2 (OneAsymmetricKey) wrapper for Ed25519:
    //   SEQUENCE 0x30 length 0x53
    //     INTEGER 0x02 0x01 0x01            (version = 1, PKCS#8 v2)
    //     SEQUENCE 0x30 0x05
    //       OID 0x06 0x03 0x2B 0x65 0x70    (Ed25519)
    //     OCTET STRING 0x04 0x22
    //       OCTET STRING 0x04 0x20 [32-byte seed]
    //     [1] OPTIONAL public key
    //       0xA1 0x23
    //         BIT STRING 0x03 0x21 0x00 [32-byte public]
    let mut der = vec![
        0x30, 0x53, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend_from_slice(seed);
    let signing = SigningKey::from_bytes(seed);
    let pk = signing.verifying_key().to_bytes();
    der.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0x00]);
    der.extend_from_slice(&pk);
    der
}

#[tokio::test]
async fn create_room_then_redeem_works() {
    let (b2c, cap, sk, kid, audit, _mock, _tmp) = build_state().await;

    let token = issue_token(&sk, &kid, "lic-1", &["b2c.create_room"]);
    let claims = cap
        .verify(Some(&format!("Bearer {token}")), "b2c.create_room")
        .await
        .unwrap();
    assert_eq!(claims.sub, "lic-1");

    let resp = b2c
        .create_room(
            &claims.sub,
            &claims.matrix_user_id,
            CreateRoomRequest {
                invoice_number: "2026-0042".to_string(),
                invoice_subject: "Heizungsreparatur".to_string(),
                topic: "Fragen".to_string(),
                qr_token_ttl_days: Some(7),
            },
        )
        .await
        .unwrap();
    assert!(resp.qr_url.contains("2026-0042"));
    assert!(resp.qr_url.contains(&resp.qr_token));

    let redeemed = b2c
        .redeem(RedeemRequest {
            qr_token: resp.qr_token,
        })
        .await
        .unwrap();
    assert_eq!(redeemed.matrix_login_token, "syn_test_token");
    assert_eq!(redeemed.room_id, resp.room_id);

    audit.verify_chain().await.unwrap();
}

#[tokio::test]
async fn token_replay_is_rejected() {
    let (_b2c, cap, sk, kid, _audit, _mock, _tmp) = build_state().await;
    let token = issue_token(&sk, &kid, "lic-r", &["b2c.create_room"]);

    let _ = cap
        .verify(Some(&format!("Bearer {token}")), "b2c.create_room")
        .await
        .unwrap();
    let second = cap
        .verify(Some(&format!("Bearer {token}")), "b2c.create_room")
        .await;
    assert!(matches!(second, Err(CapabilityError::Replay)));
}

#[tokio::test]
async fn missing_capability_is_rejected() {
    let (_b2c, cap, sk, kid, _audit, _mock, _tmp) = build_state().await;
    let token = issue_token(&sk, &kid, "lic-m", &["other.cap"]);

    let result = cap
        .verify(Some(&format!("Bearer {token}")), "b2c.create_room")
        .await;
    assert!(matches!(result, Err(CapabilityError::MissingCapability(_))));
}

#[tokio::test]
async fn invalid_qr_token_returns_not_found() {
    let (b2c, _cap, _sk, _kid, _audit, _mock, _tmp) = build_state().await;
    let result = b2c
        .redeem(RedeemRequest {
            qr_token: "definitely-not-a-real-token".to_string(),
        })
        .await;
    assert!(result.is_err());
}

#[test]
fn invoice_number_normalisation() {
    assert_eq!(normalise_invoice_number("2026-0042"), "2026-0042");
    assert_eq!(normalise_invoice_number("2026/0042"), "2026-0042");
    assert_eq!(normalise_invoice_number("RE 2026 04 2"), "re-2026-04-2");
    assert_eq!(normalise_invoice_number("------"), "");
}
