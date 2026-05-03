// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Capability tokens (JWT) issued by the imogo license server.
//!
//! Tokens authorise the imogo desktop application to call the b2c
//! provisioning API on behalf of a specific licensee. Algorithm is `EdDSA`
//! (Ed25519); replay protection uses the `jti` claim against a SQLite-backed
//! cache.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;
use tracing::trace;

use crate::keys::CapabilityKeyRegistry;

const ISSUER: &str = "imogo-license-server";

/// Errors raised by capability-token verification.
#[derive(Debug, Error)]
#[allow(clippy::module_name_repetitions)]
pub enum CapabilityError {
    /// Authorization header missing or not a valid Bearer token.
    #[error("missing or malformed authorization header")]
    BadAuthHeader,

    /// JOSE header decode failed.
    #[error("token decode failed: {0}")]
    Decode(String),

    /// `kid` not registered.
    #[error("unknown key id")]
    UnknownKeyId,

    /// Signature or claim validation failed.
    #[error("invalid signature or claims: {0}")]
    Invalid(String),

    /// Token expired or future-dated outside leeway.
    #[error("token expired or future-dated")]
    Expired,

    /// Issued-at older than the 24h cap.
    #[error("issued-at too far in the past")]
    IatTooOld,

    /// `jti` already seen in the cache.
    #[error("token replay (jti seen)")]
    Replay,

    /// `caps` did not include the required capability.
    #[error("missing required capability: {0}")]
    MissingCapability(String),

    /// Underlying sqlx error.
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Capability JWT claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityClaims {
    /// Issuer, must equal `imogo-license-server`.
    pub iss: String,
    /// Subject: opaque license id of the holder.
    pub sub: String,
    /// Fully qualified Matrix user id of the licensee on B2B.
    pub matrix_user_id: String,
    /// Capabilities granted by this token.
    pub caps: Vec<String>,
    /// Issued-at, Unix seconds.
    pub iat: i64,
    /// Expiration, Unix seconds.
    pub exp: i64,
    /// Token id (UUID v4) for replay protection.
    pub jti: String,
}

/// Capability-token verifier.
#[derive(Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct CapabilityVerifier {
    keys: CapabilityKeyRegistry,
    pool: SqlitePool,
}

impl std::fmt::Debug for CapabilityVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapabilityVerifier")
            .field("registered_keys", &self.keys.len())
            .finish_non_exhaustive()
    }
}

impl CapabilityVerifier {
    /// Construct a new verifier using the given key registry and pool.
    #[must_use]
    pub fn new(keys: CapabilityKeyRegistry, pool: SqlitePool) -> Self {
        Self { keys, pool }
    }

    /// Validate a Bearer token and require that its `caps` list contains
    /// `required_cap`.
    ///
    /// # Errors
    ///
    /// Returns the specific [`CapabilityError`] for the first failure
    /// encountered. Callers should map all variants to HTTP 401.
    pub async fn verify(
        &self,
        bearer_value: Option<&str>,
        required_cap: &str,
    ) -> Result<CapabilityClaims, CapabilityError> {
        let token = strip_bearer(bearer_value).ok_or(CapabilityError::BadAuthHeader)?;

        let header = decode_header(token).map_err(|e| CapabilityError::Decode(e.to_string()))?;

        if header.alg != Algorithm::EdDSA {
            return Err(CapabilityError::Invalid(format!(
                "unexpected alg {:?}",
                header.alg
            )));
        }

        let kid = header
            .kid
            .ok_or_else(|| CapabilityError::Invalid("missing kid".to_string()))?;
        let key = self
            .keys
            .lookup(&kid)
            .ok_or(CapabilityError::UnknownKeyId)?;

        let decoding_key = ed25519_decoding_key(&key.key.to_bytes());

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[ISSUER]);
        validation.set_required_spec_claims(&["iss", "sub", "iat", "exp"]);
        validation.leeway = 60;

        let token_data = decode::<CapabilityClaims>(token, &decoding_key, &validation)
            .map_err(|e| CapabilityError::Invalid(e.to_string()))?;
        let claims = token_data.claims;

        // iat-not-too-old check (jsonwebtoken does not enforce this).
        let now = current_unix_secs()?;
        if now - claims.iat > 24 * 3600 {
            return Err(CapabilityError::IatTooOld);
        }

        if !claims.caps.iter().any(|c| c == required_cap) {
            return Err(CapabilityError::MissingCapability(required_cap.to_string()));
        }

        // Replay protection: insert jti only after signature has been
        // verified, to avoid attacker-induced cache pollution.
        let inserted = insert_jti_if_fresh(&self.pool, &claims.jti, claims.exp).await?;
        if !inserted {
            return Err(CapabilityError::Replay);
        }

        trace!(
            jti = claims.jti.as_str(),
            sub = claims.sub.as_str(),
            "capability token verified"
        );
        Ok(claims)
    }
}

/// Insert a jti into the cache. Returns true if it was new.
async fn insert_jti_if_fresh(pool: &SqlitePool, jti: &str, exp: i64) -> Result<bool, sqlx::Error> {
    let now = Utc::now();
    let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let expires_at_ts = DateTime::<Utc>::from_timestamp(exp + 60, 0).unwrap_or(now);
    let expires_str = expires_at_ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM capability_jti_cache WHERE expires_at < ?")
        .bind(&now_str)
        .execute(&mut *tx)
        .await?;

    let res =
        sqlx::query("INSERT OR IGNORE INTO capability_jti_cache (jti, expires_at) VALUES (?, ?)")
            .bind(jti)
            .bind(&expires_str)
            .execute(&mut *tx)
            .await?;

    tx.commit().await?;
    Ok(res.rows_affected() == 1)
}

/// Extract the bearer token from an `Authorization: Bearer <token>` header
/// value. Case-insensitive on the `Bearer` keyword.
fn strip_bearer(value: Option<&str>) -> Option<&str> {
    let v = value?.trim();
    let (head, tail) = v.split_once(' ')?;
    if head.eq_ignore_ascii_case("Bearer") {
        let token = tail.trim();
        (!token.is_empty()).then_some(token)
    } else {
        None
    }
}

fn current_unix_secs() -> Result<i64, CapabilityError> {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CapabilityError::Expired)?
        .as_secs();
    i64::try_from(n).map_err(|_| CapabilityError::Expired)
}

/// Build a jsonwebtoken `DecodingKey` from raw 32-byte Ed25519 public key
/// bytes. Despite the `from_ed_der` name, jsonwebtoken 9.x feeds the bytes
/// directly to ring's `UnparsedPublicKey` for `ED25519`, which expects the
/// raw 32-byte key (not a `SubjectPublicKeyInfo` DER wrapper).
fn ed25519_decoding_key(raw: &[u8]) -> DecodingKey {
    DecodingKey::from_ed_der(raw)
}
