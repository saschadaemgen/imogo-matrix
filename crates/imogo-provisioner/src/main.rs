// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! imogo-provisioner binary entry point.

use std::process::ExitCode;

use imogo_provisioner::{config::Config, http, telemetry};
use tracing::{error, info};

#[tokio::main]
async fn main() -> ExitCode {
    // Load configuration from file plus environment variables.
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to load configuration: {e:#}");
            return ExitCode::from(2);
        }
    };

    // Initialise structured logging based on the loaded config.
    if let Err(e) = telemetry::init(&config.log) {
        eprintln!("failed to initialise telemetry: {e:#}");
        return ExitCode::from(2);
    }

    info!(
        version = imogo_provisioner::VERSION,
        listen = %config.http.listen,
        "imogo-provisioner starting"
    );

    // Run the HTTP server until shutdown signal.
    match http::run(config).await {
        Ok(()) => {
            info!("imogo-provisioner shut down cleanly");
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!(error = ?e, "imogo-provisioner exited with error");
            ExitCode::FAILURE
        }
    }
}
