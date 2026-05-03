// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Matrix client construction and AS login.
//!
//! Pull-AS architecture:
//! 1. Build a `matrix_sdk::Client` for the homeserver URL.
//! 2. Manually `POST /_matrix/client/v3/login` with
//!    `m.login.application_service` and the AS bearer token. Tuwunel returns
//!    `access_token`, `user_id`, `device_id`.
//! 3. Build a [`MatrixSession`] from the response and call
//!    `restore_session(...)`.
//! 4. Caller proceeds to `sync_once` -> handler -> `sync` long-poll.

use matrix_sdk::{
    Client, SessionMeta, SessionTokens, authentication::matrix::MatrixSession,
    store::RoomLoadSettings,
};
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use crate::{config::MatrixConfig, error::ModError};

#[derive(Debug, Deserialize)]
struct LoginResponse {
    access_token: String,
    user_id: String,
    device_id: String,
}

/// Build a Matrix client and complete the AS login flow.
///
/// # Errors
///
/// Returns [`ModError::Matrix`] for matrix-sdk failures, [`ModError::Http`]
/// for transport failures, and [`ModError::LoginResponse`] for unexpected
/// response shapes.
pub async fn build_and_login(cfg: &MatrixConfig) -> Result<Client, ModError> {
    let client = Client::builder()
        .homeserver_url(cfg.homeserver_url.as_str())
        .build()
        .await
        .map_err(|e| ModError::Matrix(e.to_string()))?;

    let login_url = format!(
        "{}/_matrix/client/v3/login",
        cfg.homeserver_url.as_str().trim_end_matches('/')
    );

    let localpart = cfg
        .user_id
        .strip_prefix('@')
        .and_then(|s| s.split_once(':').map(|(lp, _)| lp))
        .unwrap_or(cfg.user_id.as_str());

    let body = json!({
        "type": "m.login.application_service",
        "identifier": {
            "type": "m.id.user",
            "user": localpart,
        },
        "device_id": cfg.device_id,
    });

    let resp = reqwest::Client::new()
        .post(&login_url)
        .bearer_auth(&cfg.as_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| ModError::Http(e.to_string()))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| ModError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(ModError::LoginResponse(format!(
            "login failed: status {} body {text}",
            status.as_u16()
        )));
    }

    let parsed: LoginResponse =
        serde_json::from_str(&text).map_err(|e| ModError::LoginResponse(e.to_string()))?;

    info!(
        user_id = parsed.user_id.as_str(),
        device_id = parsed.device_id.as_str(),
        "AS login succeeded"
    );

    let user_id = matrix_sdk::ruma::OwnedUserId::try_from(parsed.user_id.clone())
        .map_err(|e| ModError::LoginResponse(format!("invalid user_id in response: {e}")))?;
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(parsed.device_id);

    let session = MatrixSession {
        meta: SessionMeta { user_id, device_id },
        tokens: SessionTokens {
            access_token: parsed.access_token,
            refresh_token: None,
        },
    };

    client
        .matrix_auth()
        .restore_session(session, RoomLoadSettings::default())
        .await
        .map_err(|e| ModError::Matrix(e.to_string()))?;

    Ok(client)
}
