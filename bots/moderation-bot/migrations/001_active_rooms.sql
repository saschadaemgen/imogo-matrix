-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Sascha Daemgen, IT and More Systems

CREATE TABLE IF NOT EXISTS moderation_active_rooms (
    room_id TEXT PRIMARY KEY,
    activated_by TEXT NOT NULL,
    activated_at INTEGER NOT NULL,
    note TEXT
);
