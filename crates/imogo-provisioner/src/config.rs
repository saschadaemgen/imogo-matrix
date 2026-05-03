// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Configuration loading for the provisioner.
//!
//! Loads from `provisioner.toml` in the current working directory if present,
//! then overlays environment variables prefixed with `IMOGO_PROVISIONER_`.
//! Environment variables use double underscores as separators, so
//! `IMOGO_PROVISIONER_HTTP__LISTEN` overrides `http.listen`.

use std::{collections::BTreeMap, net::SocketAddr};

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::Error;

/// Top-level configuration for the provisioner.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// HTTP server settings.
    pub http: HttpConfig,
    /// Logging settings.
    pub log: LogConfig,
    /// Matrix homeserver connections.
    #[serde(default)]
    pub matrix: MatrixConfig,
    /// Inbound webhook configuration (license server calls).
    #[serde(default)]
    pub webhook: WebhookConfig,
    /// Database configuration (audit log and nonce cache).
    #[serde(default)]
    pub db: DbConfig,
    /// Provisioning policy: which homeserver receives B2B accounts, who is
    /// invited to support rooms, what tiers are allowed.
    #[serde(default)]
    pub provisioning: ProvisioningConfig,
}

/// HTTP server settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// Address to listen on, e.g. `0.0.0.0:8080`.
    pub listen: SocketAddr,
    /// Request timeout in seconds.
    pub request_timeout_secs: u64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".parse().expect("valid default listen addr"),
            request_timeout_secs: 30,
        }
    }
}

/// Logging settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Log level filter, follows the `tracing-subscriber` `env_filter` syntax.
    pub filter: String,
    /// If true, emit logs as JSON. If false, human-readable text.
    pub json: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: "imogo_provisioner=info,tower_http=info".to_string(),
            json: false,
        }
    }
}

/// Matrix configuration: a map of logical homeserver names to their
/// connection settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MatrixConfig {
    /// Connections to homeservers, keyed by logical name (e.g. `b2b`, `b2c`).
    #[serde(default)]
    pub homeservers: BTreeMap<String, HomeserverConfig>,
}

/// One Matrix homeserver connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeserverConfig {
    /// Base URL of the homeserver, e.g. `https://matrix.imogo.de`.
    pub url: Url,
    /// Server name as in `server_name` of the homeserver, e.g. `imogo.de`.
    pub server_name: String,
    /// Application service ID matching the registration `id` field.
    pub appservice_id: String,
    /// AS token: the provisioner sends this to the homeserver.
    pub as_token: String,
    /// HS token: the homeserver sends this to the provisioner.
    pub hs_token: String,
    /// The localpart used by the AS bot itself, e.g. `imogo-provisioner`.
    pub sender_localpart: String,
}

/// Webhook configuration (incoming calls from the imogo license server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Maximum allowed clock skew in seconds for inbound webhook timestamps.
    pub max_timestamp_skew_secs: i64,
    /// Legacy field kept for schema compatibility. The persistent nonce store
    /// replaced the in-memory LRU; this field is read from config but no
    /// longer drives any behaviour.
    pub nonce_cache_capacity: usize,
    /// How long a nonce stays in the persistent cache before garbage collection.
    pub nonce_ttl_secs: i64,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            max_timestamp_skew_secs: 300,
            nonce_cache_capacity: 10_000,
            nonce_ttl_secs: 600,
        }
    }
}

/// Database configuration. `SQLite` is used for audit log and nonce cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConfig {
    /// Filesystem path to the `SQLite` database. Created if missing.
    pub path: String,
    /// Maximum number of concurrent connections in the pool.
    pub max_connections: u32,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            path: "./imogo-provisioner.db".to_string(),
            max_connections: 5,
        }
    }
}

/// Provisioning policy. Which homeserver receives new B2B accounts, who is
/// invited to support rooms, what tiers are allowed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisioningConfig {
    /// Logical homeserver name where new B2B accounts are created. Must
    /// match a key in [`MatrixConfig::homeservers`].
    pub b2b_homeserver: String,
    /// Matrix user IDs (fully qualified, e.g. `@support-team:imogo.de`) to
    /// invite into every newly created support room with power level 100.
    #[serde(default)]
    pub support_invitees: Vec<String>,
    /// Allowed tier strings. Webhook payloads with other tiers are rejected.
    #[serde(default = "default_tiers")]
    pub allowed_tiers: Vec<String>,
}

fn default_tiers() -> Vec<String> {
    vec![
        "solo".to_string(),
        "kmu".to_string(),
        "pro".to_string(),
        "enterprise".to_string(),
    ]
}

impl Default for ProvisioningConfig {
    fn default() -> Self {
        Self {
            b2b_homeserver: "b2b".to_string(),
            support_invitees: Vec::new(),
            allowed_tiers: default_tiers(),
        }
    }
}

impl Config {
    /// Load configuration from defaults, optional `provisioner.toml`, and
    /// environment variables.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if a configuration source is malformed.
    pub fn load() -> Result<Self, Error> {
        let figment = Figment::new()
            .merge(Serialized::defaults(Self::default()))
            .merge(Toml::file("provisioner.toml"))
            .merge(Env::prefixed("IMOGO_PROVISIONER_").split("__"));

        figment.extract().map_err(Error::from)
    }
}
