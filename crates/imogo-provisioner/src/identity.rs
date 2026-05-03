// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Generators for stable Matrix UUIDs, display names, and initial passwords.

use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use data_encoding::BASE32_NOPAD;
use rand::{RngCore, rngs::OsRng};

/// 16 random bytes, base32-encoded (lowercase, no padding) yields 26 ASCII
/// chars. Forms the stable Matrix localpart of a customer account.
#[must_use]
pub fn generate_matrix_uuid() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    BASE32_NOPAD.encode(&bytes).to_ascii_lowercase()
}

/// First 8 chars of a Matrix UUID, used in the support room alias to keep
/// it short while still globally unique within practical limits.
#[must_use]
pub fn matrix_uuid_short(uuid: &str) -> String {
    uuid.chars().take(8).collect()
}

/// 32 random bytes, base64 (no padding). Used as the initial password
/// returned to the license server on activation. Never persisted.
#[must_use]
pub fn generate_initial_password() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    STANDARD_NO_PAD.encode(bytes)
}

/// Build the display name. Format depends on whether a company is given.
#[must_use]
pub fn build_display_name(person_name: &str, company: Option<&str>) -> String {
    match company {
        Some(c) if !c.is_empty() => format!("{person_name}, {c}"),
        _ => format!("{person_name} (imogo)"),
    }
}

/// Build the fully qualified Matrix user id.
#[must_use]
pub fn build_user_id(matrix_uuid: &str, server_name: &str) -> String {
    format!("@{matrix_uuid}:{server_name}")
}

/// Build the fully qualified support room alias.
#[must_use]
pub fn build_support_room_alias(matrix_uuid: &str, server_name: &str) -> String {
    let short = matrix_uuid_short(matrix_uuid);
    format!("#support-{short}:{server_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_uuid_is_26_chars_lowercase_alnum() {
        for _ in 0..50 {
            let uuid = generate_matrix_uuid();
            assert_eq!(uuid.len(), 26);
            assert!(
                uuid.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            );
        }
    }

    #[test]
    fn initial_passwords_are_distinct() {
        let a = generate_initial_password();
        let b = generate_initial_password();
        assert_ne!(a, b);
        assert!(a.len() >= 40);
    }

    #[test]
    fn display_name_with_company() {
        assert_eq!(
            build_display_name("Max Mustermann", Some("Mustermann GmbH")),
            "Max Mustermann, Mustermann GmbH"
        );
    }

    #[test]
    fn display_name_without_company() {
        assert_eq!(
            build_display_name("Max Mustermann", None),
            "Max Mustermann (imogo)"
        );
    }

    #[test]
    fn user_id_format() {
        assert_eq!(build_user_id("abc123", "imogo.de"), "@abc123:imogo.de");
    }

    #[test]
    fn support_room_alias_format() {
        assert_eq!(
            build_support_room_alias("abcdefghijkl", "imogo.de"),
            "#support-abcdefgh:imogo.de"
        );
    }
}
