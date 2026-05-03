// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Inbound webhook handling for license server calls.
//!
//! The license server signs every request with Ed25519. The provisioner
//! verifies the signature, the request freshness (timestamp), and the
//! request uniqueness (nonce) before any business logic runs.
//!
//! Briefing-02c-1 builds only the verification layer. Briefing-02c-3 will
//! plug actual business logic into the verified-request path.

use std::{
    num::NonZeroUsize,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use ed25519_dalek::{Signature, Verifier};
use lru::LruCache;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::keys::KeyRegistry;

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
}

/// Verifier holds the key registry, replay cache, and configuration.
#[derive(Clone)]
pub struct WebhookVerifier {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for WebhookVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookVerifier")
            .field(
                "max_timestamp_skew_secs",
                &self.inner.max_timestamp_skew_secs,
            )
            .field("registered_keys", &self.inner.keys.len())
            .finish()
    }
}

struct Inner {
    keys: KeyRegistry,
    nonce_cache: Mutex<LruCache<String, ()>>,
    max_timestamp_skew_secs: i64,
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
    ///
    /// `nonce_cache_capacity` is clamped to at least 1 internally; passing 0
    /// produces a 1-entry cache rather than an error.
    ///
    /// # Panics
    ///
    /// Does not panic at runtime: the internal `NonZeroUsize` construction
    /// is performed on `cap.max(1)`, which is always non-zero.
    #[must_use]
    pub fn new(
        keys: KeyRegistry,
        nonce_cache_capacity: usize,
        max_timestamp_skew_secs: i64,
    ) -> Self {
        let cap = NonZeroUsize::new(nonce_cache_capacity.max(1))
            .expect("nonce_cache_capacity max with 1 is always non-zero");
        Self {
            inner: Arc::new(Inner {
                keys,
                nonce_cache: Mutex::new(LruCache::new(cap)),
                max_timestamp_skew_secs,
            }),
        }
    }

    /// Look up the key registry directly. Useful for tests.
    #[must_use]
    pub fn keys(&self) -> &KeyRegistry {
        &self.inner.keys
    }

    /// Verify a request. Inputs are everything we need from the HTTP layer:
    /// HTTP method, URL path (including query string!), all four required
    /// headers, and the raw request body bytes.
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

        // Parse timestamp.
        let timestamp =
            timestamp_str
                .parse::<i64>()
                .map_err(|e| WebhookVerifyError::MalformedHeader {
                    header: HEADER_TIMESTAMP,
                    reason: e.to_string(),
                })?;

        // Check timestamp freshness against current time. Fitting Unix seconds
        // into i64 is safe until far past year 2038; cast is acceptable here.
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| WebhookVerifyError::TimestampOutOfRange)?
            .as_secs();
        let now = i64::try_from(now_secs).map_err(|_| WebhookVerifyError::TimestampOutOfRange)?;

        if (now - timestamp).abs() > self.inner.max_timestamp_skew_secs {
            return Err(WebhookVerifyError::TimestampOutOfRange);
        }

        // Parse signature.
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

        // Look up the key.
        let registered = self
            .inner
            .keys
            .lookup(key_id)
            .ok_or(WebhookVerifyError::UnknownKeyId)?;

        // Compute body hash.
        let mut hasher = Sha256::new();
        hasher.update(body);
        let body_hash_hex = hex::encode(hasher.finalize());

        // Build the canonical signing string.
        let signing_string = build_signing_string(
            method,
            path_with_query,
            timestamp_str,
            nonce,
            &body_hash_hex,
        );

        // Verify.
        registered
            .key
            .verify(signing_string.as_bytes(), &signature)
            .map_err(|_| WebhookVerifyError::BadSignature)?;

        // Insert nonce ONLY after signature has been verified, to avoid
        // attacker-induced cache pollution.
        let mut cache = self.inner.nonce_cache.lock().await;
        if cache.contains(nonce) {
            return Err(WebhookVerifyError::NonceReplay);
        }
        cache.put(nonce.to_string(), ());

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
