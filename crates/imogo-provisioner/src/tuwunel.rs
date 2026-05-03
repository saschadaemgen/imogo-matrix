// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Direct HTTP client for the Tuwunel Client-Server API, used in
//! application-service mode (authenticating with the AS token).
//!
//! We do not rely on the high-level matrix-sdk for these calls because the
//! AS-specific operations (create user, send-as-user) are not part of the
//! standard client SDK. Calls are simple enough to issue directly via reqwest.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tracing::{debug, instrument};

/// Errors raised by [`TuwunelClient`] calls.
#[derive(Debug, Error)]
#[allow(clippy::module_name_repetitions)]
pub enum TuwunelError {
    /// Transport-level reqwest failure (connection refused, TLS, etc.).
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// Homeserver returned a non-2xx response.
    #[error("matrix api error: status {status}, body: {body}")]
    Api {
        /// HTTP status code returned by the homeserver.
        status: u16,
        /// Response body, truncated by reqwest's default.
        body: String,
    },
}

/// Result of a successful account registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredAccount {
    /// Fully qualified user id assigned by the homeserver.
    pub user_id: String,
}

/// Result of a successful room creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedRoom {
    /// Matrix room id (`!xxx:server`).
    pub room_id: String,
    /// Optional canonical alias if one was set.
    pub room_alias: Option<String>,
}

/// Power-level setting for a Matrix room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerLevels {
    /// Per-user power level overrides.
    pub users: std::collections::BTreeMap<String, i64>,
    /// Default power level for users not in `users`.
    pub users_default: i64,
    /// Default power level required to send a message event.
    pub events_default: i64,
    /// Default power level required to send a state event.
    pub state_default: i64,
    /// Power level required to invite a user.
    pub invite: i64,
    /// Power level required to kick a user.
    pub kick: i64,
    /// Power level required to ban a user.
    pub ban: i64,
    /// Power level required to redact events.
    pub redact: i64,
}

/// Lightweight Tuwunel HTTP client, scoped to a single homeserver and AS token.
#[derive(Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct TuwunelClient {
    http: Client,
    homeserver_url: String,
    as_token: String,
}

impl std::fmt::Debug for TuwunelClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TuwunelClient")
            .field("homeserver_url", &self.homeserver_url)
            .finish_non_exhaustive()
    }
}

impl TuwunelClient {
    /// Construct the client. `homeserver_url` should not have a trailing slash.
    #[must_use]
    pub fn new(http: Client, homeserver_url: String, as_token: String) -> Self {
        Self {
            http,
            homeserver_url,
            as_token,
        }
    }

    /// Register a new user with a given localpart and initial password. Uses
    /// the `m.login.application_service` login type.
    ///
    /// # Errors
    ///
    /// Returns [`TuwunelError::Api`] if the homeserver rejects the
    /// registration, or [`TuwunelError::Http`] for transport-level failures.
    #[instrument(skip(self, password), fields(localpart = %localpart))]
    pub async fn register_user(
        &self,
        localpart: &str,
        password: &str,
    ) -> Result<RegisteredAccount, TuwunelError> {
        let url = format!(
            "{}/_matrix/client/v3/register?kind=user",
            self.homeserver_url
        );
        let body = json!({
            "type": "m.login.application_service",
            "username": localpart,
            "password": password,
            "inhibit_login": true,
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.as_token)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(TuwunelError::Api {
                status: status.as_u16(),
                body: text,
            });
        }

        let parsed: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| TuwunelError::Api {
                status: status.as_u16(),
                body: format!("invalid response json: {e}"),
            })?;
        let user_id = parsed
            .get("user_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TuwunelError::Api {
                status: status.as_u16(),
                body: "missing user_id in response".to_string(),
            })?;

        debug!(user_id, "user registered");
        Ok(RegisteredAccount {
            user_id: user_id.to_string(),
        })
    }

    /// Set the display name of the given user (acting as that user via
    /// `?user_id=...`).
    ///
    /// # Errors
    ///
    /// Returns [`TuwunelError::Api`] if the homeserver rejects the call.
    #[instrument(skip(self), fields(user_id = %user_id))]
    pub async fn set_display_name(
        &self,
        user_id: &str,
        display_name: &str,
    ) -> Result<(), TuwunelError> {
        let path = format!("/_matrix/client/v3/profile/{user_id}/displayname");
        let url = format!("{}{path}?user_id={user_id}", self.homeserver_url);
        let body = json!({ "displayname": display_name });

        let resp = self
            .http
            .put(&url)
            .bearer_auth(&self.as_token)
            .json(&body)
            .send()
            .await?;

        check_ok(resp).await
    }

    /// Create a private room with an alias and initial power levels.
    ///
    /// # Errors
    ///
    /// Returns [`TuwunelError::Api`] if the homeserver rejects the call.
    #[instrument(skip(self, power_levels))]
    pub async fn create_room(
        &self,
        room_alias_localpart: &str,
        topic: &str,
        invite: &[String],
        power_levels: &PowerLevels,
    ) -> Result<CreatedRoom, TuwunelError> {
        let url = format!("{}/_matrix/client/v3/createRoom", self.homeserver_url);
        let body = json!({
            "preset": "private_chat",
            "visibility": "private",
            "room_alias_name": room_alias_localpart,
            "topic": topic,
            "invite": invite,
            "power_level_content_override": power_levels,
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.as_token)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(TuwunelError::Api {
                status: status.as_u16(),
                body: text,
            });
        }

        let parsed: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| TuwunelError::Api {
                status: status.as_u16(),
                body: format!("invalid response json: {e}"),
            })?;
        let room_id = parsed
            .get("room_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TuwunelError::Api {
                status: status.as_u16(),
                body: "missing room_id in response".to_string(),
            })?;
        let room_alias = parsed
            .get("room_alias")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        debug!(room_id, "room created");
        Ok(CreatedRoom {
            room_id: room_id.to_string(),
            room_alias,
        })
    }
}

async fn check_ok(resp: reqwest::Response) -> Result<(), TuwunelError> {
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = resp.text().await.unwrap_or_default();
        Err(TuwunelError::Api {
            status: status.as_u16(),
            body,
        })
    }
}
