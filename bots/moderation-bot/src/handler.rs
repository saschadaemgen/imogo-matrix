// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Matrix event handler. Wires together command parsing, power-level
//! enforcement, the banned-word matcher, the audit log, and the live mute
//! and pin coordinators in [`crate::mute`] and [`crate::pinned`].
//!
//! The handler logic is intentionally kept thin around the pure modules:
//! every code path here that hits matrix-sdk is verified against a real
//! Tuwunel during the live-test phase (L01-L07 of Briefing-04b).

use std::sync::Arc;

use matrix_sdk::{
    Client, Room,
    config::SyncSettings,
    ruma::{
        OwnedEventId, OwnedRoomId, OwnedUserId,
        events::room::{
            member::{MembershipState, StrippedRoomMemberEvent},
            message::{
                MessageType, OriginalSyncRoomMessageEvent, Relation, RoomMessageEventContent,
                SyncRoomMessageEvent,
            },
        },
    },
};
use regex::Regex;
use serde_json::json;
use sqlx::SqlitePool;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::{
    audit::{self, AuditEntry},
    banned_words::{self, BannedWord, Severity, WordCache},
    command::{self, Command},
    config::Config,
    error::ModError,
    mute, pinned, power_level, rooms,
};

/// Shared application state passed to the matrix-sdk event handler.
#[derive(Clone)]
pub struct BotState {
    /// Matrix client (kept here so background tasks can re-acquire `Room`s
    /// after their auto-unmute timer fires).
    pub client: Client,
    /// Database pool.
    pub pool: SqlitePool,
    /// Banned-word cache.
    pub word_cache: WordCache,
    /// Auto-discovery alias regex.
    pub alias_regex: Arc<RwLock<Regex>>,
    /// The bot's own user id (for self-message filtering).
    pub bot_user_id: OwnedUserId,
    /// Bot policy config.
    pub config: Arc<Config>,
}

impl std::fmt::Debug for BotState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotState")
            .field("bot_user_id", &self.bot_user_id)
            .finish_non_exhaustive()
    }
}

/// Inviter allowlist hook. Always returns `true` in this phase; future
/// briefings will replace this with a real allowlist (per-room or
/// per-homeserver).
#[must_use]
pub fn is_inviter_allowed(_inviter: &OwnedUserId) -> bool {
    // TODO: Allowlist (Briefing 05+)
    true
}

/// Run the bot: prime sync, register handlers, long-poll.
///
/// # Errors
///
/// Returns [`ModError::Matrix`] from the underlying sync.
pub async fn run(state: BotState) -> Result<(), ModError> {
    info!(user_id = state.bot_user_id.as_str(), "starting sync");

    let client = state.client.clone();

    // Step 1: sync_once without our handlers so we do not process old events.
    let initial_token = match client.sync_once(SyncSettings::default()).await {
        Ok(resp) => Some(resp.next_batch),
        Err(e) => {
            warn!(error = %e, "initial sync failed, continuing without skip-token");
            None
        }
    };

    // Step 2: auto-discovery for already-joined rooms.
    if let Err(e) = auto_discover(&client, &state).await {
        warn!(error = %e, "auto-discovery failed");
    }

    // Step 2b: process invites that arrived during the prime syncs and
    // would otherwise be missed by the StrippedRoomMemberEvent handler
    // (matrix-sdk only re-delivers invite_state on changes).
    for invited in client.invited_rooms() {
        let room_id = invited.room_id().to_owned();
        info!(room_id = %room_id, "processing pending invite from prime sync");

        // Synthesize the inviter from the invite-state member event.
        // Best-effort: if we cannot determine the inviter, accept anyway.
        // When the allowlist becomes real (Briefing 05+) the inviter
        // resolution must become more robust.
        let inviter = invited
            .invite_details()
            .await
            .ok()
            .and_then(|details| details.inviter.map(|m| m.user_id().to_owned()));

        if let Some(inviter_id) = inviter
            && !is_inviter_allowed(&inviter_id)
        {
            let _ = invited.leave().await;
            continue;
        }

        match invited.join().await {
            Ok(()) => {
                info!(room_id = %room_id, "pending invite accepted");
                let append_res = audit::append(
                    &state.pool,
                    AuditEntry::now(
                        Some(room_id.to_string()),
                        state.bot_user_id.to_string(),
                        "room_invite_accepted".to_string(),
                        None,
                        None,
                        json!({ "source": "prime_sync_replay" }),
                    ),
                )
                .await;
                if let Err(e) = append_res {
                    warn!(error = %e, "audit append for prime-sync replay failed");
                }

                // Run alias-based auto-discovery for this newly joined room.
                let regex = state.alias_regex.read().await.clone();
                if let Err(e) = check_alias_and_record(&state, &invited, &regex).await {
                    warn!(error = %e, room_id = %room_id, "post-replay auto-discovery failed");
                }
            }
            Err(e) => {
                warn!(error = %e, room_id = %room_id, "pending invite join failed");
            }
        }
    }

    // Step 3: register the auto-join handler.
    let state_for_invite = state.clone();
    client.add_event_handler(move |event: StrippedRoomMemberEvent, room: Room| {
        let s = state_for_invite.clone();
        async move {
            if let Err(e) = on_room_invite(s, event, room).await {
                error!(error = %e, "invite handler failed");
            }
        }
    });

    // Step 4: register the message handler.
    let state_for_msg = state.clone();
    client.add_event_handler(move |event: SyncRoomMessageEvent, room: Room| {
        let s = state_for_msg.clone();
        async move {
            let SyncRoomMessageEvent::Original(original) = event else {
                return;
            };
            if let Err(e) = on_message(s, original, room).await {
                error!(error = %e, "message handler failed");
            }
        }
    });

    // Step 5: long-poll.
    let settings = if let Some(t) = initial_token {
        SyncSettings::default().token(t)
    } else {
        SyncSettings::default()
    };
    client
        .sync(settings)
        .await
        .map_err(|e| ModError::Matrix(e.to_string()))?;

    Ok(())
}

