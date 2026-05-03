// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Public key registries for verifying inbound material from the imogo
//! license server.
//!
//! Two parallel registries:
//! - [`KeyRegistry`] holds keys that verify webhook signatures (Ed25519 over
//!   the canonical signing string from Briefing-02c-1).
//! - [`CapabilityKeyRegistry`] holds keys that verify capability JWTs
//!   (`EdDSA`, Briefing-02e). Operationally separate from webhook keys so
//!   rotation, key material, and trust scope can move independently.
//!
//! Each key has a stable identifier (`key_id`) so the license server can
//! announce which key signed a given request via the appropriate header
//! (`X-Imogo-Key-Id` for webhooks, `kid` JOSE header for capability JWTs).
//! This enables key rotation: during a rotation window both the old and
//! new key are valid.
//!
//! Production keys will be added here once the license server is built (see
//! Master-Briefing 17). The current entries are test keys gated behind the
//! `dev-keys` feature flag, so production builds cannot accept test
//! signatures.

use std::collections::BTreeMap;

use ed25519_dalek::VerifyingKey;

/// A registered public key, identified by its `key_id`.
#[derive(Clone, Debug)]
pub struct RegisteredKey {
    /// Stable identifier sent by the license server.
    pub key_id: &'static str,
    /// The Ed25519 verifying key.
    pub key: VerifyingKey,
    /// Free-form note for operators.
    pub note: &'static str,
}

/// Webhook signing-key registry.
#[derive(Clone, Debug, Default)]
#[allow(clippy::module_name_repetitions)]
pub struct KeyRegistry {
    keys: BTreeMap<String, RegisteredKey>,
}

impl KeyRegistry {
    /// Build the registry with all compiled-in webhook keys.
    #[must_use]
    pub fn with_compiled_in_keys() -> Self {
        // `mut` is only used when the `dev-keys` feature compiles in the
        // insertion below; without the feature this is an empty registry.
        #[cfg_attr(not(feature = "dev-keys"), allow(unused_mut))]
        let mut registry = Self::default();

        #[cfg(feature = "dev-keys")]
        {
            if let Some(k) = test_keys::license_server_dev_key() {
                registry.insert(k);
            } else {
                tracing::warn!(
                    "DEV_PUBLIC_KEY_BYTES placeholder is not a valid Ed25519 \
                     encoding; no dev key registered. Replace bytes or inject \
                     a key at runtime via KeyRegistry::insert."
                );
            }
        }

        registry
    }

    /// Add a key to the registry.
    pub fn insert(&mut self, key: RegisteredKey) {
        self.keys.insert(key.key_id.to_string(), key);
    }

    /// Look up a key by its `key_id`.
    #[must_use]
    pub fn lookup(&self, key_id: &str) -> Option<&RegisteredKey> {
        self.keys.get(key_id)
    }

    /// Number of registered keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// True if no keys are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Capability-token signing-key registry. Operationally separate from
/// [`KeyRegistry`] so the license server can rotate webhook and capability
/// keys independently.
#[derive(Clone, Debug, Default)]
#[allow(clippy::module_name_repetitions)]
pub struct CapabilityKeyRegistry {
    keys: BTreeMap<String, RegisteredKey>,
}

impl CapabilityKeyRegistry {
    /// Build the registry with all compiled-in capability keys.
    #[must_use]
    pub fn with_compiled_in_keys() -> Self {
        // `mut` is only used when the `dev-keys` feature compiles in the
        // insertion below; without the feature this is an empty registry.
        #[cfg_attr(not(feature = "dev-keys"), allow(unused_mut))]
        let mut registry = Self::default();

        #[cfg(feature = "dev-keys")]
        {
            if let Some(k) = test_keys::license_server_capability_dev_key() {
                registry.insert(k);
            } else {
                tracing::warn!(
                    "CAP_DEV_PUBLIC_KEY_BYTES placeholder is not a valid Ed25519 \
                     encoding; no dev capability key registered. Replace bytes or \
                     inject a key at runtime via CapabilityKeyRegistry::insert."
                );
            }
        }

        registry
    }

    /// Add a key to the registry.
    pub fn insert(&mut self, key: RegisteredKey) {
        self.keys.insert(key.key_id.to_string(), key);
    }

    /// Look up a key by its `key_id`.
    #[must_use]
    pub fn lookup(&self, key_id: &str) -> Option<&RegisteredKey> {
        self.keys.get(key_id)
    }

    /// Number of registered keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// True if no keys are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(feature = "dev-keys")]
pub mod test_keys {
    //! Test keys used in development and integration tests.
    //!
    //! Test keys must NEVER be used in production builds. The `dev-keys`
    //! cargo feature is the gate that prevents this.

    use ed25519_dalek::VerifyingKey;

    use super::RegisteredKey;

    /// Placeholder bytes for the dev license server webhook public key.
    ///
    /// These are NOT a valid pre-generated key; they are 32 random-looking
    /// bytes that may or may not be a valid Ed25519 point encoding. The
    /// integration test suite generates a fresh keypair at test time and
    /// patches the registry directly via `KeyRegistry::insert`.
    pub const DEV_PUBLIC_KEY_BYTES: [u8; 32] = [
        0x3a, 0x4f, 0x10, 0x9c, 0x8e, 0x42, 0xb7, 0xc1, 0x55, 0x91, 0xfd, 0x6e, 0x77, 0x2c, 0x84,
        0x18, 0xab, 0x3c, 0x6f, 0x90, 0xd2, 0x57, 0xee, 0x44, 0x09, 0x88, 0x71, 0xb6, 0x3a, 0x05,
        0x29, 0xf4,
    ];

    /// Placeholder bytes for the dev license server capability-token public key.
    /// Same disclaimer as `DEV_PUBLIC_KEY_BYTES`.
    pub const CAP_DEV_PUBLIC_KEY_BYTES: [u8; 32] = [
        0x6c, 0x82, 0x47, 0xb1, 0xa3, 0x59, 0xd1, 0xe7, 0x21, 0x3a, 0x9f, 0x40, 0x88, 0x12, 0x33,
        0x9e, 0x57, 0xc1, 0x0b, 0xfa, 0x44, 0x69, 0x2d, 0x8a, 0xb6, 0x12, 0x05, 0x77, 0xea, 0x91,
        0x3c, 0x68,
    ];

    /// Construct the dev license-server webhook [`RegisteredKey`].
    ///
    /// Returns `None` if `DEV_PUBLIC_KEY_BYTES` does not decode.
    #[must_use]
    pub fn license_server_dev_key() -> Option<RegisteredKey> {
        VerifyingKey::from_bytes(&DEV_PUBLIC_KEY_BYTES)
            .ok()
            .map(|key| RegisteredKey {
                key_id: "dev-license-server-2026",
                key,
                note: "Development-only webhook key. Never accept in production.",
            })
    }

    /// Construct the dev license-server capability-token [`RegisteredKey`].
    ///
    /// Returns `None` if `CAP_DEV_PUBLIC_KEY_BYTES` does not decode.
    #[must_use]
    pub fn license_server_capability_dev_key() -> Option<RegisteredKey> {
        VerifyingKey::from_bytes(&CAP_DEV_PUBLIC_KEY_BYTES)
            .ok()
            .map(|key| RegisteredKey {
                key_id: "license-server-cap-dev-2026",
                key,
                note: "Development-only capability-token key. Never accept in production.",
            })
    }
}
