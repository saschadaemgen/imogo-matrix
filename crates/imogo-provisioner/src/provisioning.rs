// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! License-event-driven provisioning workflows.
//!
//! The provisioner receives a verified `license.activated` webhook and runs
//! through these steps in order:
//!
//! 1. Validate the payload (presence of `license_id`, `tier`, customer name).
//! 2. Check the accounts repository for an existing record (idempotency).
//! 3. Generate stable matrix UUID, display name, room alias.
//! 4. Register the account on the configured B2B homeserver.
//! 5. Set the display name.
//! 6. Create the support room with the configured invitees and power levels.
//! 7. Persist the account record.
//! 8. Append audit log entries for every step.
//! 9. Return the account record plus the initial password to the caller.
//!
//! The initial password is returned exactly once and never persisted.

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tracing::{info, instrument, warn};

use crate::{
    accounts::{AccountError, AccountRecord, AccountsRepo, NewAccount},
    audit::{AuditError, AuditLog, NewAuditEntry},
    config::ProvisioningConfig,
    identity,
    matrix::{HomeserverConnection, MatrixRegistry},
    tuwunel::{PowerLevels, TuwunelClient, TuwunelError},
};

/// Errors raised by [`ProvisioningService`].
#[derive(Debug, Error)]
#[allow(clippy::module_name_repetitions)]
pub enum ProvisioningError {
    /// `license_id` was empty or absent.
    #[error("license_id missing or empty")]
    MissingLicenseId,

    /// `tier` was empty or not in `allowed_tiers`.
    #[error("tier missing or not allowed: {0}")]
    InvalidTier(String),

    /// Customer name was empty.
    #[error("customer name missing")]
    MissingCustomerName,

    /// `b2b_homeserver` from config did not match any registered homeserver.
    #[error("configured b2b homeserver '{0}' not registered")]
    HomeserverNotRegistered(String),

    /// Tuwunel call failed (transport or API error).
    #[error("tuwunel api error: {0}")]
    Tuwunel(#[from] TuwunelError),

    /// `accounts` table operation failed.
    #[error("account record error: {0}")]
    Account(#[from] AccountError),

    /// Audit append failed.
    #[error("audit log error: {0}")]
    Audit(#[from] AuditError),
}

/// Inbound license activation payload (from the license server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseActivatedPayload {
    /// Event discriminator. Must equal `"license.activated"`.
    pub event_type: String,
    /// Opaque license id from the license server.
    pub license_id: String,
    /// Tier label (validated against `allowed_tiers`).
    pub tier: String,
    /// Customer info used for display name and room topic.
    pub customer: CustomerInfo,
}

/// Customer details supplied by the license server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerInfo {
    /// Person name (required).
    pub name: String,
    /// Optional company name.
    #[serde(default)]
    pub company: Option<String>,
    /// Optional contact email (not used by the provisioner yet).
    #[serde(default)]
    pub email: Option<String>,
}

/// Outcome of a successful activation. Returned in the webhook response.
#[derive(Debug, Clone, Serialize)]
pub struct ActivationOutcome {
    /// True if the account already existed (idempotent return).
    pub already_existed: bool,
    /// Persistent account record.
    pub account: AccountRecord,
    /// Only populated for newly created accounts. Never persisted, never
    /// returned again on subsequent calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_password: Option<String>,
}

/// Service that orchestrates the activation workflow.
#[derive(Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct ProvisioningService {
    accounts: AccountsRepo,
    audit: AuditLog,
    matrix: MatrixRegistry,
    cfg: ProvisioningConfig,
    http: reqwest::Client,
}

impl std::fmt::Debug for ProvisioningService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvisioningService")
            .field("b2b_homeserver", &self.cfg.b2b_homeserver)
            .field("allowed_tiers", &self.cfg.allowed_tiers)
            .finish_non_exhaustive()
    }
}

impl ProvisioningService {
    /// Construct a new service.
    #[must_use]
    pub fn new(
        accounts: AccountsRepo,
        audit: AuditLog,
        matrix: MatrixRegistry,
        cfg: ProvisioningConfig,
        http: reqwest::Client,
    ) -> Self {
        Self {
            accounts,
            audit,
            matrix,
            cfg,
            http,
        }
    }

