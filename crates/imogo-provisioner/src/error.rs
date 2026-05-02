// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Provisioner error types.

use thiserror::Error;

/// Top-level error type returned by library functions.
///
/// `figment::Error` is comparatively large, so it is boxed here to keep
/// the enum compact. A manual `From<figment::Error>` impl auto-boxes.
#[derive(Debug, Error)]
pub enum Error {
    /// Configuration loading failed.
    #[error("configuration error: {0}")]
    Config(Box<figment::Error>),

    /// Logging initialisation failed.
    #[error("telemetry error: {0}")]
    Telemetry(String),

    /// I/O error from the standard library.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<figment::Error> for Error {
    fn from(e: figment::Error) -> Self {
        Self::Config(Box::new(e))
    }
}
