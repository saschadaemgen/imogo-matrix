// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Inbound webhook handling for license server calls.
//!
//! The license server signs every request with Ed25519. The provisioner
//! verifies the signature, the request freshness (timestamp), and the
//! request uniqueness (nonce) before any business logic runs.
//!
//! Briefing-02c-2 backs the nonce check with a SQLite-persistent
//! [`crate::nonce_store::NonceStore`] so a process restart does not open a
//! replay window.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use ed25519_dalek::{Signature, Verifier};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{keys::KeyRegistry, nonce_store::NonceStore};

/// `X-Imogo-Timestamp` header name.
pub const HEADER_TIMESTAMP: &str = "x-imogo-timestamp";
/// `X-Imogo-Nonce` header name.
pub const HEADER_NONCE: &str = "x-imogo-nonce";
/// `X-Imogo-Signature` header name.
pub const HEADER_SIGNATURE: &str = "x-imogo-signature";
/// `X-Imogo-Key-Id` header name.
pub const HEADER_KEY_ID: &str = "x-imogo-key-id";

/// Errors returned by signature verification.
#[derive(Debug, Error)]
pub enum WebhookVerifyError {
    /// One of the four required signing headers was absent.
    #[error("missing required header: {0}")]
    MissingHeader(&'static str),

    /// A header was present but could not be parsed.
    #[error("malformed header value: {header}: {reason}")]
    MalformedHeader {
        /// Name of the offending header.
        header: &'static str,
        /// Human-readable parser explanation.
        reason: String,
    },

    /// Timestamp is older or further in the future than the configured skew.
    #[error("timestamp out of acceptable range")]
    TimestampOutOfRange,

    /// Same nonce was already accepted within the replay window.
    #[error("nonce already seen (replay)")]
    NonceReplay,

    /// `X-Imogo-Key-Id` did not match any registered public key.
    #[error("unknown key id")]
    UnknownKeyId,

    /// Cryptographic verification of the Ed25519 signature failed.
    #[error("signature verification failed")]
    BadSignature,

    /// The persistent nonce store returned a database error.
    #[error("nonce store error: {0}")]
    NonceStore(String),
}

/// Verifier holds the key registry, the persistent nonce store, and the
/// configured maximum clock skew. Cheap to clone (all fields are cloneable
/// handles).
#[derive(Clone)]
pub struct WebhookVerifier {
    keys: KeyRegistry,
    nonce_store: NonceStore,
    max_timestamp_skew_secs: i64,
}

impl std::fmt::Debug for WebhookVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookVerifier")
            .field("registered_keys", &self.keys.len())
            .field("max_timestamp_skew_secs", &self.max_timestamp_skew_secs)
            .finish_non_exhaustive()
    }
}

/// Output of a successful verification: the parsed timestamp and nonce, plus
/// the verified key id used. Useful for downstream auditing.
#[derive(Debug, Clone)]
pub struct VerifiedRequest {
    /// Key id that signed the verified request.
    pub key_id: String,
    /// Timestamp the sender claimed, in Unix seconds.
    pub timestamp_unix_seconds: i64,
    /// Nonce the sender attached to this request.
    pub nonce: String,
}

impl WebhookVerifier {
    /// Construct a new verifier.
    #[must_use]
    pub fn new(keys: KeyRegistry, nonce_store: NonceStore, max_timestamp_skew_secs: i64) -> Self {
        Self {
            keys,
            nonce_store,
            max_timestamp_skew_secs,
        }
    }

    /// Look up the key registry directly. Useful for tests.
    #[must_use]
    pub fn keys(&self) -> &KeyRegistry {
        &self.keys
    }

