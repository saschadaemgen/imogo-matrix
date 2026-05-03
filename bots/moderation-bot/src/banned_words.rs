// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Banned-word list and matcher.
//!
//! The list is stored in `moderation_banned_words` and held in an
//! `RwLock`-protected in-memory cache for fast matching on every incoming
//! message. The cache is refreshed on bot startup and after every
//! `ban-word add`/`remove` command.

use std::{fmt, str::FromStr, sync::Arc};

use chrono::Utc;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::error::ModError;

/// How a banned word matches against a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// Case-insensitive substring search.
    Substring,
    /// Case-insensitive whole-word match (`\b<word>\b` regex).
    WholeWord,
}

impl MatchMode {
    /// Stable lower-snake-case label used in the database.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Substring => "substring",
            Self::WholeWord => "whole_word",
        }
    }
}

impl FromStr for MatchMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "substring" => Ok(Self::Substring),
            "whole_word" => Ok(Self::WholeWord),
            other => Err(format!("invalid match_mode: {other}")),
        }
    }
}

impl fmt::Display for MatchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Severity of an automatic moderation action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Redact the offending message.
    Redact,
    /// Post a warning reply, leave the message intact.
    Warn,
    /// Kick the offending user from the room.
    Kick,
}

impl Severity {
    /// Stable lower-snake-case label used in the database.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Redact => "redact",
            Self::Warn => "warn",
            Self::Kick => "kick",
        }
    }
}

impl FromStr for Severity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "redact" => Ok(Self::Redact),
            "warn" => Ok(Self::Warn),
            "kick" => Ok(Self::Kick),
            other => Err(format!("invalid severity: {other}")),
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One banned word with its match mode and severity.
#[derive(Debug, Clone)]
pub struct BannedWord {
    /// The word (stored case-insensitively in the DB).
    pub word: String,
    /// How the word matches against incoming messages.
    pub match_mode: MatchMode,
    /// What action to take on a match.
    pub severity: Severity,
}

/// Hot-swappable in-memory cache of banned words.
#[derive(Clone, Default)]
pub struct WordCache {
    inner: Arc<RwLock<Vec<BannedWord>>>,
}

impl fmt::Debug for WordCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WordCache").finish_non_exhaustive()
    }
}

impl WordCache {
    /// Empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the cache contents from the database.
    ///
    /// # Errors
    ///
    /// Returns [`ModError::Db`] on database errors.
    pub async fn refresh(&self, pool: &SqlitePool) -> Result<(), ModError> {
        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT word, match_mode, severity FROM moderation_banned_words ORDER BY word ASC",
        )
        .fetch_all(pool)
        .await?;

        let mut words = Vec::with_capacity(rows.len());
        for (word, match_mode, severity) in rows {
            let mm = MatchMode::from_str(&match_mode)
                .map_err(|e| ModError::Db(sqlx::Error::Decode(e.into())))?;
            let sv = Severity::from_str(&severity)
                .map_err(|e| ModError::Db(sqlx::Error::Decode(e.into())))?;
            words.push(BannedWord {
                word,
                match_mode: mm,
                severity: sv,
            });
        }

        let mut guard = self.inner.write().await;
        *guard = words;
        Ok(())
    }

    /// Find the first banned word matching `message`, if any.
    pub async fn first_match(&self, message: &str) -> Option<BannedWord> {
        let guard = self.inner.read().await;
        first_match_in(&guard, message).cloned()
    }

    /// Number of cached words.
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    /// True if the cache is empty.
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }
}

/// Pure matching helper used by [`WordCache::first_match`] and exhaustively
/// covered by unit tests.
#[must_use]
pub fn first_match_in<'a>(words: &'a [BannedWord], message: &str) -> Option<&'a BannedWord> {
    let lower_msg = message.to_ascii_lowercase();
    words.iter().find(|w| matches_one(w, &lower_msg))
}

