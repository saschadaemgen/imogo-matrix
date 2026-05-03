// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! imogo FAQ bot binary entry point.

use std::{path::PathBuf, process::ExitCode, sync::Arc};

use arc_swap::ArcSwap;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use faq_bot::{
    config::{Config, LogConfig},
    faqs,
    handler::FaqStore,
    matrix_client, reload,
};

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e:#}");
            return ExitCode::from(2);
        }
    };

    if let Err(e) = init_logging(&config.log) {
        eprintln!("logging init failed: {e}");
        return ExitCode::from(2);
    }

    info!(version = faq_bot::VERSION, "imogo FAQ-bot starting");

    let path = PathBuf::from(&config.faqs.path);
    let initial = match faqs::load(&path).await {
        Ok(f) => f,
        Err(e) => {
            error!(error = %e, "loading FAQs failed");
            return ExitCode::FAILURE;
        }
    };
    info!(count = initial.len(), "FAQs loaded");
    let store: FaqStore = Arc::new(ArcSwap::new(Arc::new(initial)));

    let _watcher = if config.faqs.watch {
        match reload::spawn_watcher(path.clone(), store.clone()) {
            Ok(w) => Some(w),
            Err(e) => {
                error!(error = %e, "watcher setup failed");
                None
            }
        }
    } else {
        None
    };

    let client = match matrix_client::build_client(&config.matrix).await {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "matrix client build failed");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = matrix_client::run(client, store, config.matrix.user_id.clone()).await {
        error!(error = %e, "sync loop ended");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn init_logging(cfg: &LogConfig) -> Result<(), String> {
    let filter = EnvFilter::try_new(&cfg.filter).map_err(|e| e.to_string())?;
    let registry = tracing_subscriber::registry().with(filter);
    if cfg.json {
        registry
            .with(fmt::layer().json())
            .try_init()
            .map_err(|e| e.to_string())?;
    } else {
        registry
            .with(fmt::layer())
            .try_init()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
