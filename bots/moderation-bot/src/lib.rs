// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! imogo moderation bot.
//!
//! Eigenständiger Application Service auf B2B, der per Befehl moderiert und
//! Bann-Wörter automatisch redactet/warnt/kickt. Login-Pfad: `m.login.application_service`
//! gefolgt von `restore_session` (Pull-AS-Architektur).
//!
//! Pure-function modules (`audit`, `banned_words`, `command`, `format`,
//! `power_level`, `rooms`) sind isolation-testbar; Matrix-SDK-spezifische
//! Logik lebt in `matrix_client` und `handler` und wird über Live-Tests
//! gegen Tuwunel verifiziert.

pub mod audit;
pub mod banned_words;
pub mod command;
pub mod config;
pub mod db;
pub mod error;
pub mod format;
pub mod handler;
pub mod matrix_client;
pub mod mute;
pub mod pinned;
pub mod power_level;
pub mod reload;
pub mod rooms;

/// Crate version, taken from Cargo.toml at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