    /// Handle a verified `license.activated` event.
    ///
    /// # Errors
    ///
    /// Returns [`ProvisioningError`] for validation failures, homeserver
    /// failures, or persistence failures. The webhook handler maps each
    /// variant to a specific HTTP status code.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, payload), fields(license_id = %payload.license_id))]
    pub async fn handle_license_activated(
        &self,
        payload: LicenseActivatedPayload,
    ) -> Result<ActivationOutcome, ProvisioningError> {
        // Step 1: Validate
        self.validate_payload(&payload)?;

        // Step 2: Idempotency check
        if let Some(existing) = self.accounts.find_by_license(&payload.license_id).await? {
            info!(
                license_id = payload.license_id.as_str(),
                "account already exists, returning existing"
            );
            self.audit
                .append(NewAuditEntry {
                    event_type: "license.activated.idempotent".to_string(),
                    actor: "license-server".to_string(),
                    subject: Some(format!("license:{}", payload.license_id)),
                    payload_json: json!({
                        "license_id": payload.license_id,
                        "matrix_user_id": existing.matrix_user_id,
                    })
                    .to_string(),
                })
                .await?;

            return Ok(ActivationOutcome {
                already_existed: true,
                account: existing,
                initial_password: None,
            });
        }

        // Step 3: Generate identities
        let conn = self.matrix.get(&self.cfg.b2b_homeserver).ok_or_else(|| {
            ProvisioningError::HomeserverNotRegistered(self.cfg.b2b_homeserver.clone())
        })?;

        let server_name = conn.config.server_name.clone();
        let matrix_uuid = identity::generate_matrix_uuid();
        let user_id = identity::build_user_id(&matrix_uuid, &server_name);
        let display_name = identity::build_display_name(
            &payload.customer.name,
            payload.customer.company.as_deref(),
        );
        let room_alias_full = identity::build_support_room_alias(&matrix_uuid, &server_name);
        let room_alias_localpart = format!("support-{}", identity::matrix_uuid_short(&matrix_uuid));
        let initial_password = identity::generate_initial_password();

        // Step 4: Register on Tuwunel
        let tuwunel = self.tuwunel_for(conn);
        let _registered = tuwunel
            .register_user(&matrix_uuid, &initial_password)
            .await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "account.created".to_string(),
                actor: "system".to_string(),
                subject: Some(format!("account:{user_id}")),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "matrix_user_id": user_id,
                    "tier": payload.tier,
                })
                .to_string(),
            })
            .await?;

        // Step 5: Display name (failure here is non-fatal; logged and continued).
        if let Err(e) = tuwunel.set_display_name(&user_id, &display_name).await {
            warn!(error = %e, "set display name failed, continuing");
        }

        // Step 6: Create support room
        let topic = format!(
            "Premium-Support fuer {}. SLA: {}",
            display_name,
            sla_for_tier(&payload.tier)
        );
        let mut invitees = self.cfg.support_invitees.clone();
        invitees.push(user_id.clone());

        let mut user_levels = std::collections::BTreeMap::new();
        for support in &self.cfg.support_invitees {
            user_levels.insert(support.clone(), 100);
        }
        // Provisioner bot itself.
        user_levels.insert(
            format!("@{}:{}", conn.config.sender_localpart, server_name),
            100,
        );
        // Customer.
        user_levels.insert(user_id.clone(), 50);

        let power_levels = PowerLevels {
            users: user_levels,
            users_default: 0,
            events_default: 50,
            state_default: 50,
            invite: 50,
            kick: 50,
            ban: 50,
            redact: 50,
        };

        let room = tuwunel
            .create_room(&room_alias_localpart, &topic, &invitees, &power_levels)
            .await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "room.created".to_string(),
                actor: "system".to_string(),
                subject: Some(format!("room:{}", room.room_id)),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "matrix_user_id": user_id,
                    "room_alias": room_alias_full,
                    "invited": invitees,
                })
                .to_string(),
            })
            .await?;

        // Step 7: Persist account record
        let account = self
            .accounts
            .insert(NewAccount {
                license_id: payload.license_id.clone(),
                matrix_uuid,
                matrix_homeserver: self.cfg.b2b_homeserver.clone(),
                matrix_user_id: user_id,
                support_room_id: room.room_id,
                display_name,
                tier: payload.tier.clone(),
            })
            .await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "license.activated.completed".to_string(),
                actor: "system".to_string(),
                subject: Some(format!("license:{}", payload.license_id)),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "matrix_user_id": account.matrix_user_id,
                    "support_room_id": account.support_room_id,
                })
                .to_string(),
            })
            .await?;

        Ok(ActivationOutcome {
            already_existed: false,
            account,
            initial_password: Some(initial_password),
        })
    }

    fn validate_payload(&self, p: &LicenseActivatedPayload) -> Result<(), ProvisioningError> {
        if p.license_id.trim().is_empty() {
            return Err(ProvisioningError::MissingLicenseId);
        }
        if !self.cfg.allowed_tiers.iter().any(|t| t == &p.tier) {
            return Err(ProvisioningError::InvalidTier(p.tier.clone()));
        }
        if p.customer.name.trim().is_empty() {
            return Err(ProvisioningError::MissingCustomerName);
        }
        Ok(())
    }

    fn tuwunel_for(&self, conn: &HomeserverConnection) -> TuwunelClient {
        TuwunelClient::new(
            self.http.clone(),
            conn.config.url.as_str().trim_end_matches('/').to_string(),
            conn.config.as_token.clone(),
        )
    }
}

#[must_use]
fn sla_for_tier(tier: &str) -> &'static str {
    match tier {
        "pro" | "enterprise" => "Antwort innerhalb von 4 Stunden",
        "kmu" => "Antwort innerhalb von 24 Stunden",
        _ => "Antwort innerhalb von 48 Stunden",
    }
}
