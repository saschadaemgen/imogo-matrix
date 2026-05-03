// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! Integration tests: load the real `data/faqs.yaml` file and exercise matches.

use std::path::PathBuf;

use faq_bot::faqs;

fn data_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("data/faqs.yaml");
    p
}

#[tokio::test]
async fn loads_yaml_file() {
    let faqs = faqs::load(&data_path()).await.expect("load");
    assert!(!faqs.is_empty());
    for f in &faqs {
        assert!(!f.id.is_empty());
        assert!(!f.summary.is_empty());
        assert!(!f.answer.is_empty());
    }
}

#[tokio::test]
async fn matches_zugferd_question() {
    let faqs = faqs::load(&data_path()).await.expect("load");
    let m = faqs::match_faq("Was ist ZUGFeRD?", &faqs);
    assert!(m.is_some());
}

#[tokio::test]
async fn matches_storno_question() {
    let faqs = faqs::load(&data_path()).await.expect("load");
    let m = faqs::match_faq("Wie kann ich eine Rechnung stornieren?", &faqs);
    assert!(m.is_some());
}