async fn auto_discover(client: &Client, state: &BotState) -> Result<(), ModError> {
    let regex = state.alias_regex.read().await.clone();
    let joined = client.joined_rooms();
    let mut new_count = 0;
    for room in joined {
        if let Some(c) = check_alias_and_record(state, &room, &regex).await? {
            new_count += c;
        }
    }
    info!(new_count, "auto-discovery complete");
    Ok(())
}

/// Returns `Some(1)` if the room was newly inserted into
/// `moderation_active_rooms`, `Some(0)` if it matched but was already
/// present, and `None` if the alias did not match.
async fn check_alias_and_record(
    state: &BotState,
    room: &Room,
    regex: &Regex,
) -> Result<Option<i32>, ModError> {
    let Some(alias) = room.canonical_alias() else {
        return Ok(None);
    };
    if !regex.is_match(alias.as_str()) {
        return Ok(None);
    }
    let room_id = room.room_id().to_string();
    let inserted = rooms::insert_if_absent(
        &state.pool,
        &room_id,
        state.bot_user_id.as_str(),
        Some("auto-discovered"),
    )
    .await?;
    if inserted {
        audit::append(
            &state.pool,
            AuditEntry::now(
                Some(room_id),
                state.bot_user_id.to_string(),
                "auto_discovered".to_string(),
                None,
                None,
                json!({ "alias": alias.as_str() }),
            ),
        )
        .await?;
        Ok(Some(1))
    } else {
        Ok(Some(0))
    }
}

