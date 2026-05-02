// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Tracing initialisation.
//!
//! Reads the configured filter and output format and installs a global
//! subscriber. Idempotent within the same process: a second call is a no-op.

use std::sync::OnceLock;

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::{config::LogConfig, error::Error};

static INIT: OnceLock<()> = OnceLock::new();

/// Initialise the global tracing subscriber from the supplied log config.
///
/// Calling this function more than once in the same process is safe: the
/// first call wins, subsequent calls return `Ok(())` without changing the
/// subscriber.
///
/// # Errors
///
/// Returns [`Error::Telemetry`] if the filter string is invalid or the
/// global subscriber cannot be installed (e.g. another subscriber was set
/// directly on the `tracing` registry by a different code path).
pub fn init(cfg: &LogConfig) -> Result<(), Error> {
    let mut already_initialised = true;
    INIT.get_or_init(|| {
        already_initialised = false;
    });
    if already_initialised {
        return Ok(());
    }

    let filter = EnvFilter::try_new(&cfg.filter)
        .map_err(|e| Error::Telemetry(format!("invalid log filter: {e}")))?;

    let registry = tracing_subscriber::registry().with(filter);

    if cfg.json {
        registry
            .with(fmt::layer().json())
            .try_init()
            .map_err(|e| Error::Telemetry(e.to_string()))?;
    } else {
        registry
            .with(fmt::layer())
            .try_init()
            .map_err(|e| Error::Telemetry(e.to_string()))?;
    }

    Ok(())
}
