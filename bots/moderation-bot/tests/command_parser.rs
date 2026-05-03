// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests for the `!mod` command parser. The parser itself has
//! exhaustive unit tests inside `command.rs`; this file covers a few
//! end-to-end shapes to detect regressions in module-boundary refactors.

use moderation_bot::{
    banned_words::{MatchMode, Severity},
    command::{self, Command, parse_duration_secs},
};

#[test]
fn full_workflow_commands_round_trip() {
    let activate = command::parse("!mod aktivieren community open")
        .unwrap()
        .unwrap();
    assert_eq!(
        activate,
        Command::Activate {
            note: Some("community open".to_string())
        }
    );

    let add = command::parse("!mod ban-word add hass whole_word kick")
        .unwrap()
        .unwrap();
    assert_eq!(
        add,
        Command::BanWordAdd {
            word: "hass".to_string(),
            mode: MatchMode::WholeWord,
            severity: Severity::Kick,
        }
    );

    let mute = command::parse("!mod mute @bob:test 1h abkuehlung")
        .unwrap()
        .unwrap();
    assert_eq!(
        mute,
        Command::Mute {
            user_id: "@bob:test".to_string(),
            duration_secs: 3600,
            reason: Some("abkuehlung".to_string()),
        }
    );
}

#[test]
fn duration_parser_edge_cases() {
    assert!(parse_duration_secs("0s").is_ok());
    assert_eq!(parse_duration_secs("0s").unwrap(), 0);
    assert!(parse_duration_secs("").is_err());
    assert!(parse_duration_secs("s").is_err());
    assert!(parse_duration_secs("99X").is_err());
}

#[test]
fn non_command_messages_are_none() {
    assert!(command::parse("guten morgen").unwrap().is_none());
    assert!(command::parse("").unwrap().is_none());
    assert!(
        command::parse("aber !mod kommt mitten im satz")
            .unwrap()
            .is_none()
    );
}
