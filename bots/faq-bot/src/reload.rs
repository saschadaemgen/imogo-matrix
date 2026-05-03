// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Hot-reload of the FAQ file using filesystem notifications.

use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{faqs::load, handler::FaqStore};

/// Spawn a background task that watches `path` and updates `store` on
/// changes. Returns the watcher (must be kept alive by the caller).
///
/// # Errors
///
/// Returns errors from `notify` initialisation or filesystem access.
#[allow(clippy::needless_pass_by_value)]
pub fn spawn_watcher(path: PathBuf, store: FaqStore) -> Result<RecommendedWatcher> {
    let (tx, mut rx) = mpsc::channel::<Event>(64);

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            let _ = tx.blocking_send(event);
        }
    })?;
    watcher.watch(&path, RecursiveMode::NonRecursive)?;

    let path_clone = path.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                continue;
            }
            // Debounce briefly so the editor has time to flush.
            tokio::time::sleep(Duration::from_millis(150)).await;
            match load(&path_clone).await {
                Ok(faqs) => {
                    let count = faqs.len();
                    store.store(Arc::new(faqs));
                    info!(count, "FAQs reloaded");
                }
                Err(e) => {
                    warn!(error = %e, "FAQ reload failed, keeping previous");
                }
            }
        }
    });

    Ok(watcher)
}
