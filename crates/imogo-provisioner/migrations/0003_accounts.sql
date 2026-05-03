-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Sascha Daemgen, IT and More Systems

CREATE TABLE IF NOT EXISTS accounts (
    license_id        TEXT PRIMARY KEY,
    matrix_uuid       TEXT NOT NULL UNIQUE,
    matrix_homeserver TEXT NOT NULL,
    matrix_user_id    TEXT NOT NULL UNIQUE,
    support_room_id   TEXT NOT NULL,
    display_name      TEXT NOT NULL,
    tier              TEXT NOT NULL,
    created_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_accounts_matrix_user_id ON accounts(matrix_user_id);
