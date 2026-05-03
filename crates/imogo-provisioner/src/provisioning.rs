// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! License-event-driven provisioning workflows.
//!
//! Briefing-02c-3 added the activation flow. Briefing-02d adds the three
//! lifecycle handlers `handle_license_expired`, `handle_license_deactivated`,
//! and `handle_license_tier_changed`. All four are idempotent: replaying a
//! verified event for an already-handled state is a no-op (with an
//! `*.idempotent` audit entry).
//!
//! The initial password is returned exactly once on activation and never
//! persisted.

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tracing::{info, instrument, warn};

use crate::{
    accounts::{AccountError, AccountRecord, AccountState, AccountsRepo, NewAccount},
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

    /// Lifecycle event arrived for a license that has no account record.
    /// The webhook handler maps this to HTTP 409 Conflict.
    #[error("account not found for license '{0}', activate first")]
    AccountNotFound(String),

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

/// Inbound license expiration payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseExpiredPayload {
    /// Event discriminator. Must equal `"license.expired"`.
    pub event_type: String,
    /// Opaque license id from the license server.
    pub license_id: String,
}

/// Inbound license deactivation payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseDeactivatedPayload {
    /// Event discriminator. Must equal `"license.deactivated"`.
    pub event_type: String,
    /// Opaque license id from the license server.
    pub license_id: String,
}

/// Inbound license tier-change payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseTierChangedPayload {
    /// Event discriminator. Must equal `"license.tier_changed"`.
    pub event_type: String,
    /// Opaque license id from the license server.
    pub license_id: String,
    /// Target tier (validated against `allowed_tiers`).
    pub new_tier: String,
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

/// Outcome of a lifecycle event (`expired`, `deactivated`, `tier_changed`).
#[derive(Debug, Clone, Serialize)]
pub struct LifecycleOutcome {
    /// True if the requested transition was a no-op (state already correct).
    pub already_in_target_state: bool,
    /// State label before the call. Stable lower-snake-case.
    pub previous_state: String,
    /// State label after the call. Stable lower-snake-case.
    pub new_state: String,
    /// The (possibly updated) account record.
    pub account: AccountRecord,
}

