-- SPDX-License-Identifier: AGPL-3.0-or-later
-- Copyright (C) 2026 Sascha Daemgen, IT and More Systems

ALTER TABLE accounts ADD COLUMN state TEXT NOT NULL DEFAULT 'active';
ALTER TABLE accounts ADD COLUMN expired_at TEXT;
ALTER TABLE accounts ADD COLUMN deactivated_at TEXT;

CREATE INDEX IF NOT EXISTS idx_accounts_state ON accounts(state);
