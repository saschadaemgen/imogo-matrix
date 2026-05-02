// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Matrix homeserver connection management.
//!
//! Holds one [`matrix_sdk::Client`] per configured homeserver and provides
//! lookup by logical name. Authentication uses the application service token
//! (`as_token`) configured per homeserver. Inbound calls from the homeserver
//! are validated by comparing the `hs_token` query parameter using a
//! constant-time comparison.

use std::{collections::BTreeMap, sync::Arc};

use matrix_sdk::{Client, ClientBuildError, config::SyncSettings};
use subtle::ConstantTimeEq;
use tracing::{debug, info, instrument, warn};

use crate::config::HomeserverConfig;

/// Registry of Matrix homeserver connections.
#[derive(Clone, Debug)]
pub struct MatrixRegistry {
    inner: Arc<BTreeMap<String, HomeserverConnection>>,
}

/// One configured homeserver connection plus the associated tokens.
#[derive(Clone, Debug)]
pub struct HomeserverConnection {
    /// Logical name as used in config, e.g. `b2b`.
    pub name: String,
    /// Configuration values used to build this connection.
    pub config: HomeserverConfig,
    /// Active matrix-sdk client.
    pub client: Client,
}

impl MatrixRegistry {
    /// Build the registry from configuration. Each homeserver is validated by
    /// constructing a `matrix-sdk` client. Network calls are deferred until
    /// [`MatrixRegistry::ping_all`] is invoked.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`ClientBuildError`] of the first homeserver
    /// that fails to build.
    pub async fn build(
        homeservers: &BTreeMap<String, HomeserverConfig>,
    ) -> Result<Self, ClientBuildError> {
        let mut map = BTreeMap::new();

        for (name, hs) in homeservers {
            info!(
                name = name.as_str(),
                url = hs.url.as_str(),
                server_name = hs.server_name.as_str(),
                "building matrix client"
            );
            // Note: we deliberately do NOT pre-set `server_versions` on the
            // builder. Pre-setting would short-circuit the SDK's network call
            // and our ping would always succeed even for unreachable hosts.
            let client = Client::builder()
                .homeserver_url(hs.url.as_str())
                .build()
                .await?;
            map.insert(
                name.clone(),
                HomeserverConnection {
                    name: name.clone(),
                    config: hs.clone(),
                    client,
                },
            );
        }

        Ok(Self {
            inner: Arc::new(map),
        })
    }

    /// Iterate over all configured connections.
    pub fn iter(&self) -> impl Iterator<Item = &HomeserverConnection> {
        self.inner.values()
    }

    /// Look up a homeserver connection by logical name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&HomeserverConnection> {
        self.inner.get(name)
    }

    /// Best-effort connectivity check for all configured homeservers.
    /// Returns the names of homeservers that responded successfully.
    pub async fn ping_all(&self) -> Vec<String> {
        let mut healthy = Vec::new();
        for conn in self.inner.values() {
            if conn.ping().await {
                healthy.push(conn.name.clone());
            }
        }
        healthy
    }

    /// True iff every configured homeserver is currently reachable.
    pub async fn all_healthy(&self) -> bool {
        let healthy = self.ping_all().await;
        healthy.len() == self.inner.len()
    }
}

impl HomeserverConnection {
    /// Hit the homeserver with a fresh low-cost call to verify connectivity.
    ///
    /// Uses a direct `reqwest` GET to `/_matrix/client/versions` rather than
    /// `matrix_sdk::Client::server_versions`, because the SDK caches the
    /// versions response on the first call (or skips the network entirely if
    /// `ClientBuilder::server_versions` was set), which would mask a
    /// homeserver going offline after process start.
    #[instrument(skip(self), fields(name = self.name.as_str()))]
    pub async fn ping(&self) -> bool {
        let versions_url = match self.config.url.join("/_matrix/client/versions") {
            Ok(u) => u,
            Err(e) => {
                warn!(error = %e, "could not build versions URL");
                return false;
            }
        };
        match reqwest::get(versions_url).await {
            Ok(resp) if resp.status().is_success() => {
                debug!(status = %resp.status(), "homeserver reachable");
                true
            }
            Ok(resp) => {
                warn!(status = %resp.status(), "homeserver versions endpoint returned non-2xx");
                false
            }
            Err(e) => {
                warn!(error = %e, "homeserver ping failed");
                false
            }
        }
    }

    /// Verify an inbound `hs_token` using a constant-time comparison.
    #[must_use]
    pub fn verify_hs_token(&self, presented: &str) -> bool {
        let expected = self.config.hs_token.as_bytes();
        let presented = presented.as_bytes();
        if expected.len() != presented.len() {
            return false;
        }
        expected.ct_eq(presented).into()
    }
}

/// Stub: how a real sync loop would be wired up. Not used in 02b, kept here
/// as a marker for 02c when we actually pull events.
#[allow(dead_code)]
async fn _placeholder_sync(client: &Client) {
    let _ = client.sync(SyncSettings::default()).await;
}
