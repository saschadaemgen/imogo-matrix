// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Configuration loading for the provisioner.
//!
//! Loads from `provisioner.toml` in the current working directory if present,
//! then overlays environment variables prefixed with `IMOGO_PROVISIONER_`.
//! Environment variables use double underscores as separators, so
//! `IMOGO_PROVISIONER_HTTP__LISTEN` overrides `http.listen`.

use std::net::SocketAddr;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Top-level configuration for the provisioner.
///
/// `Default` delegates to [`HttpConfig::default`] and [`LogConfig::default`],
/// which in turn carry the real built-in values (listen address, timeout,
/// log filter).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// HTTP server settings.
    pub http: HttpConfig,
    /// Logging settings.
    pub log: LogConfig,
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
    /// Examples: `info`, `debug`, `imogo_provisioner=debug,tower_http=info`.
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

impl Config {
    /// Load configuration from defaults, optional `provisioner.toml`, and
    /// environment variables.
    ///
    /// Resolution order, last wins:
    /// 1. Built-in [`Config::default`] values.
    /// 2. `provisioner.toml` in the current working directory, if present.
    /// 3. Environment variables prefixed `IMOGO_PROVISIONER_`, double
    ///    underscore as nested-key separator.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] if a configuration source is malformed
    /// (invalid TOML, unparseable values, type mismatches).
    pub fn load() -> Result<Self, Error> {
        let figment = Figment::new()
            .merge(Serialized::defaults(Self::default()))
            .merge(Toml::file("provisioner.toml"))
            .merge(Env::prefixed("IMOGO_PROVISIONER_").split("__"));

        figment.extract().map_err(Error::from)
    }
}