/// Service that orchestrates the activation and lifecycle workflows.
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
        self.validate_activation_payload(&payload)?;

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

        let power_levels = self.build_power_levels(conn, &server_name, &user_id, 50);

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

    /// Handle a verified `license.expired` event. Idempotent.
    ///
    /// Transitions an `active` account to `read_only`. If the account is
    /// already `read_only` or `deactivated`, returns the existing record
    /// without changes (and writes an `*.idempotent` audit entry).
    ///
    /// # Errors
    ///
    /// Returns [`ProvisioningError::AccountNotFound`] (mapped to 409),
    /// [`ProvisioningError::HomeserverNotRegistered`], [`ProvisioningError::Tuwunel`],
    /// [`ProvisioningError::Account`], or [`ProvisioningError::Audit`].
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, payload), fields(license_id = %payload.license_id))]
    pub async fn handle_license_expired(
        &self,
        payload: LicenseExpiredPayload,
    ) -> Result<LifecycleOutcome, ProvisioningError> {
        let account = self
            .accounts
            .find_by_license(&payload.license_id)
            .await?
            .ok_or_else(|| ProvisioningError::AccountNotFound(payload.license_id.clone()))?;

        let previous_state = account.state.as_str().to_string();

        if matches!(
            account.state,
            AccountState::ReadOnly | AccountState::Deactivated
        ) {
            self.audit
                .append(NewAuditEntry {
                    event_type: "license.expired.idempotent".to_string(),
                    actor: "license-server".to_string(),
                    subject: Some(format!("license:{}", payload.license_id)),
                    payload_json: json!({
                        "license_id": payload.license_id,
                        "current_state": previous_state,
                    })
                    .to_string(),
                })
                .await?;
            return Ok(LifecycleOutcome {
                already_in_target_state: true,
                previous_state: previous_state.clone(),
                new_state: previous_state,
                account,
            });
        }

        // Update Matrix power levels (set customer to 0 = no write).
        let conn = self.matrix.get(&account.matrix_homeserver).ok_or_else(|| {
            ProvisioningError::HomeserverNotRegistered(account.matrix_homeserver.clone())
        })?;
        let tuwunel = self.tuwunel_for(conn);

        let server_name = conn.config.server_name.clone();
        let power_levels = self.build_power_levels(conn, &server_name, &account.matrix_user_id, 0);

        tuwunel
            .update_power_levels(&account.support_room_id, &power_levels)
            .await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "power_level.updated".to_string(),
                actor: "system".to_string(),
                subject: Some(format!("room:{}", account.support_room_id)),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "matrix_user_id": account.matrix_user_id,
                    "new_user_level": 0,
                    "reason": "license_expired",
                })
                .to_string(),
            })
            .await?;

        self.accounts.mark_expired(&payload.license_id).await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "license.expired.processed".to_string(),
                actor: "license-server".to_string(),
                subject: Some(format!("license:{}", payload.license_id)),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "previous_state": previous_state,
                    "new_state": "read_only",
                })
                .to_string(),
            })
            .await?;

        let updated = self
            .accounts
            .find_by_license(&payload.license_id)
            .await?
            .ok_or_else(|| ProvisioningError::AccountNotFound(payload.license_id.clone()))?;

        info!(
            license_id = payload.license_id.as_str(),
            "license expired processed"
        );

        Ok(LifecycleOutcome {
            already_in_target_state: false,
            previous_state,
            new_state: "read_only".to_string(),
            account: updated,
        })
    }

    /// Handle a verified `license.deactivated` event. Idempotent.
    ///
    /// Transitions any non-deactivated account to `deactivated`. The Tuwunel
    /// account is sent through the admin deactivate endpoint; if that call
    /// fails, we still mark the local state as deactivated and warn-log,
    /// matching the briefing rule "Tuwunel-Deaktivierung ist nicht-fatal".
    ///
    /// # Errors
    ///
    /// Returns [`ProvisioningError::AccountNotFound`] (mapped to 409) or
    /// any of the database/audit error variants.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, payload), fields(license_id = %payload.license_id))]
    pub async fn handle_license_deactivated(
        &self,
        payload: LicenseDeactivatedPayload,
    ) -> Result<LifecycleOutcome, ProvisioningError> {
        let account = self
            .accounts
            .find_by_license(&payload.license_id)
            .await?
            .ok_or_else(|| ProvisioningError::AccountNotFound(payload.license_id.clone()))?;

        let previous_state = account.state.as_str().to_string();

        if account.state == AccountState::Deactivated {
            self.audit
                .append(NewAuditEntry {
                    event_type: "license.deactivated.idempotent".to_string(),
                    actor: "license-server".to_string(),
                    subject: Some(format!("license:{}", payload.license_id)),
                    payload_json: json!({
                        "license_id": payload.license_id,
                        "current_state": previous_state,
                    })
                    .to_string(),
                })
                .await?;
            return Ok(LifecycleOutcome {
                already_in_target_state: true,
                previous_state: previous_state.clone(),
                new_state: previous_state,
                account,
            });
        }

        let conn = self.matrix.get(&account.matrix_homeserver).ok_or_else(|| {
            ProvisioningError::HomeserverNotRegistered(account.matrix_homeserver.clone())
        })?;
        let tuwunel = self.tuwunel_for(conn);
        let server_name = conn.config.server_name.clone();

        // Best-effort power-level lockdown before we kill the login.
        let power_levels = self.build_power_levels(conn, &server_name, &account.matrix_user_id, 0);
        if let Err(e) = tuwunel
            .update_power_levels(&account.support_room_id, &power_levels)
            .await
        {
            warn!(error = %e, "update power levels failed during deactivation, continuing");
        }

        // Tuwunel deactivate. Failure here is non-fatal; we still mark
        // the DB state as deactivated so the license server's view stays
        // consistent. A later reconcile pass can retry the Tuwunel side.
        if let Err(e) = tuwunel.deactivate_user(&account.matrix_user_id).await {
            warn!(error = %e, "tuwunel deactivate_user failed, marking deactivated in db anyway");
        }

        self.audit
            .append(NewAuditEntry {
                event_type: "account.deactivated".to_string(),
                actor: "system".to_string(),
                subject: Some(format!("account:{}", account.matrix_user_id)),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "matrix_user_id": account.matrix_user_id,
                    "previous_state": previous_state,
                })
                .to_string(),
            })
            .await?;

        self.accounts.mark_deactivated(&payload.license_id).await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "license.deactivated.processed".to_string(),
                actor: "license-server".to_string(),
                subject: Some(format!("license:{}", payload.license_id)),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "previous_state": previous_state,
                    "new_state": "deactivated",
                })
                .to_string(),
            })
            .await?;

        let updated = self
            .accounts
            .find_by_license(&payload.license_id)
            .await?
            .ok_or_else(|| ProvisioningError::AccountNotFound(payload.license_id.clone()))?;

        info!(
            license_id = payload.license_id.as_str(),
            "license deactivated processed"
        );

        Ok(LifecycleOutcome {
            already_in_target_state: false,
            previous_state,
            new_state: "deactivated".to_string(),
            account: updated,
        })
    }

    /// Handle a verified `license.tier_changed` event. Idempotent.
    ///
    /// Updates the tier in the database and the support room topic. Power
    /// levels are unchanged (all tiers share the same PL scheme).
    ///
    /// # Errors
    ///
    /// Returns [`ProvisioningError::AccountNotFound`] if the license has no
    /// record, [`ProvisioningError::InvalidTier`] if the new tier is not in
    /// `allowed_tiers`, or any of the persistence error variants.
    #[allow(clippy::too_many_lines)]
    #[instrument(skip(self, payload), fields(license_id = %payload.license_id))]
    pub async fn handle_license_tier_changed(
        &self,
        payload: LicenseTierChangedPayload,
    ) -> Result<LifecycleOutcome, ProvisioningError> {
        let account = self
            .accounts
            .find_by_license(&payload.license_id)
            .await?
            .ok_or_else(|| ProvisioningError::AccountNotFound(payload.license_id.clone()))?;

        let state_str = account.state.as_str().to_string();

        if account.tier == payload.new_tier {
            self.audit
                .append(NewAuditEntry {
                    event_type: "license.tier_changed.idempotent".to_string(),
                    actor: "license-server".to_string(),
                    subject: Some(format!("license:{}", payload.license_id)),
                    payload_json: json!({
                        "license_id": payload.license_id,
                        "tier": payload.new_tier,
                    })
                    .to_string(),
                })
                .await?;
            return Ok(LifecycleOutcome {
                already_in_target_state: true,
                previous_state: state_str.clone(),
                new_state: state_str,
                account,
            });
        }

        if !self
            .cfg
            .allowed_tiers
            .iter()
            .any(|t| t == &payload.new_tier)
        {
            return Err(ProvisioningError::InvalidTier(payload.new_tier.clone()));
        }

        self.accounts
            .update_tier(&payload.license_id, &payload.new_tier)
            .await?;

        // Update the room topic so the SLA in the topic reflects the new tier.
        let conn = self.matrix.get(&account.matrix_homeserver).ok_or_else(|| {
            ProvisioningError::HomeserverNotRegistered(account.matrix_homeserver.clone())
        })?;
        let tuwunel = self.tuwunel_for(conn);
        let new_topic = format!(
            "Premium-Support fuer {}. SLA: {}",
            account.display_name,
            sla_for_tier(&payload.new_tier)
        );
        if let Err(e) = tuwunel
            .update_room_topic(&account.support_room_id, &new_topic)
            .await
        {
            warn!(error = %e, "update room topic failed during tier change, continuing");
        }

        self.audit
            .append(NewAuditEntry {
                event_type: "room.topic_updated".to_string(),
                actor: "system".to_string(),
                subject: Some(format!("room:{}", account.support_room_id)),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "new_tier": payload.new_tier,
                    "new_topic": new_topic,
                })
                .to_string(),
            })
            .await?;

        self.audit
            .append(NewAuditEntry {
                event_type: "license.tier_changed.processed".to_string(),
                actor: "license-server".to_string(),
                subject: Some(format!("license:{}", payload.license_id)),
                payload_json: json!({
                    "license_id": payload.license_id,
                    "previous_tier": account.tier,
                    "new_tier": payload.new_tier,
                })
                .to_string(),
            })
            .await?;

        let updated = self
            .accounts
            .find_by_license(&payload.license_id)
            .await?
            .ok_or_else(|| ProvisioningError::AccountNotFound(payload.license_id.clone()))?;

        info!(
            license_id = payload.license_id.as_str(),
            new_tier = payload.new_tier.as_str(),
            "license tier change processed"
        );

        Ok(LifecycleOutcome {
            already_in_target_state: false,
            previous_state: state_str.clone(),
            new_state: state_str,
            account: updated,
        })
    }

    fn validate_activation_payload(
        &self,
        p: &LicenseActivatedPayload,
    ) -> Result<(), ProvisioningError> {
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

    /// Build a [`PowerLevels`] structure with the support team and the
    /// provisioner bot at 100, and the customer at `customer_level`.
    fn build_power_levels(
        &self,
        conn: &HomeserverConnection,
        server_name: &str,
        customer_user_id: &str,
        customer_level: i64,
    ) -> PowerLevels {
        let mut user_levels = std::collections::BTreeMap::new();
        for support in &self.cfg.support_invitees {
            user_levels.insert(support.clone(), 100);
        }
        user_levels.insert(
            format!("@{}:{}", conn.config.sender_localpart, server_name),
            100,
        );
        user_levels.insert(customer_user_id.to_string(), customer_level);

        PowerLevels {
            users: user_levels,
            users_default: 0,
            events_default: 50,
            state_default: 50,
            invite: 50,
            kick: 50,
            ban: 50,
            redact: 50,
        }
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
