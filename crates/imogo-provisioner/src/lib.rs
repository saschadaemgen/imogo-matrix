// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! imogo-provisioner library root.
//!
//! This crate provides a Matrix Application Service that manages account
//! lifecycle for imogo platform licensees. It is started as a binary, but
//! the bulk of the logic lives in this library so it can be tested in
//! isolation.

pub mod accounts;
pub mod audit;
pub mod config;
pub mod db;
pub mod error;
pub mod http;
pub mod identity;
pub mod keys;
pub mod matrix;
pub mod nonce_store;
pub mod provisioning;
pub mod telemetry;
pub mod tuwunel;
pub mod webhook;

/// Crate version, taken from Cargo.toml at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
