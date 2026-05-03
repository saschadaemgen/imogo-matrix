-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Sascha Daemgen, IT and More Systems

CREATE TABLE IF NOT EXISTS moderation_audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,
    room_id TEXT,
    actor_user_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_user_id TEXT,
    target_event_id TEXT,
    payload_json TEXT NOT NULL,
    prev_hash TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE
);

CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON moderation_audit_log(timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_actor ON moderation_audit_log(actor_user_id);
CREATE INDEX IF NOT EXISTS idx_audit_target ON moderation_audit_log(target_user_id);
