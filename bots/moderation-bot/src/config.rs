// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Configuration loading.
//!
//! Loads from defaults, optional `mod-bot.toml`, and env vars prefixed
//! `IMOGO_MOD_BOT_`. The `as_token` has no sensible default and must be
//! supplied via either source.

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::ModError;

/// Top-level moderation-bot configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Matrix client settings (homeserver URL, AS token, identity).
    pub matrix: MatrixConfig,
    /// Database file location.
    pub database: DatabaseConfig,
    /// Bot policy and Power-Level thresholds.
    pub bot: BotConfig,
    /// Logging settings.
    pub telemetry: TelemetryConfig,
}

/// Matrix client settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixConfig {
    /// Base URL of the homeserver.
    pub homeserver_url: Url,
    /// Fully qualified bot user id, e.g. `@imogo-moderator:imogo.de`.
    pub user_id: String,
    /// Application Service token returned by Tuwunel at registration time.
    pub as_token: String,
    /// Stable device id, kept across restarts.
    pub device_id: String,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            homeserver_url: "https://matrix.imogo.de".parse().expect("default url"),
            user_id: "@imogo-moderator:imogo.de".to_string(),
            as_token: String::new(),
            device_id: "moderation-bot".to_string(),
        }
    }
}

/// Database file location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Filesystem path to the `SQLite` database. Created if missing.
    pub path: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: "data/moderation.db".to_string(),
        }
    }
}

/// Bot policy and Power-Level thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    /// Required Power Level to use the kick command.
    pub pl_kick: i64,
    /// Required Power Level to use the ban/unban command.
    pub pl_ban: i64,
    /// Required Power Level to use mute/unmute.
    pub pl_mute: i64,
    /// Required Power Level to use pin/unpin.
    pub pl_pin: i64,
    /// Required Power Level to manage banned words and (de)activate the
    /// bot in a room.
    pub pl_word_admin: i64,
    /// Auto-discovery: rooms with a canonical alias matching this regex
    /// are inserted into `moderation_active_rooms` on bot startup.
    pub auto_discover_alias_pattern: String,
    /// Maximum mute duration in seconds.
    pub max_mute_seconds: u64,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            pl_kick: 50,
            pl_ban: 50,
            pl_mute: 50,
            pl_pin: 50,
            pl_word_admin: 50,
            auto_discover_alias_pattern: r"^#community.*:imogo\.de$".to_string(),
            max_mute_seconds: 604_800,
        }
    }
}

/// Logging settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// `tracing-subscriber` `env_filter` syntax.
    pub log_level: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
        }
    }
}

impl Config {
    /// Load from defaults, optional `mod-bot.toml`, and env vars prefixed
    /// `IMOGO_MOD_BOT_`.
    ///
    /// # Errors
    ///
    /// Returns [`ModError::Config`] if a configuration source is malformed.
    pub fn load() -> Result<Self, ModError> {
        Figment::new()
            .merge(Serialized::defaults(Self::default()))
            .merge(Toml::file("mod-bot.toml"))
            .merge(Env::prefixed("IMOGO_MOD_BOT_").split("__"))
            .extract()
            .map_err(ModError::from)
    }
}
