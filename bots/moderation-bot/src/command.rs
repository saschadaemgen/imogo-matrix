// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Command parser for `!mod ...` instructions.
//!
//! Pure synchronous code, exhaustively unit-tested. The handler in
//! [`crate::handler`] uses [`parse`] to obtain a [`Command`] and dispatches
//! to the appropriate Matrix-SDK call.

use crate::{
    banned_words::{MatchMode, Severity},
    error::ModError,
};

/// All supported commands. The `!mod` prefix is stripped before parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `!mod aktivieren [note]`
    Activate { note: Option<String> },
    /// `!mod deaktivieren`
    Deactivate,
    /// `!mod status`
    Status,
    /// `!mod ban-word add <word> [mode] [severity]`
    BanWordAdd {
        word: String,
        mode: MatchMode,
        severity: Severity,
    },
    /// `!mod ban-word remove <word>`
    BanWordRemove { word: String },
    /// `!mod ban-word list`
    BanWordList,
    /// `!mod kick @user[:server] [reason]`
    Kick {
        user_id: String,
        reason: Option<String>,
    },
    /// `!mod ban @user[:server] [reason]`
    Ban {
        user_id: String,
        reason: Option<String>,
    },
    /// `!mod unban @user[:server]`
    Unban { user_id: String },
    /// `!mod mute @user[:server] <duration> [reason]`
    Mute {
        user_id: String,
        duration_secs: u64,
        reason: Option<String>,
    },
    /// `!mod unmute @user[:server]`
    Unmute { user_id: String },
    /// `!mod pin` (must be a reply to the target event)
    Pin,
    /// `!mod unpin` (must be a reply to the target event)
    Unpin,
    /// `!mod help`
    Help,
}

/// Parse a message body into a [`Command`].
///
/// Returns `Ok(None)` if the message is not a `!mod` command.
/// Returns `Ok(Some(cmd))` for a recognised command.
/// Returns `Err(ModError::InvalidCommand)` for a malformed `!mod` invocation.
///
/// # Errors
///
/// Returns [`ModError::InvalidCommand`] for syntactically invalid `!mod` input.
pub fn parse(body: &str) -> Result<Option<Command>, ModError> {
    let trimmed = body.trim_start();
    let Some(rest) = trimmed.strip_prefix("!mod") else {
        return Ok(None);
    };
    let rest = rest.trim();
    if rest.is_empty() {
        return Ok(Some(Command::Help));
    }

    parse_subcommand(rest).map(Some)
}

#[allow(clippy::too_many_lines)]
fn parse_subcommand(rest: &str) -> Result<Command, ModError> {
    let (head, tail) = split_first_word(rest);
    match head {
        "help" | "hilfe" => Ok(Command::Help),
        "aktivieren" | "activate" => {
            let note = if tail.is_empty() {
                None
            } else {
                Some(tail.to_string())
            };
            Ok(Command::Activate { note })
        }
        "deaktivieren" | "deactivate" => Ok(Command::Deactivate),
        "status" => Ok(Command::Status),
        "ban-word" => parse_ban_word(tail),
        "kick" => parse_kick_or_ban(tail).map(|(u, r)| Command::Kick {
            user_id: u,
            reason: r,
        }),
        "ban" => parse_kick_or_ban(tail).map(|(u, r)| Command::Ban {
            user_id: u,
            reason: r,
        }),
        "unban" => {
            let (user_id, leftover) = split_first_word(tail);
            if user_id.is_empty() || !leftover.is_empty() {
                Err(ModError::InvalidCommand(
                    "unban needs exactly one user id".into(),
                ))
            } else {
                Ok(Command::Unban {
                    user_id: user_id.to_string(),
                })
            }
        }
        "mute" => parse_mute(tail),
        "unmute" => {
            let (user_id, leftover) = split_first_word(tail);
            if user_id.is_empty() || !leftover.is_empty() {
                Err(ModError::InvalidCommand(
                    "unmute needs exactly one user id".into(),
                ))
            } else {
                Ok(Command::Unmute {
                    user_id: user_id.to_string(),
                })
            }
        }
        "pin" => Ok(Command::Pin),
        "unpin" => Ok(Command::Unpin),
        other => Err(ModError::InvalidCommand(format!(
            "unknown subcommand '{other}', try !mod help"
        ))),
    }
}