async fn on_room_invite(
    state: BotState,
    event: StrippedRoomMemberEvent,
    room: Room,
) -> Result<(), ModError> {
    // The invite event is sent to every room participant; we only act on the
    // one that targets the bot itself.
    if event.state_key != state.bot_user_id {
        return Ok(());
    }
    if !matches!(event.content.membership, MembershipState::Invite) {
        return Ok(());
    }

    let inviter = event.sender;
    let room_id = room.room_id().to_owned();

    if !is_inviter_allowed(&inviter) {
        if let Err(e) = room.leave().await {
            warn!(error = %e, "leave on rejected invite failed");
        }
        audit::append(
            &state.pool,
            AuditEntry::now(
                Some(room_id.to_string()),
                state.bot_user_id.to_string(),
                "room_invite_rejected".to_string(),
                None,
                None,
                json!({ "inviter": inviter.to_string() }),
            ),
        )
        .await?;
        info!(room_id = %room_id, inviter = %inviter, "room invite rejected (inviter not allowed)");
        return Ok(());
    }

    match room.join().await {
        Ok(()) => {
            info!(room_id = %room_id, inviter = %inviter, "room invite accepted");
            audit::append(
                &state.pool,
                AuditEntry::now(
                    Some(room_id.to_string()),
                    state.bot_user_id.to_string(),
                    "room_invite_accepted".to_string(),
                    None,
                    None,
                    json!({ "inviter": inviter.to_string() }),
                ),
            )
            .await?;

            let regex = state.alias_regex.read().await.clone();
            if let Ok(Some(1)) = check_alias_and_record(&state, &room, &regex).await {
                info!(room_id = %room_id, "auto-discovered after invite");
                audit::append(
                    &state.pool,
                    AuditEntry::now(
                        Some(room_id.to_string()),
                        state.bot_user_id.to_string(),
                        "auto_discovered_after_invite".to_string(),
                        None,
                        None,
                        json!({}),
                    ),
                )
                .await?;
            }
        }
        Err(e) => {
            warn!(error = %e, room_id = %room_id, "join after invite failed");
            audit::append(
                &state.pool,
                AuditEntry::now(
                    Some(room_id.to_string()),
                    state.bot_user_id.to_string(),
                    "room_invite_join_failed".to_string(),
                    None,
                    None,
                    json!({
                        "inviter": inviter.to_string(),
                        "error": e.to_string(),
                    }),
                ),
            )
            .await?;
        }
    }

    Ok(())
}

async fn on_message(
    state: BotState,
    event: OriginalSyncRoomMessageEvent,
    room: Room,
) -> Result<(), ModError> {
    if event.sender == state.bot_user_id {
        return Ok(());
    }

    let MessageType::Text(ref text_content) = event.content.msgtype else {
        return Ok(());
    };
    let body = text_content.body.clone();
    let room_id = room.room_id().to_owned();

    // Auto-moderation runs only in active rooms and only on non-admin users.
    if rooms::is_active(&state.pool, room_id.as_str()).await? {
        run_auto_moderation(&state, &event, &room, &body).await?;
    }

    // Command dispatch (on `!mod` messages).
    if let Some(cmd) = command::parse(&body)? {
        dispatch_command(&state, &event, &room, cmd).await?;
    }

    Ok(())
}

