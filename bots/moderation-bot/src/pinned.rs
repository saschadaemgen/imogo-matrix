// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Pin and unpin helpers operating on `m.room.pinned_events`.
//!
//! `apply_pin` reads the current pinned-event list via
//! `room.get_state_event_static`, mutates it, and writes the new list with
//! `room.send_state_event`. The function is idempotent: pinning an
//! already-pinned event is a no-op for the on-wire state but still produces
//! an audit entry, since the operator may want a record of the attempt.

use matrix_sdk::{
    Room,
    deserialized_responses::SyncOrStrippedState,
    ruma::{
        OwnedEventId,
        events::{SyncStateEvent, room::pinned_events::RoomPinnedEventsEventContent},
    },
};

use crate::error::ModError;

/// Read the current `m.room.pinned_events` content from the room state.
///
/// Returns an empty list if no such state event exists yet.
///
/// # Errors
///
/// Returns [`ModError::Matrix`] for matrix-sdk errors and JSON decode errors.
pub async fn read_pinned(room: &Room) -> Result<Vec<OwnedEventId>, ModError> {
    let raw_opt = room
        .get_state_event_static::<RoomPinnedEventsEventContent>()
        .await
        .map_err(|e| ModError::Matrix(e.to_string()))?;

    let Some(raw) = raw_opt else {
        return Ok(Vec::new());
    };

    match raw
        .deserialize()
        .map_err(|e| ModError::Matrix(e.to_string()))?
    {
        SyncOrStrippedState::Sync(SyncStateEvent::Original(orig)) => Ok(orig.content.pinned),
        // A redacted or stripped pinned-events state event reads as "no pins".
        SyncOrStrippedState::Sync(SyncStateEvent::Redacted(_))
        | SyncOrStrippedState::Stripped(_) => Ok(Vec::new()),
    }
}

/// Pin or unpin one event. Returns `(updated_list, changed)`. `changed` is
/// `true` when the resulting list differs from the input.
#[must_use]
pub fn toggle(
    current: &[OwnedEventId],
    target: &OwnedEventId,
    pin: bool,
) -> (Vec<OwnedEventId>, bool) {
    if pin {
        if current.iter().any(|e| e == target) {
            return (current.to_vec(), false);
        }
        let mut next = current.to_vec();
        next.push(target.clone());
        (next, true)
    } else {
        let next: Vec<OwnedEventId> = current.iter().filter(|e| *e != target).cloned().collect();
        let changed = next.len() != current.len();
        (next, changed)
    }
}

/// Read, mutate, and write the pinned-event list. `pin = true` adds, `pin
/// = false` removes. Returns `Ok(true)` when a state event was actually sent.
///
/// # Errors
///
/// Returns [`ModError::Matrix`] for state-read or state-send errors.
pub async fn apply_pin(room: &Room, target: &OwnedEventId, pin: bool) -> Result<bool, ModError> {
    let current = read_pinned(room).await?;
    let (next, changed) = toggle(&current, target, pin);
    if !changed {
        return Ok(false);
    }
    let content = RoomPinnedEventsEventContent::new(next);
    room.send_state_event(content)
        .await
        .map_err(|e| ModError::Matrix(e.to_string()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::owned_event_id;

    #[test]
    fn pin_into_empty_list() {
        let current = Vec::new();
        let target = owned_event_id!("$one:test");
        let (next, changed) = toggle(&current, &target, true);
        assert!(changed);
        assert_eq!(next, vec![target]);
    }

    #[test]
    fn pin_already_present_is_noop() {
        let target = owned_event_id!("$one:test");
        let current = vec![target.clone()];
        let (next, changed) = toggle(&current, &target, true);
        assert!(!changed);
        assert_eq!(next, current);
    }

    #[test]
    fn unpin_present_event_removes_it() {
        let target = owned_event_id!("$one:test");
        let other = owned_event_id!("$two:test");
        let current = vec![target.clone(), other.clone()];
        let (next, changed) = toggle(&current, &target, false);
        assert!(changed);
        assert_eq!(next, vec![other]);
    }

    #[test]
    fn unpin_missing_event_is_noop() {
        let target = owned_event_id!("$missing:test");
        let other = owned_event_id!("$two:test");
        let current = vec![other.clone()];
        let (next, changed) = toggle(&current, &target, false);
        assert!(!changed);
        assert_eq!(next, vec![other]);
    }
}
