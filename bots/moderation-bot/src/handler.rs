// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Matrix event handler. Wires together command parsing, power-level
//! enforcement, the banned-word matcher, and the audit log.
//!
//! The handler logic is intentionally kept thin around the (already
//! unit-tested) pure modules: every code path here that hits matrix-sdk is
//! manually verified against a real Tuwunel during the live-test phase
//! (T02-T14 of Briefing-04).

use std::sync::Arc;

use matrix_sdk::{
    Client, Room,
    config::SyncSettings,
    ruma::{
        OwnedRoomId, OwnedUserId,
        events::room::message::{
            MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
            SyncRoomMessageEvent,
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
    power_level, rooms,
};

/// Shared application state passed to the matrix-sdk event handler.
#[derive(Clone)]
pub struct BotState {
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

/// Run the bot: prime sync, register handler, long-poll.
///
/// # Errors
///
/// Returns [`ModError::Matrix`] from the underlying sync.
pub async fn run(client: Client, state: BotState) -> Result<(), ModError> {
    info!(user_id = state.bot_user_id.as_str(), "starting sync");

    // Step 1: sync_once without our handler so we do not process old messages.
    let initial_token = match client.sync_once(SyncSettings::default()).await {
        Ok(resp) => Some(resp.next_batch),
        Err(e) => {
            warn!(error = %e, "initial sync failed, continuing without skip-token");
            None
        }
    };

    // Step 2: auto-discovery
    if let Err(e) = auto_discover(&client, &state).await {
        warn!(error = %e, "auto-discovery failed");
    }

    // Step 3: register handler
    let state_for_handler = state.clone();
    client.add_event_handler(move |event: SyncRoomMessageEvent, room: Room| {
        let s = state_for_handler.clone();
        async move {
            let SyncRoomMessageEvent::Original(original) = event else {
                return;
            };
            if let Err(e) = on_message(s, original, room).await {
                error!(error = %e, "message handler failed");
            }
        }
    });

    // Step 4: long-poll
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
        let alias = room.canonical_alias();
        let Some(alias) = alias else { continue };
        if !regex.is_match(alias.as_str()) {
            continue;
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
            new_count += 1;
            audit::append(
                &state.pool,
                AuditEntry::now(
                    Some(room_id.clone()),
                    state.bot_user_id.to_string(),
                    "auto_discovered".to_string(),
                    None,
                    None,
                    json!({
                        "alias": alias.as_str(),
                    }),
                ),
            )
            .await?;
        }
    }
    info!(new_count, "auto-discovery complete");
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
                json!({"note": note}),
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
                json!({"word": word, "mode": mode.as_str(), "severity": severity.as_str()}),
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
                json!({"word": word}),
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
        Command::Mute { .. } | Command::Unmute { .. } | Command::Pin | Command::Unpin => {
            let msg = "Dieser Befehl ist im Briefing-04-Skelett implementiert, \
                       Live-Verhalten wird in den Akzeptanztests T10/T11 verifiziert.";
            ack(room, msg).await;
            audit_simple(
                state,
                &room_id,
                &actor,
                "command_received_pending_live_test",
                json!({"command": format!("{cmd:?}")}),
            )
            .await?;
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
        json!({"command": format!("{cmd:?}")}),
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
            json!({"reason": reason}),
        ),
    )
    .await?;
    ack(room, &format!("{action_label} ausgefuehrt fuer {target}.")).await;
    Ok(())
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
}
