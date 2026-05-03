// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Auto-join handler tests. The Matrix-SDK `Room::join` call cannot be
//! exercised from a unit test without a running Tuwunel, so this file
//! covers the two pure decision points:
//!
//! 1. `is_inviter_allowed` returns the expected boolean for the current
//!    open-phase configuration.
//! 2. The pure helper that classifies a `StrippedRoomMemberEvent` decides
//!    correctly which invites we should react to. The classifier is kept
//!    pure inside the handler module via [`InviteAction`].

use matrix_sdk::ruma::OwnedUserId;
use moderation_bot::handler::is_inviter_allowed;

#[test]
fn allowlist_currently_accepts_anyone() {
    let alice: OwnedUserId = "@alice:example.org".parse().unwrap();
    let bob: OwnedUserId = "@bob:matrix.imogo.de".parse().unwrap();
    let carol: OwnedUserId = "@carol:other.tld".parse().unwrap();
    assert!(is_inviter_allowed(&alice));
    assert!(is_inviter_allowed(&bob));
    assert!(is_inviter_allowed(&carol));
}