    /// Verify an inbound webhook.
    ///
    /// Inputs are everything we need from the HTTP layer: HTTP method, URL
    /// path (including query string), all four required headers, and the raw
    /// request body bytes.
    ///
    /// On success the nonce is recorded so subsequent identical requests are
    /// rejected as replays.
    ///
    /// # Errors
    ///
    /// Returns the specific [`WebhookVerifyError`] that caused the rejection.
    /// Callers should map all variants to HTTP 401, with logs for diagnostics.
    #[allow(clippy::too_many_arguments)]
    pub async fn verify(
        &self,
        method: &str,
        path_with_query: &str,
        timestamp_header: Option<&str>,
        nonce_header: Option<&str>,
        signature_header: Option<&str>,
        key_id_header: Option<&str>,
        body: &[u8],
    ) -> Result<VerifiedRequest, WebhookVerifyError> {
        let timestamp_str =
            timestamp_header.ok_or(WebhookVerifyError::MissingHeader(HEADER_TIMESTAMP))?;
        let nonce = nonce_header.ok_or(WebhookVerifyError::MissingHeader(HEADER_NONCE))?;
        let signature_str =
            signature_header.ok_or(WebhookVerifyError::MissingHeader(HEADER_SIGNATURE))?;
        let key_id = key_id_header.ok_or(WebhookVerifyError::MissingHeader(HEADER_KEY_ID))?;

        let timestamp =
            timestamp_str
                .parse::<i64>()
                .map_err(|e| WebhookVerifyError::MalformedHeader {
                    header: HEADER_TIMESTAMP,
                    reason: e.to_string(),
                })?;

        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| WebhookVerifyError::TimestampOutOfRange)?
                .as_secs(),
        )
        .map_err(|_| WebhookVerifyError::TimestampOutOfRange)?;

        if (now - timestamp).abs() > self.max_timestamp_skew_secs {
            return Err(WebhookVerifyError::TimestampOutOfRange);
        }

        let sig_bytes = STANDARD_NO_PAD.decode(signature_str).map_err(|e| {
            WebhookVerifyError::MalformedHeader {
                header: HEADER_SIGNATURE,
                reason: e.to_string(),
            }
        })?;
        let sig_array: [u8; 64] =
            sig_bytes
                .try_into()
                .map_err(|_| WebhookVerifyError::MalformedHeader {
                    header: HEADER_SIGNATURE,
                    reason: "signature must decode to 64 bytes".to_string(),
                })?;
        let signature = Signature::from_bytes(&sig_array);

        let registered = self
            .keys
            .lookup(key_id)
            .ok_or(WebhookVerifyError::UnknownKeyId)?;

        let mut hasher = Sha256::new();
        hasher.update(body);
        let body_hash_hex = hex::encode(hasher.finalize());

        let signing_string = build_signing_string(
            method,
            path_with_query,
            timestamp_str,
            nonce,
            &body_hash_hex,
        );

        registered
            .key
            .verify(signing_string.as_bytes(), &signature)
            .map_err(|_| WebhookVerifyError::BadSignature)?;

        // Insert nonce only after signature has been verified.
        let inserted = self
            .nonce_store
            .try_insert(nonce, key_id)
            .await
            .map_err(|e| WebhookVerifyError::NonceStore(e.to_string()))?;

        if !inserted {
            return Err(WebhookVerifyError::NonceReplay);
        }

        Ok(VerifiedRequest {
            key_id: key_id.to_string(),
            timestamp_unix_seconds: timestamp,
            nonce: nonce.to_string(),
        })
    }
}

/// Build the canonical string that gets signed by the license server.
///
/// Format (5 lines, newline-separated, ASCII-only):
///
/// ```text
/// <METHOD>
/// <PATH-WITH-QUERY>
/// <TIMESTAMP>
/// <NONCE>
/// <SHA-256-HEX>
/// ```
#[must_use]
pub fn build_signing_string(
    method: &str,
    path_with_query: &str,
    timestamp: &str,
    nonce: &str,
    body_sha256_hex: &str,
) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        path_with_query,
        timestamp,
        nonce,
        body_sha256_hex
    )
}
