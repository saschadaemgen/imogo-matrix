-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Sascha Daemgen, IT and More Systems

CREATE TABLE IF NOT EXISTS audit_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at   TEXT NOT NULL,
    event_type   TEXT NOT NULL,
    actor        TEXT NOT NULL,
    subject      TEXT,
    payload_json TEXT NOT NULL,
    prev_hash    TEXT NOT NULL,
    entry_hash   TEXT NOT NULL UNIQUE
);

CREATE INDEX IF NOT EXISTS idx_audit_log_created_at ON audit_log(created_at);
CREATE INDEX IF NOT EXISTS idx_audit_log_event_type ON audit_log(event_type);
CREATE INDEX IF NOT EXISTS idx_audit_log_subject ON audit_log(subject);
