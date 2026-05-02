// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Health and readiness handlers.

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;

use super::appservice::AppState;
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

/// Readiness response payload.
#[derive(Debug, Serialize)]
pub struct Ready {
    /// `"ok"` if all configured homeservers respond, `"degraded"` otherwise.
    pub status: &'static str,
    /// Crate version of the running provisioner.
    pub version: &'static str,
    /// Logical names of homeservers that responded successfully.
    pub healthy_homeservers: Vec<String>,
    /// Total count of configured homeservers.
    pub total_homeservers: usize,
}

/// Liveness endpoint. Returns 200 as long as the process is up.
#[allow(clippy::unused_async)]
pub async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: VERSION,
    })
}

/// Readiness endpoint. Returns 200 only when every configured homeserver is
/// reachable. With zero configured homeservers the endpoint returns 200 and
/// reports an empty list.
pub async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<Ready>) {
    let healthy = state.registry.ping_all().await;
    let total = state.registry.iter().count();
    let ready = healthy.len() == total;
    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status_code,
        Json(Ready {
            status: if ready { "ok" } else { "degraded" },
            version: VERSION,
            healthy_homeservers: healthy,
            total_homeservers: total,
        }),
    )
}