fn parse_ban_word(tail: &str) -> Result<Command, ModError> {
    let (action, rest) = split_first_word(tail);
    match action {
        "add" => {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.is_empty() {
                return Err(ModError::InvalidCommand("ban-word add needs a word".into()));
            }
            let word = parts[0].to_string();
            let mode = parts.get(1).map_or(Ok(MatchMode::Substring), |s| {
                s.parse::<MatchMode>().map_err(ModError::InvalidCommand)
            })?;
            let severity = parts.get(2).map_or(Ok(Severity::Redact), |s| {
                s.parse::<Severity>().map_err(ModError::InvalidCommand)
            })?;
            Ok(Command::BanWordAdd {
                word,
                mode,
                severity,
            })
        }
        "remove" | "rm" | "delete" => {
            let (word, leftover) = split_first_word(rest);
            if word.is_empty() || !leftover.is_empty() {
                Err(ModError::InvalidCommand(
                    "ban-word remove needs exactly one word".into(),
                ))
            } else {
                Ok(Command::BanWordRemove {
                    word: word.to_string(),
                })
            }
        }
        "list" | "ls" => Ok(Command::BanWordList),
        other => Err(ModError::InvalidCommand(format!(
            "ban-word: unknown action '{other}'"
        ))),
    }
}

fn parse_kick_or_ban(tail: &str) -> Result<(String, Option<String>), ModError> {
    let (user_id, rest) = split_first_word(tail);
    if user_id.is_empty() {
        return Err(ModError::InvalidCommand("kick/ban needs a user id".into()));
    }
    let reason = if rest.is_empty() {
        None
    } else {
        Some(strip_quotes(rest))
    };
    Ok((user_id.to_string(), reason))
}

fn parse_mute(tail: &str) -> Result<Command, ModError> {
    let (user_id, rest) = split_first_word(tail);
    if user_id.is_empty() {
        return Err(ModError::InvalidCommand("mute needs a user id".into()));
    }
    let (dur_str, rest) = split_first_word(rest);
    if dur_str.is_empty() {
        return Err(ModError::InvalidCommand(
            "mute needs a duration like 30s, 5m, 2h, 1d".into(),
        ));
    }
    let duration_secs = parse_duration_secs(dur_str)?;
    let reason = if rest.is_empty() {
        None
    } else {
        Some(strip_quotes(rest))
    };
    Ok(Command::Mute {
        user_id: user_id.to_string(),
        duration_secs,
        reason,
    })
}

/// Parse a duration string like `30s`, `5m`, `2h`, `1d` into seconds.
///
/// # Errors
///
/// Returns [`ModError::InvalidCommand`] if the format is wrong.
pub fn parse_duration_secs(s: &str) -> Result<u64, ModError> {
    if s.len() < 2 {
        return Err(ModError::InvalidCommand(format!(
            "invalid duration '{s}', expected like 30s, 5m, 2h, 1d"
        )));
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let n: u64 = num_str
        .parse()
        .map_err(|_| ModError::InvalidCommand(format!("invalid number in duration '{s}'")))?;
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        other => {
            return Err(ModError::InvalidCommand(format!(
                "unknown duration unit '{other}', use s/m/h/d"
            )));
        }
    };
    Ok(n.saturating_mul(mult))
}

/// Split `s` at the first whitespace. Returns `(head, tail)` where both are
/// trimmed. If `s` has no whitespace, `tail` is the empty string.
fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    s.split_once(char::is_whitespace)
        .map_or((s, ""), |(a, b)| (a, b.trim()))
}

