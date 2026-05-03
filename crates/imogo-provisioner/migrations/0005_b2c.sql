-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Sascha Daemgen, IT and More Systems

CREATE TABLE IF NOT EXISTS b2c_rooms (
    qr_token            TEXT PRIMARY KEY,
    handwerker_license  TEXT NOT NULL,
    handwerker_user_id  TEXT NOT NULL,
    invoice_number      TEXT NOT NULL,
    invoice_subject     TEXT NOT NULL,
    room_id             TEXT NOT NULL,
    room_alias          TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    next_guest_index    INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_b2c_rooms_handwerker ON b2c_rooms(handwerker_license);
CREATE INDEX IF NOT EXISTS idx_b2c_rooms_invoice ON b2c_rooms(invoice_number);
CREATE INDEX IF NOT EXISTS idx_b2c_rooms_expires ON b2c_rooms(expires_at);

CREATE TABLE IF NOT EXISTS b2c_guests (
    matrix_user_id   TEXT PRIMARY KEY,
    qr_token         TEXT NOT NULL,
    guest_index      INTEGER NOT NULL,
    created_at       TEXT NOT NULL,
    FOREIGN KEY (qr_token) REFERENCES b2c_rooms(qr_token)
);
CREATE INDEX IF NOT EXISTS idx_b2c_guests_qr_token ON b2c_guests(qr_token);

CREATE TABLE IF NOT EXISTS capability_jti_cache (
    jti          TEXT PRIMARY KEY,
    expires_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_capability_jti_expires ON capability_jti_cache(expires_at);