async fn run_auto_moderation(
    state: &BotState,
    event: &OriginalSyncRoomMessageEvent,
    room: &Room,
    body: &str,
) -> Result<(), ModError> {
    let Some(matched): Option<BannedWord> = state.word_cache.first_match(body).await else {
        return Ok(());
    };

    // Admin shield: do not moderate users with PL >= 100.
    let sender_pl = power_level::current_power_level(room, &event.sender).await?;
    if sender_pl >= 100 {
        debug!("auto-moderation skipped: sender is admin");
        return Ok(());
    }

    let room_id = room.room_id().to_string();
    let action_label = match matched.severity {
        Severity::Redact => {
            if let Err(e) = room
                .redact(&event.event_id, Some("banned word"), None)
                .await
            {
                warn!(error = %e, "redact failed");
            }
            "auto_moderation_redact"
        }
        Severity::Warn => {
            let warn_text = "Bitte beachte unsere Community-Regeln.";
            let content = RoomMessageEventContent::text_plain(warn_text);
            if let Err(e) = room.send(content).await {
                warn!(error = %e, "warn message failed");
            }
            "auto_moderation_warn"
        }
        Severity::Kick => {
            if let Err(e) = room.kick_user(&event.sender, Some("banned word")).await {
                warn!(error = %e, "kick failed");
            }
            "auto_moderation_kick"
        }
    };

    audit::append(
        &state.pool,
        AuditEntry::now(
            Some(room_id),
            state.bot_user_id.to_string(),
            action_label.to_string(),
            Some(event.sender.to_string()),
            Some(event.event_id.to_string()),
            json!({
                "word": matched.word,
                "match_mode": matched.match_mode.as_str(),
                "severity": matched.severity.as_str(),
            }),
        ),
    )
    .await?;

    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn dispatch_command(
    state: &BotState,
    event: &OriginalSyncRoomMessageEvent,
    room: &Room,
    cmd: Command,
) -> Result<(), ModError> {
    let room_id = room.room_id().to_owned();
    let actor = event.sender.to_string();

    // Power-level gate. The required threshold depends on the command.
    let required_pl = required_pl_for(&state.config, &cmd);
    let sender_pl = power_level::current_power_level(room, &event.sender).await?;
    if sender_pl < required_pl {
        return deny_low_power_level(state, room, &room_id, &actor, &cmd).await;
    }

    // Some commands also need the bot itself to have admin power.
    if cmd_needs_bot_power(&cmd) {
        let bot_pl = power_level::current_power_level(room, &state.bot_user_id).await?;
        if bot_pl < 50 {
            let msg = "Ich brauche selbst Power Level 50 oder hoeher in diesem Raum, \
                       bitte vom Raum-Admin setzen lassen.";
            let _ = room.send(RoomMessageEventContent::text_plain(msg)).await;
            return Ok(());
        }
    }

    // Some commands only make sense in an active room.
    if cmd_needs_active_room(&cmd) && !rooms::is_active(&state.pool, room_id.as_str()).await? {
        let msg = "Dieser Raum ist noch nicht aktiviert. Nutze !mod aktivieren zuerst.";
        let _ = room.send(RoomMessageEventContent::text_plain(msg)).await;
        return Ok(());
    }

    match cmd {
        Command::Help => {
            let _ = room
                .send(RoomMessageEventContent::text_markdown(help_text()))
                .await;
        }
        Command::Activate { note } => {
            rooms::activate(&state.pool, room_id.as_str(), &actor, note.as_deref()).await?;
            audit_simple(
                state,
                &room_id,
                &actor,
                "room_activated",
                json!({ "note": note }),
            )
            .await?;
            ack(room, "Raum aktiviert. Bot moderiert ab jetzt.").await;
        }
        Command::Deactivate => {
            rooms::deactivate(&state.pool, room_id.as_str()).await?;
            audit_simple(state, &room_id, &actor, "room_deactivated", json!({})).await?;
            ack(
                room,
                "Raum deaktiviert. Bot ignoriert Befehle in diesem Raum bis zur Reaktivierung.",
            )
            .await;
        }
        Command::Status => {
            let active = rooms::is_active(&state.pool, room_id.as_str()).await?;
            let count = state.word_cache.len().await;
            let msg = format!("Status: aktiv = {active}, Bann-Woerter = {count}.");
            ack(room, &msg).await;
        }
        Command::BanWordAdd {
            word,
            mode,
            severity,
        } => {
            banned_words::add(&state.pool, &word, &actor, mode, severity).await?;
            state.word_cache.refresh(&state.pool).await?;
            audit_simple(
                state,
                &room_id,
                &actor,
                "banned_word_added",
                json!({
                    "word": word,
                    "mode": mode.as_str(),
                    "severity": severity.as_str(),
                }),
            )
            .await?;
            ack(room, &format!("Bann-Wort '{word}' hinzugefuegt.")).await;
        }
        Command::BanWordRemove { word } => {
            banned_words::remove(&state.pool, &word).await?;
            state.word_cache.refresh(&state.pool).await?;
            audit_simple(
                state,
                &room_id,
                &actor,
                "banned_word_removed",
                json!({ "word": word }),
            )
            .await?;
            ack(room, &format!("Bann-Wort '{word}' entfernt.")).await;
        }
        Command::BanWordList => {
            let list = banned_words::list(&state.pool).await?;
            let msg = format_banned_words(&list);
            ack(room, &msg).await;
        }
        Command::Kick { user_id, reason } => {
            handle_user_action(
                state,
                room,
                &room_id,
                &actor,
                &user_id,
                reason.as_deref(),
                UserAction::Kick,
            )
            .await?;
        }
        Command::Ban { user_id, reason } => {
            handle_user_action(
                state,
                room,
                &room_id,
                &actor,
                &user_id,
                reason.as_deref(),
                UserAction::Ban,
            )
            .await?;
        }
        Command::Unban { user_id } => {
            handle_user_action(
                state,
                room,
                &room_id,
                &actor,
                &user_id,
                Some("manual unban"),
                UserAction::Unban,
            )
            .await?;
        }
        Command::Mute {
            user_id,
            duration_secs,
            reason,
        } => {
            handle_mute(state, room, &actor, &user_id, duration_secs, reason).await?;
        }
        Command::Unmute { user_id } => {
            handle_unmute(state, room, &actor, &user_id).await?;
        }
        Command::Pin => {
            handle_pin_or_unpin(state, event, room, &actor, true).await?;
        }
        Command::Unpin => {
            handle_pin_or_unpin(state, event, room, &actor, false).await?;
        }
    }

    Ok(())
}

fn required_pl_for(cfg: &Config, cmd: &Command) -> i64 {
    match cmd {
        Command::Help | Command::Status => 0,
        Command::Activate { .. } | Command::Deactivate => cfg.bot.pl_word_admin,
        Command::BanWordAdd { .. } | Command::BanWordRemove { .. } | Command::BanWordList => {
            cfg.bot.pl_word_admin
        }
        Command::Kick { .. } => cfg.bot.pl_kick,
        Command::Ban { .. } | Command::Unban { .. } => cfg.bot.pl_ban,
        Command::Mute { .. } | Command::Unmute { .. } => cfg.bot.pl_mute,
        Command::Pin | Command::Unpin => cfg.bot.pl_pin,
    }
}

fn cmd_needs_bot_power(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Kick { .. }
            | Command::Ban { .. }
            | Command::Unban { .. }
            | Command::Mute { .. }
            | Command::Unmute { .. }
            | Command::Pin
            | Command::Unpin
            | Command::Activate { .. }
    )
}