/// Strip a single pair of surrounding double-quotes if present.
fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(body: &str) -> Command {
        parse(body).expect("parse").expect("not None")
    }

    #[test]
    fn non_command_returns_none() {
        assert!(parse("hello world").unwrap().is_none());
        assert!(parse("the !mod prefix in the middle").unwrap().is_none());
    }

    #[test]
    fn empty_command_is_help() {
        assert_eq!(parse_ok("!mod"), Command::Help);
        assert_eq!(parse_ok("!mod   "), Command::Help);
    }

    #[test]
    fn explicit_help_command() {
        assert_eq!(parse_ok("!mod help"), Command::Help);
        assert_eq!(parse_ok("!mod hilfe"), Command::Help);
    }

    #[test]
    fn activate_with_and_without_note() {
        assert_eq!(
            parse_ok("!mod aktivieren"),
            Command::Activate { note: None }
        );
        assert_eq!(
            parse_ok("!mod aktivieren community open"),
            Command::Activate {
                note: Some("community open".to_string())
            }
        );
    }

    #[test]
    fn deactivate_and_status() {
        assert_eq!(parse_ok("!mod deaktivieren"), Command::Deactivate);
        assert_eq!(parse_ok("!mod status"), Command::Status);
    }

    #[test]
    fn ban_word_add_with_defaults() {
        assert_eq!(
            parse_ok("!mod ban-word add foo"),
            Command::BanWordAdd {
                word: "foo".to_string(),
                mode: MatchMode::Substring,
                severity: Severity::Redact,
            }
        );
    }

    #[test]
    fn ban_word_add_with_mode_and_severity() {
        assert_eq!(
            parse_ok("!mod ban-word add hass whole_word kick"),
            Command::BanWordAdd {
                word: "hass".to_string(),
                mode: MatchMode::WholeWord,
                severity: Severity::Kick,
            }
        );
    }

    #[test]
    fn ban_word_invalid_severity() {
        let r = parse("!mod ban-word add foo substring strange");
        assert!(matches!(r, Err(ModError::InvalidCommand(_))));
    }

    #[test]
    fn ban_word_remove_and_list() {
        assert_eq!(
            parse_ok("!mod ban-word remove foo"),
            Command::BanWordRemove {
                word: "foo".to_string()
            }
        );
        assert_eq!(parse_ok("!mod ban-word list"), Command::BanWordList);
    }

    #[test]
    fn kick_and_ban_with_reason() {
        assert_eq!(
            parse_ok("!mod kick @bob:test \"spamming the room\""),
            Command::Kick {
                user_id: "@bob:test".to_string(),
                reason: Some("spamming the room".to_string()),
            }
        );
        assert_eq!(
            parse_ok("!mod ban @eve:test bot account"),
            Command::Ban {
                user_id: "@eve:test".to_string(),
                reason: Some("bot account".to_string()),
            }
        );
    }

    #[test]
    fn unban_and_unmute() {
        assert_eq!(
            parse_ok("!mod unban @bob:test"),
            Command::Unban {
                user_id: "@bob:test".to_string()
            }
        );
        assert_eq!(
            parse_ok("!mod unmute @bob:test"),
            Command::Unmute {
                user_id: "@bob:test".to_string()
            }
        );
    }

    #[test]
    fn mute_with_duration_and_reason() {
        assert_eq!(
            parse_ok("!mod mute @bob:test 5m abkühlung"),
            Command::Mute {
                user_id: "@bob:test".to_string(),
                duration_secs: 300,
                reason: Some("abkühlung".to_string()),
            }
        );
    }

    #[test]
    fn duration_units() {
        assert_eq!(parse_duration_secs("30s").unwrap(), 30);
        assert_eq!(parse_duration_secs("5m").unwrap(), 300);
        assert_eq!(parse_duration_secs("2h").unwrap(), 7_200);
        assert_eq!(parse_duration_secs("1d").unwrap(), 86_400);
        assert!(parse_duration_secs("5x").is_err());
        assert!(parse_duration_secs("notanumber").is_err());
    }

    #[test]
    fn pin_and_unpin() {
        assert_eq!(parse_ok("!mod pin"), Command::Pin);
        assert_eq!(parse_ok("!mod unpin"), Command::Unpin);
    }

    #[test]
    fn unknown_subcommand_errors() {
        let r = parse("!mod totally-unknown-thing");
        assert!(matches!(r, Err(ModError::InvalidCommand(_))));
    }
}
