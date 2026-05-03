// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! imogo FAQ bot.
//!
//! A reactive Matrix bot that answers frequently asked questions in imogo's
//! community rooms. The bot triggers on three patterns: explicit `!faq` slash
//! commands, mentions of its Mxid, and any message in a 1:1 DM room.

pub mod config;
pub mod faqs;
pub mod handler;
pub mod matrix_client;
pub mod reload;

/// Crate version, taken from Cargo.toml at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
