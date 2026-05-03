// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Error types for the moderation bot.

use thiserror::Error;

/// Top-level error type returned by bot library functions.
///
/// `figment::Error` is large, so we box it. Matrix-SDK and reqwest errors
/// are flattened to `String` because their internal types are not stable
/// enough across versions to expose directly.
#[derive(Debug, Error)]
pub enum ModError {
    /// I/O error from the standard library.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Configuration loading or merging failed.
    #[error("configuration error: {0}")]
    Config(Box<figment::Error>),

    /// Database error.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    /// `SQLx` migration error.
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    /// Matrix SDK error.
    #[error("matrix error: {0}")]
    Matrix(String),

    /// Reqwest / HTTP error.
    #[error("http error: {0}")]
    Http(String),

    /// Login response missing or malformed.
    #[error("login response invalid: {0}")]
    LoginResponse(String),

    /// Command parser error.
    #[error("invalid command: {0}")]
    InvalidCommand(String),

    /// `m.room.power_levels` state event missing.
    #[error("no power_levels state in room")]
    NoPowerLevelsState,

    /// Regex compilation error.
    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),

    /// YAML parser error.
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

impl From<figment::Error> for ModError {
    fn from(e: figment::Error) -> Self {
        Self::Config(Box::new(e))
    }
}
