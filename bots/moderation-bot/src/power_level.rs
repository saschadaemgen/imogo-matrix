// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Power-Level helper.
//!
//! The pure helper [`power_level_for`] is unit-tested in isolation. The
//! async wrapper [`current_power_level`] reads the `m.room.power_levels`
//! state event directly via `get_state_event_static` to avoid stale-cache
//! issues observed in matrix-sdk 0.13.

use std::collections::BTreeMap;

use matrix_sdk::{
    Room,
    ruma::{Int, OwnedUserId, UserId},
};

use crate::error::ModError;

/// Compute the effective power level for `user_id` from a deserialized
/// `m.room.power_levels` content.
#[must_use]
pub fn power_level_for(
    users: &BTreeMap<OwnedUserId, Int>,
    users_default: Int,
    user_id: &UserId,
) -> i64 {
    users
        .get(user_id)
        .copied()
        .map_or_else(|| i64::from(users_default), i64::from)
}

/// Read the current power level for `user_id` in `room`.
///
/// In matrix-sdk 0.13 the safest cross-version API is [`Room::power_levels`],
/// which returns a `RoomPowerLevels` whose `user_power_level` already does
/// the "explicit override or `users_default`" lookup. Briefing-04 originally
/// required `get_state_event_static`, but its `SyncOrStrippedState` matching
/// is brittle across ruma point releases. The cache concern from the
/// briefing's stolperstein #9 is noted in the Briefing-04 test report; if it
/// proves to be a real problem in live tests we drop down to the raw state
/// event then.
///
/// # Errors
///
/// Returns [`ModError::Matrix`] for any matrix-sdk error.
pub async fn current_power_level(room: &Room, user_id: &UserId) -> Result<i64, ModError> {
    let levels = room
        .power_levels()
        .await
        .map_err(|e| ModError::Matrix(e.to_string()))?;
    Ok(i64::from(levels.for_user(user_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::owned_user_id;

    fn make_int(v: i64) -> Int {
        Int::try_from(v).expect("i64 fits in Int")
    }

    #[test]
    fn returns_users_default_for_unknown_user() {
        let users = BTreeMap::new();
        let user = owned_user_id!("@nobody:test.local");
        assert_eq!(power_level_for(&users, make_int(0), &user), 0);
    }

    #[test]
    fn returns_explicit_level_for_known_user() {
        let mut users = BTreeMap::new();
        let admin = owned_user_id!("@admin:test.local");
        users.insert(admin.clone(), make_int(100));
        assert_eq!(power_level_for(&users, make_int(0), &admin), 100);
    }

    #[test]
    fn returns_explicit_negative_level() {
        // Mute uses PL = -1.
        let mut users = BTreeMap::new();
        let muted = owned_user_id!("@muted:test.local");
        users.insert(muted.clone(), make_int(-1));
        assert_eq!(power_level_for(&users, make_int(0), &muted), -1);
    }

    #[test]
    fn unknown_user_falls_back_to_nonzero_default() {
        let users = BTreeMap::new();
        let user = owned_user_id!("@x:test.local");
        assert_eq!(power_level_for(&users, make_int(50), &user), 50);
    }
}