fn matches_one(w: &BannedWord, lower_msg: &str) -> bool {
    let lower_word = w.word.to_ascii_lowercase();
    if lower_word.is_empty() {
        return false;
    }
    match w.match_mode {
        MatchMode::Substring => lower_msg.contains(&lower_word),
        MatchMode::WholeWord => {
            let pattern = format!(r"\b{}\b", regex::escape(&lower_word));
            RegexBuilder::new(&pattern)
                .case_insensitive(true)
                .build()
                .map(|r| r.is_match(lower_msg))
                .unwrap_or(false)
        }
    }
}

/// Insert (or replace) a banned word in the database.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn add(
    pool: &SqlitePool,
    word: &str,
    added_by: &str,
    match_mode: MatchMode,
    severity: Severity,
) -> Result<(), ModError> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO moderation_banned_words (word, added_by, added_at, match_mode, severity) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(word) DO UPDATE SET \
             match_mode = excluded.match_mode, \
             severity = excluded.severity, \
             added_by = excluded.added_by, \
             added_at = excluded.added_at",
    )
    .bind(word)
    .bind(added_by)
    .bind(now)
    .bind(match_mode.as_str())
    .bind(severity.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove a banned word. No-op if missing.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn remove(pool: &SqlitePool, word: &str) -> Result<(), ModError> {
    sqlx::query("DELETE FROM moderation_banned_words WHERE word = ?")
        .bind(word)
        .execute(pool)
        .await?;
    Ok(())
}

/// Read all banned words ordered by word ascending.
///
/// # Errors
///
/// Returns [`ModError::Db`] on database errors.
pub async fn list(pool: &SqlitePool) -> Result<Vec<BannedWord>, ModError> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT word, match_mode, severity FROM moderation_banned_words ORDER BY word ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for (word, mm, sv) in rows {
        let match_mode =
            MatchMode::from_str(&mm).map_err(|e| ModError::Db(sqlx::Error::Decode(e.into())))?;
        let severity =
            Severity::from_str(&sv).map_err(|e| ModError::Db(sqlx::Error::Decode(e.into())))?;
        out.push(BannedWord {
            word,
            match_mode,
            severity,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(word: &str, mode: MatchMode, severity: Severity) -> BannedWord {
        BannedWord {
            word: word.to_string(),
            match_mode: mode,
            severity,
        }
    }

    #[test]
    fn substring_matches_case_insensitively() {
        let words = vec![w("spamtest", MatchMode::Substring, Severity::Redact)];
        assert!(first_match_in(&words, "Hey das ist ein SpamTest!").is_some());
        assert!(first_match_in(&words, "kein bann hier").is_none());
    }

    #[test]
    fn whole_word_does_not_match_substring() {
        let words = vec![w("test", MatchMode::WholeWord, Severity::Warn)];
        assert!(first_match_in(&words, "this is a test").is_some());
        assert!(first_match_in(&words, "tester").is_none());
        assert!(first_match_in(&words, "testing").is_none());
    }

    #[test]
    fn whole_word_handles_punctuation() {
        let words = vec![w("hello", MatchMode::WholeWord, Severity::Warn)];
        assert!(first_match_in(&words, "Hello!").is_some());
        assert!(first_match_in(&words, "(hello)").is_some());
        assert!(first_match_in(&words, "say-hello-world").is_some());
    }

    #[test]
    fn empty_word_does_not_match() {
        let words = vec![w("", MatchMode::Substring, Severity::Redact)];
        assert!(first_match_in(&words, "anything").is_none());
    }

    #[test]
    fn first_match_wins() {
        let words = vec![
            w("alpha", MatchMode::Substring, Severity::Warn),
            w("beta", MatchMode::Substring, Severity::Kick),
        ];
        let m = first_match_in(&words, "alphabet beta").unwrap();
        assert_eq!(m.word, "alpha");
    }

    #[test]
    fn match_mode_round_trip() {
        for s in ["substring", "whole_word"] {
            let m = MatchMode::from_str(s).unwrap();
            assert_eq!(m.as_str(), s);
        }
        assert!(MatchMode::from_str("invalid").is_err());
    }

    #[test]
    fn severity_round_trip() {
        for s in ["redact", "warn", "kick"] {
            let sv = Severity::from_str(s).unwrap();
            assert_eq!(sv.as_str(), s);
        }
        assert!(Severity::from_str("invalid").is_err());
    }
}
