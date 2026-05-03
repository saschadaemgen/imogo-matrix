// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Power-Level pure-helper tests. The async wrapper that talks to a real
//! homeserver is exercised manually in T04/T07/T09 against Tuwunel.

use std::collections::BTreeMap;

use matrix_sdk::ruma::{Int, owned_user_id};
use moderation_bot::power_level::power_level_for;

fn make_int(v: i64) -> Int {
    Int::try_from(v).expect("i64 fits in Int")
}

#[test]
fn admin_pl_returns_100() {
    let mut users = BTreeMap::new();
    let admin = owned_user_id!("@admin:test.local");
    users.insert(admin.clone(), make_int(100));
    assert_eq!(power_level_for(&users, make_int(0), &admin), 100);
}

#[test]
fn ordinary_user_uses_default() {
    let users = BTreeMap::new();
    let user = owned_user_id!("@bob:test.local");
    assert_eq!(power_level_for(&users, make_int(0), &user), 0);
}

#[test]
fn muted_user_returns_negative_one() {
    let mut users = BTreeMap::new();
    let muted = owned_user_id!("@muted:test.local");
    users.insert(muted.clone(), make_int(-1));
    assert_eq!(power_level_for(&users, make_int(0), &muted), -1);
}

#[test]
fn nonzero_default_is_respected_for_unknown_user() {
    let users = BTreeMap::new();
    let user = owned_user_id!("@x:test.local");
    assert_eq!(power_level_for(&users, make_int(50), &user), 50);
}
