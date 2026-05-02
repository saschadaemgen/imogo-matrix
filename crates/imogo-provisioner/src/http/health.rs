// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Health and readiness handlers.

use axum::Json;
use serde::Serialize;

use crate::VERSION;

/// Health response payload.
#[derive(Debug, Serialize)]
#[allow(clippy::module_name_repetitions)]
pub struct Health {
    /// Always `"ok"` when the process is running.
    pub status: &'static str,
    /// Crate version of the running provisioner.
    pub version: &'static str,
}

/// Liveness endpoint. Returns 200 as long as the process is up.
pub async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: VERSION,
    })
}

/// Readiness endpoint. In 02a always returns 200. From 02b on, this will
/// check Matrix Application Service connectivity.
pub async fn readyz() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: VERSION,
    })
}
