// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Configuration for the FAQ bot.

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use url::Url;

/// Top-level bot configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Matrix client settings.
    pub matrix: MatrixConfig,
    /// FAQ source file settings.
    pub faqs: FaqsConfig,
    /// Logging settings.
    pub log: LogConfig,
}

/// Matrix client settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixConfig {
    /// Base URL of the homeserver, e.g. `https://matrix.imogo.de`.
    pub homeserver_url: Url,
    /// Fully qualified Matrix user id of the bot account.
    pub user_id: String,
    /// AS-issued access token for the bot account.
    pub access_token: String,
    /// Display name to set on the bot's profile.
    pub display_name: String,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            homeserver_url: "https://matrix.imogo.de".parse().expect("default url"),
            user_id: "@bot-faq:imogo.de".to_string(),
            access_token: String::new(),
            display_name: "imogo FAQ-Bot".to_string(),
        }
    }
}

/// FAQ source file settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct FaqsConfig {
    /// Filesystem path to the YAML data file (relative to cwd).
    pub path: String,
    /// If true, watch the file and reload on change without a restart.
    pub watch: bool,
}

impl Default for FaqsConfig {
    fn default() -> Self {
        Self {
            path: "data/faqs.yaml".to_string(),
            watch: true,
        }
    }
}

/// Logging settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// `tracing-subscriber` `env_filter` syntax.
    pub filter: String,
    /// If true, emit logs as JSON.
    pub json: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: "faq_bot=info,matrix_sdk=warn".to_string(),
            json: false,
        }
    }
}

impl Config {
    /// Load from defaults, optional `faq-bot.toml`, and env vars prefixed
    /// `IMOGO_FAQ_BOT_`. The figment error is boxed so the returned
    /// `Result` stays small (clippy `result_large_err`).
    ///
    /// # Errors
    ///
    /// Returns the underlying figment error if a source is malformed.
    pub fn load() -> Result<Self, Box<figment::Error>> {
        Figment::new()
            .merge(Serialized::defaults(Self::default()))
            .merge(Toml::file("faq-bot.toml"))
            .merge(Env::prefixed("IMOGO_FAQ_BOT_").split("__"))
            .extract()
            .map_err(Box::new)
    }
}
