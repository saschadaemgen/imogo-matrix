// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Decision logic for the FAQ bot. Pure synchronous code (no Matrix SDK
//! types here) so it can be exhaustively unit-tested in isolation from the
//! sync loop in [`crate::matrix_client`].

use std::sync::Arc;

use arc_swap::ArcSwap;
use tracing::{debug, info};

use crate::faqs::{Faq, match_faq};

/// Holds the current FAQ list, hot-swappable on reload.
pub type FaqStore = Arc<ArcSwap<Vec<Faq>>>;

/// Decide what (if anything) the bot should reply with for a given message.
///
/// `bot_user_id` is the bot's own Mxid (e.g. `@bot-faq:imogo.de`).
/// `message_body` is the plain text of the incoming message.
///
/// Returns `Some(reply)` to post a reply, `None` to stay silent.
#[must_use]
pub fn decide_reply(
    bot_user_id: &str,
    message_body: &str,
    faqs: &[Faq],
    is_dm: bool,
) -> Option<String> {
    let trigger = identify_trigger(bot_user_id, message_body, is_dm);
    let question = match trigger {
        Trigger::None => return None,
        Trigger::Mention(rest) | Trigger::SlashCommand(rest) | Trigger::Dm(rest) => rest,
    };

    let trimmed = question.trim();
    if trimmed.is_empty() {
        return Some(help_text());
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower == "help" || lower == "hilfe" {
        return Some(help_text());
    }
    if lower == "version" {
        return Some(format!("imogo FAQ-Bot Version {}", crate::VERSION));
    }
    if lower == "liste" || lower == "list" {
        return Some(format_list(faqs));
    }

    if let Some(faq) = match_faq(trimmed, faqs) {
        debug!(faq_id = faq.id.as_str(), "matched faq");
        Some(format!("**{}**\n\n{}", faq.summary, faq.answer))
    } else {
        info!(question = trimmed, "no FAQ matched");
        Some(no_match_text())
    }
}

#[derive(Debug, PartialEq)]
enum Trigger<'a> {
    None,
    Mention(&'a str),
    SlashCommand(&'a str),
    Dm(&'a str),
}

fn identify_trigger<'a>(bot_user_id: &str, message_body: &'a str, is_dm: bool) -> Trigger<'a> {
    let trimmed = message_body.trim_start();

    // Slash command takes priority because it is the most explicit form.
    if let Some(rest) = trimmed.strip_prefix("!faq") {
        return Trigger::SlashCommand(rest.trim_start());
    }

    // Full-Mxid mention.
    if let Some(idx) = message_body.find(bot_user_id) {
        let rest_start = idx + bot_user_id.len();
        let rest = &message_body[rest_start..];
        let rest = rest.trim_start_matches([':', ',']).trim_start();
        return Trigger::Mention(rest);
    }

    // Short-form mention (@bot-faq without server part).
    if let Some(localpart) = bot_user_id
        .strip_prefix('@')
        .and_then(|s| s.split_once(':').map(|x| x.0))
    {
        let short = format!("@{localpart}");
        if let Some(idx) = message_body.find(&short) {
            let rest_start = idx + short.len();
            let rest = &message_body[rest_start..];
            let rest = rest.trim_start_matches([':', ',']).trim_start();
            return Trigger::Mention(rest);
        }
    }

    if is_dm {
        return Trigger::Dm(message_body.trim());
    }

    Trigger::None
}

fn help_text() -> String {
    "Hallo! Ich bin der imogo FAQ-Bot.\n\
     \n\
     Du kannst mich so ansprechen:\n\
     - `!faq <deine frage>` zum Beispiel `!faq Wie storniere ich eine Rechnung?`\n\
     - Oder erwaehne mich direkt mit @bot-faq\n\
     - Oder schreib mir privat\n\
     \n\
     Weitere Befehle: `!faq liste` (alle Stichworte), `!faq version`, `!faq help`.\n\
     \n\
     Wenn ich keine Antwort habe, frag im Support-Raum nach."
        .to_string()
}

fn no_match_text() -> String {
    "Ich habe keine passende FAQ gefunden. Schreib `!faq liste` fuer eine Uebersicht \
     aller Stichworte oder schreib im Support-Raum fuer persoenliche Hilfe."
        .to_string()
}

fn format_list(faqs: &[Faq]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("**Verfuegbare FAQs:**\n\n");
    for faq in faqs {
        let _ = writeln!(out, "- **{}** ({})", faq.summary, faq.keywords.join(", "));
    }
    if faqs.is_empty() {
        out.push_str("(keine FAQs konfiguriert)");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_faqs() -> Vec<Faq> {
        vec![Faq {
            id: "test".to_string(),
            keywords: vec!["zugferd".into()],
            summary: "Was ist ZUGFeRD?".to_string(),
            answer: "Hybrides Format".to_string(),
        }]
    }

    #[test]
    fn slash_command_triggers_reply() {
        let r = decide_reply("@bot-faq:imogo.de", "!faq zugferd", &sample_faqs(), false);
        assert!(r.is_some());
        assert!(r.unwrap().contains("Hybrides Format"));
    }

    #[test]
    fn mention_triggers_reply() {
        let r = decide_reply(
            "@bot-faq:imogo.de",
            "@bot-faq:imogo.de was ist zugferd?",
            &sample_faqs(),
            false,
        );
        assert!(r.is_some());
    }

    #[test]
    fn short_mention_triggers_reply() {
        let r = decide_reply(
            "@bot-faq:imogo.de",
            "@bot-faq was ist zugferd?",
            &sample_faqs(),
            false,
        );
        assert!(r.is_some());
    }

    #[test]
    fn no_trigger_returns_none() {
        let r = decide_reply(
            "@bot-faq:imogo.de",
            "irgendeine nachricht ohne mention",
            &sample_faqs(),
            false,
        );
        assert!(r.is_none());
    }

    #[test]
    fn dm_triggers_reply() {
        let r = decide_reply("@bot-faq:imogo.de", "was ist zugferd", &sample_faqs(), true);
        assert!(r.is_some());
    }

    #[test]
    fn empty_slash_command_shows_help() {
        let r = decide_reply("@bot-faq:imogo.de", "!faq", &sample_faqs(), false);
        let body = r.unwrap();
        assert!(body.contains("FAQ-Bot"));
    }

    #[test]
    fn version_command_works() {
        let r = decide_reply("@bot-faq:imogo.de", "!faq version", &sample_faqs(), false);
        assert!(r.unwrap().contains("Version"));
    }

    #[test]
    fn list_command_works() {
        let r = decide_reply("@bot-faq:imogo.de", "!faq liste", &sample_faqs(), false);
        let body = r.unwrap();
        assert!(body.contains("Was ist ZUGFeRD?"));
    }

    #[test]
    fn no_match_message_shown() {
        let r = decide_reply(
            "@bot-faq:imogo.de",
            "!faq voellig fremdes thema",
            &sample_faqs(),
            false,
        );
        let body = r.unwrap();
        assert!(body.contains("keine passende FAQ"));
    }
}
