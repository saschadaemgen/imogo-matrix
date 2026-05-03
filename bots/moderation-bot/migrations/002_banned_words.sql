-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Sascha Daemgen, IT and More Systems

CREATE TABLE IF NOT EXISTS moderation_banned_words (
    word TEXT PRIMARY KEY COLLATE NOCASE,
    added_by TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    match_mode TEXT NOT NULL DEFAULT 'substring',
    severity TEXT NOT NULL DEFAULT 'redact'
);
