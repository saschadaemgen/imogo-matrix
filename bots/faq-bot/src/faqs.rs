// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! FAQ data structures and matching algorithm.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs;

/// Errors raised when loading or parsing the FAQ file.
#[derive(Debug, Error)]
#[allow(clippy::module_name_repetitions)]
pub enum FaqError {
    /// I/O error reading the file.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// YAML parser error.
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// Top-level YAML document holding a list of FAQs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct FaqFile {
    /// Ordered list of FAQs. First match wins on score ties.
    pub faqs: Vec<Faq>,
}

/// One FAQ entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct Faq {
    /// Stable kebab-case identifier.
    pub id: String,
    /// Match keywords. Each is normalised and looked for as a substring in
    /// the normalised question.
    pub keywords: Vec<String>,
    /// Single-line title shown above the answer.
    pub summary: String,
    /// Markdown body of the answer.
    pub answer: String,
}

/// Load and parse the FAQ file.
///
/// # Errors
///
/// Returns [`FaqError::Io`] on filesystem errors or [`FaqError::Yaml`] on
/// parser errors.
pub async fn load(path: &Path) -> Result<Vec<Faq>, FaqError> {
    let contents = fs::read_to_string(path).await?;
    let parsed: FaqFile = serde_yaml::from_str(&contents)?;
    Ok(parsed.faqs)
}

/// Normalise a question for matching: lowercase, replace anything outside
/// `[a-z0-9 ]` with spaces, collapse whitespace.
#[must_use]
pub fn normalise(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_space = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

/// Find the best-matching FAQ for the given question, or `None` if no FAQ
/// has any keyword present in the question. On score ties, the first FAQ
/// in the list wins.
#[must_use]
pub fn match_faq<'a>(question: &str, faqs: &'a [Faq]) -> Option<&'a Faq> {
    let normalised = normalise(question);
    let mut best: Option<(&Faq, usize)> = None;

    for faq in faqs {
        let mut score = 0;
        for kw in &faq.keywords {
            let kw_norm = normalise(kw);
            if kw_norm.is_empty() {
                continue;
            }
            if normalised.contains(&kw_norm) {
                score += 1;
            }
        }
        if score > 0 {
            match best {
                None => best = Some((faq, score)),
                Some((_, prev_score)) if score > prev_score => best = Some((faq, score)),
                _ => {}
            }
        }
    }

    best.map(|(faq, _)| faq)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_faqs() -> Vec<Faq> {
        vec![
            Faq {
                id: "zugferd-grundlagen".to_string(),
                keywords: vec!["zugferd".into(), "hybrid".into()],
                summary: "Was ist ZUGFeRD?".to_string(),
                answer: "ZUGFeRD ist hybrid".to_string(),
            },
            Faq {
                id: "rechnung-stornieren".to_string(),
                keywords: vec!["storno".into(), "stornieren".into()],
                summary: "Wie stornieren?".to_string(),
                answer: "Storno-Rechnung erstellen".to_string(),
            },
        ]
    }

    #[test]
    fn matches_keyword_in_question() {
        let faqs = sample_faqs();
        let m = match_faq("Was ist ZUGFeRD eigentlich?", &faqs);
        assert!(m.is_some());
        assert_eq!(m.unwrap().id, "zugferd-grundlagen");
    }

    #[test]
    fn matches_inflected_keyword() {
        let faqs = sample_faqs();
        let m = match_faq("Wie kann ich Rechnung stornieren?", &faqs);
        assert!(m.is_some());
        assert_eq!(m.unwrap().id, "rechnung-stornieren");
    }

    #[test]
    fn no_match_returns_none() {
        let faqs = sample_faqs();
        let m = match_faq("Wie ist das Wetter heute?", &faqs);
        assert!(m.is_none());
    }

    #[test]
    fn higher_score_wins() {
        let mut faqs = sample_faqs();
        faqs.push(Faq {
            id: "winning".to_string(),
            keywords: vec!["zugferd".into(), "hybrid".into()],
            summary: "winner".to_string(),
            answer: "winner".to_string(),
        });
        // Question contains both keywords. Tie goes to first list entry,
        // which is `zugferd-grundlagen`.
        let m = match_faq("zugferd hybrid", &faqs).unwrap();
        assert_eq!(m.id, "zugferd-grundlagen");
    }

    #[test]
    fn normalises_punctuation() {
        assert_eq!(normalise("Hallo, Welt!"), "hallo welt");
        assert_eq!(normalise("ZUGFeRD-Rechnung?"), "zugferd rechnung");
    }
}