fn cmd_needs_active_room(cmd: &Command) -> bool {
    !matches!(
        cmd,
        Command::Activate { .. } | Command::Help | Command::Status | Command::Deactivate
    )
}

async fn deny_low_power_level(
    state: &BotState,
    room: &Room,
    room_id: &OwnedRoomId,
    actor: &str,
    cmd: &Command,
) -> Result<(), ModError> {
    let msg = "Du hast nicht das noetige Power Level fuer diesen Befehl.";
    let _ = room.send(RoomMessageEventContent::text_plain(msg)).await;
    audit_simple(
        state,
        room_id,
        actor,
        "command_denied_power_level",
        json!({ "command": format!("{cmd:?}") }),
    )
    .await
}

async fn audit_simple(
    state: &BotState,
    room_id: &OwnedRoomId,
    actor: &str,
    action: &str,
    payload: serde_json::Value,
) -> Result<(), ModError> {
    audit::append(
        &state.pool,
        AuditEntry::now(
            Some(room_id.to_string()),
            actor.to_string(),
            action.to_string(),
            None,
            None,
            payload,
        ),
    )
    .await
    .map(|_| ())
}

async fn ack(room: &Room, text: &str) {
    let _ = room.send(RoomMessageEventContent::text_plain(text)).await;
}

/// Discriminator for the three "act on a user" commands. Replaces the
/// closure-based dispatch from an earlier draft (which ran into closure-
/// capture vs. argument-borrow conflicts on `reason`).
enum UserAction {
    Kick,
    Ban,
    Unban,
}

impl UserAction {
    fn audit_label(&self) -> &'static str {
        match self {
            Self::Kick => "user_kicked",
            Self::Ban => "user_banned",
            Self::Unban => "user_unbanned",
        }
    }
}

async fn handle_user_action(
    state: &BotState,
    room: &Room,
    room_id: &OwnedRoomId,
    actor: &str,
    target_user_id: &str,
    reason: Option<&str>,
    action: UserAction,
) -> Result<(), ModError> {
    let target = OwnedUserId::try_from(target_user_id.to_string())
        .map_err(|e| ModError::InvalidCommand(format!("invalid user id: {e}")))?;

    let result = match action {
        UserAction::Kick => room.kick_user(&target, reason).await,
        UserAction::Ban => room.ban_user(&target, reason).await,
        UserAction::Unban => room.unban_user(&target, reason).await,
    };

    if let Err(e) = result {
        warn!(error = %e, "user action failed");
        ack(room, &format!("Aktion fehlgeschlagen: {e}")).await;
        return Ok(());
    }

    let action_label = action.audit_label();
    audit::append(
        &state.pool,
        AuditEntry::now(
            Some(room_id.to_string()),
            actor.to_string(),
            action_label.to_string(),
            Some(target.to_string()),
            None,
            json!({ "reason": reason }),
        ),
    )
    .await?;
    ack(room, &format!("{action_label} ausgefuehrt fuer {target}.")).await;
    Ok(())
}

