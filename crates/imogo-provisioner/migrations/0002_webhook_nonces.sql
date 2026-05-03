-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Sascha Daemgen, IT and More Systems

CREATE TABLE IF NOT EXISTS webhook_nonces (
    nonce      TEXT PRIMARY KEY,
    key_id     TEXT NOT NULL,
    seen_at    TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_nonces_expires_at ON webhook_nonces(expires_at);
