// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Matrix sync loop for the FAQ bot.

use anyhow::Result;
use matrix_sdk::{
    Client, Room, SessionMeta, SessionTokens,
    authentication::matrix::MatrixSession,
    config::SyncSettings,
    ruma::events::room::message::{
        MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent, SyncRoomMessageEvent,
    },
    store::RoomLoadSettings,
};
use tracing::{debug, error, info, warn};

use crate::{
    config::MatrixConfig,
    handler::{FaqStore, decide_reply},
};

/// Build a Matrix client and restore the AS-issued access token.
///
/// # Errors
///
/// Returns any error from the matrix-sdk client builder, user-id parsing, or
/// session restore.
pub async fn build_client(cfg: &MatrixConfig) -> Result<Client> {
    let client = Client::builder()
        .homeserver_url(cfg.homeserver_url.as_str())
        .build()
        .await?;

    let user_id = matrix_sdk::ruma::OwnedUserId::try_from(cfg.user_id.clone())?;
    let device_id = matrix_sdk::ruma::OwnedDeviceId::from(format!("FAQBOT-{}", crate::VERSION));

    let session = MatrixSession {
        meta: SessionMeta { user_id, device_id },
        tokens: SessionTokens {
            access_token: cfg.access_token.clone(),
            refresh_token: None,
        },
    };
    client
        .matrix_auth()
        .restore_session(session, RoomLoadSettings::default())
        .await?;

    Ok(client)
}

/// Run the sync loop:
/// 1. Initial `sync_once` to prime the client state and capture a sync token,
///    BEFORE any handler is registered (so old messages do not trigger
///    replies).
/// 2. Register the message handler.
/// 3. Long-poll `sync` from the captured token.
///
/// # Errors
///
/// Returns sync errors from matrix-sdk.
pub async fn run(client: Client, faq_store: FaqStore, bot_user_id: String) -> Result<()> {
    info!(user_id = bot_user_id.as_str(), "starting sync");

    // Step 1: prime state without our handler.
    let initial_token = match client.sync_once(SyncSettings::default()).await {
        Ok(resp) => Some(resp.next_batch),
        Err(e) => {
            warn!(error = %e, "initial sync failed, continuing without skip-token");
            None
        }
    };

    // Step 2: register handler. From this point on, only NEW events fire it.
    let bot_id_for_handler = bot_user_id.clone();
    let store_for_handler = faq_store.clone();
    client.add_event_handler(move |event: SyncRoomMessageEvent, room: Room| {
        let bot_id = bot_id_for_handler.clone();
        let store = store_for_handler.clone();
        async move {
            let SyncRoomMessageEvent::Original(original) = event else {
                return;
            };
            on_message(&bot_id, store, original, room).await;
        }
    });

    // Step 3: long-poll sync from the captured token.
    let settings = if let Some(t) = initial_token {
        SyncSettings::default().token(t)
    } else {
        SyncSettings::default()
    };
    client.sync(settings).await?;

    Ok(())
}

async fn on_message(
    bot_user_id: &str,
    faq_store: FaqStore,
    event: OriginalSyncRoomMessageEvent,
    room: Room,
) {
    if event.sender.as_str() == bot_user_id {
        return;
    }

    let MessageType::Text(ref text_content) = event.content.msgtype else {
        return;
    };
    let body = text_content.body.clone();

    let is_dm = is_direct_message_room(&room).await;

    let faqs_arc = faq_store.load();
    let reply = decide_reply(bot_user_id, &body, &faqs_arc, is_dm);

    let Some(reply_text) = reply else {
        return;
    };

    // We post the reply as a fresh message rather than a Matrix `m.relates_to`
    // reply, because matrix-sdk 0.13's `make_reply_to` API has a third
    // parameter (`AddMentions`) that varies between minor versions and adds
    // little user-visible value here. The bot's avatar/name is enough context.
    let content = RoomMessageEventContent::text_markdown(&reply_text);

    if let Err(e) = room.send(content).await {
        error!(error = %e, "failed to send reply");
    } else {
        debug!("reply sent");
    }
}

async fn is_direct_message_room(room: &Room) -> bool {
    room.is_direct().await.unwrap_or(false)
}