async fn handle_mute(
    state: &BotState,
    room: &Room,
    actor: &str,
    target_user_id: &str,
    duration_secs: u64,
    reason: Option<String>,
) -> Result<(), ModError> {
    let target = OwnedUserId::try_from(target_user_id.to_string())
        .map_err(|e| ModError::InvalidCommand(format!("invalid user id: {e}")))?;

    if duration_secs == 0 {
        ack(room, "Dauer muss groesser als 0 sein.").await;
        return Ok(());
    }
    let max = state.config.bot.max_mute_seconds;
    if duration_secs > max {
        ack(
            room,
            &format!("Dauer ueberschreitet das Maximum von {max} Sekunden."),
        )
        .await;
        return Ok(());
    }

    match mute::apply_mute(
        &state.pool,
        &state.client,
        room,
        actor,
        &target,
        duration_secs,
        reason.as_deref(),
    )
    .await
    {
        Ok(expires_at) => {
            ack(
                room,
                &format!(
                    "Mute fuer {target} fuer {duration_secs}s gesetzt. \
                     Auto-Unmute bei Unix-Sekunde {expires_at}."
                ),
            )
            .await;
        }
        Err(e) => {
            warn!(error = %e, target = %target, "mute failed");
            ack(room, &format!("Mute fehlgeschlagen: {e}")).await;
        }
    }
    Ok(())
}

async fn handle_unmute(
    state: &BotState,
    room: &Room,
    actor: &str,
    target_user_id: &str,
) -> Result<(), ModError> {
    let target = OwnedUserId::try_from(target_user_id.to_string())
        .map_err(|e| ModError::InvalidCommand(format!("invalid user id: {e}")))?;

    let previous_pl = lookup_previous_pl(&state.pool, room.room_id().as_str(), target.as_str())
        .await
        .unwrap_or(0);

    match mute::apply_unmute(&state.pool, room, actor, &target, previous_pl, false).await {
        Ok(()) => {
            ack(
                room,
                &format!("Unmute fuer {target} ausgefuehrt (PL {previous_pl} wiederhergestellt)."),
            )
            .await;
        }
        Err(e) => {
            warn!(error = %e, target = %target, "unmute failed");
            ack(room, &format!("Unmute fehlgeschlagen: {e}")).await;
        }
    }
    Ok(())
}

/// Look at the audit log for the most recent open mute that matches
/// `(room_id, target_user_id)` and return its `previous_power_level`. If no
/// open mute is found, returns `None` and the caller can fall back to 0.
async fn lookup_previous_pl(pool: &SqlitePool, room_id: &str, target_user_id: &str) -> Option<i64> {
    let open = audit::find_open_mutes(pool).await.ok()?;
    open.into_iter()
        .rfind(|m| m.room_id == room_id && m.target_user_id == target_user_id)
        .map(|m| m.previous_power_level)
}

async fn handle_pin_or_unpin(
    state: &BotState,
    event: &OriginalSyncRoomMessageEvent,
    room: &Room,
    actor: &str,
    pin: bool,
) -> Result<(), ModError> {
    let Some(target) = extract_reply_target(event) else {
        ack(
            room,
            "Bitte als Reply auf eine Nachricht senden, deren Pin-Zustand sich aendern soll.",
        )
        .await;
        audit_simple(
            state,
            &room.room_id().to_owned(),
            actor,
            "pin_no_reply",
            json!({ "pin": pin }),
        )
        .await?;
        return Ok(());
    };

    let action_label = if pin {
        "event_pinned"
    } else {
        "event_unpinned"
    };
    match pinned::apply_pin(room, &target, pin).await {
        Ok(true) => {
            audit::append(
                &state.pool,
                AuditEntry::now(
                    Some(room.room_id().to_string()),
                    actor.to_string(),
                    action_label.to_string(),
                    None,
                    Some(target.to_string()),
                    json!({ "pin": pin }),
                ),
            )
            .await?;
            let verb = if pin { "gepinnt" } else { "entpinnt" };
            ack(room, &format!("Nachricht {verb}.")).await;
        }
        Ok(false) => {
            let verb = if pin {
                "bereits gepinnt"
            } else {
                "nicht gepinnt"
            };
            ack(room, &format!("Kein Zustandswechsel: Nachricht {verb}.")).await;
            audit::append(
                &state.pool,
                AuditEntry::now(
                    Some(room.room_id().to_string()),
                    actor.to_string(),
                    format!("{action_label}_noop"),
                    None,
                    Some(target.to_string()),
                    json!({ "pin": pin }),
                ),
            )
            .await?;
        }
        Err(e) => {
            warn!(error = %e, "pin/unpin failed");
            ack(room, &format!("Pin/Unpin fehlgeschlagen: {e}")).await;
        }
    }
    Ok(())
}

