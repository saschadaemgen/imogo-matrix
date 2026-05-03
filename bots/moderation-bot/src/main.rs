// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! imogo moderation bot binary entry point.

use std::{process::ExitCode, sync::Arc};

use regex::Regex;
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use moderation_bot::{
    banned_words::WordCache,
    config::{Config, TelemetryConfig},
    db,
    handler::{self, BotState},
    matrix_client,
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

    if let Err(e) = init_logging(&config.telemetry) {
        eprintln!("logging init failed: {e}");
        return ExitCode::from(2);
    }

    info!(
        version = moderation_bot::VERSION,
        "imogo moderation-bot starting"
    );

    if config.matrix.as_token.is_empty() {
        error!(
            "matrix.as_token is empty in mod-bot.toml; set it to the value Tuwunel \
             returned at AS registration time"
        );
        return ExitCode::FAILURE;
    }

    let pool = match db::open_pool(&config.database).await {
        Ok(p) => p,
        Err(e) => {
            error!(error = %e, "database open failed");
            return ExitCode::FAILURE;
        }
    };

    let word_cache = WordCache::new();
    if let Err(e) = word_cache.refresh(&pool).await {
        error!(error = %e, "initial banned-word cache load failed");
        return ExitCode::FAILURE;
    }

    let alias_regex = match Regex::new(&config.bot.auto_discover_alias_pattern) {
        Ok(r) => Arc::new(RwLock::new(r)),
        Err(e) => {
            error!(error = %e, "invalid auto_discover_alias_pattern regex");
            return ExitCode::FAILURE;
        }
    };

    let client = match matrix_client::build_and_login(&config.matrix).await {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "matrix client login failed");
            return ExitCode::FAILURE;
        }
    };

    let bot_user_id = match config.matrix.user_id.parse() {
        Ok(uid) => uid,
        Err(e) => {
            error!(error = %e, "invalid matrix.user_id in config");
            return ExitCode::FAILURE;
        }
    };

    let state = BotState {
        pool,
        word_cache,
        alias_regex,
        bot_user_id,
        config: Arc::new(config),
    };

    if let Err(e) = handler::run(client, state).await {
        error!(error = %e, "sync loop ended");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn init_logging(cfg: &TelemetryConfig) -> Result<(), String> {
    let filter_str = format!("moderation_bot={},matrix_sdk=warn", cfg.log_level);
    let filter = EnvFilter::try_new(&filter_str).map_err(|e| e.to_string())?;
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .try_init()
        .map_err(|e| e.to_string())?;
    Ok(())
}