fn extract_reply_target(event: &OriginalSyncRoomMessageEvent) -> Option<OwnedEventId> {
    match &event.content.relates_to {
        Some(Relation::Reply { in_reply_to }) => Some(in_reply_to.event_id.clone()),
        _ => None,
    }
}

fn help_text() -> String {
    "**imogo-Moderations-Bot Befehle**\n\n\
     - `!mod aktivieren [note]` - Bot in diesem Raum aktivieren\n\
     - `!mod deaktivieren` - Bot deaktivieren\n\
     - `!mod status` - Aktivierungs-Status\n\
     - `!mod ban-word add <wort> [substring|whole_word] [redact|warn|kick]`\n\
     - `!mod ban-word remove <wort>`\n\
     - `!mod ban-word list`\n\
     - `!mod kick @user[:server] [reason]`\n\
     - `!mod ban @user[:server] [reason]`\n\
     - `!mod unban @user[:server]`\n\
     - `!mod mute @user[:server] <dauer> [reason]` (z.B. 30s, 5m, 2h, 1d)\n\
     - `!mod unmute @user[:server]`\n\
     - `!mod pin` (als Reply auf eine Nachricht)\n\
     - `!mod unpin` (als Reply auf eine Nachricht)\n\
     - `!mod help`"
        .to_string()
}

fn format_banned_words(list: &[BannedWord]) -> String {
    use std::fmt::Write as _;
    if list.is_empty() {
        return "Keine Bann-Woerter konfiguriert.".to_string();
    }
    let mut out = String::from("**Bann-Woerter:**\n");
    for (i, bw) in list.iter().enumerate() {
        if i >= 50 {
            out.push_str("(Liste gekuerzt nach 50 Eintraegen.)\n");
            break;
        }
        let _ = writeln!(out, "- `{}` ({}, {})", bw.word, bw.match_mode, bw.severity);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn required_pl_per_command() {
        let cfg = Config::default();
        assert_eq!(required_pl_for(&cfg, &Command::Help), 0);
        assert_eq!(required_pl_for(&cfg, &Command::Status), 0);
        assert_eq!(
            required_pl_for(&cfg, &Command::Activate { note: None }),
            cfg.bot.pl_word_admin
        );
        assert_eq!(required_pl_for(&cfg, &Command::Pin), cfg.bot.pl_pin);
        assert_eq!(
            required_pl_for(
                &cfg,
                &Command::Kick {
                    user_id: "@x:y".into(),
                    reason: None
                }
            ),
            cfg.bot.pl_kick
        );
    }

    #[test]
    fn help_text_has_no_em_dash() {
        let h = help_text();
        assert!(!h.contains('—'));
        assert!(h.contains("aktivieren"));
        assert!(h.contains("ban-word"));
        assert!(h.contains("mute"));
    }

    #[test]
    fn format_banned_words_empty_and_full() {
        let empty: Vec<BannedWord> = Vec::new();
        let s = format_banned_words(&empty);
        assert!(s.contains("Keine Bann-Woerter"));

        let mut list = Vec::new();
        for i in 0..3 {
            list.push(BannedWord {
                word: format!("word{i}"),
                match_mode: crate::banned_words::MatchMode::Substring,
                severity: crate::banned_words::Severity::Redact,
            });
        }
        let s = format_banned_words(&list);
        assert!(s.contains("word0"));
        assert!(s.contains("word2"));
    }

    #[test]
    fn allowlist_currently_accepts_everyone() {
        let user: OwnedUserId = "@anyone:example.org".parse().unwrap();
        assert!(is_inviter_allowed(&user));
    }
}
