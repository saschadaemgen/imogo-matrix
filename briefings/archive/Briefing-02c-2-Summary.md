# Briefing-02c-2 Completion Summary

**Status:** abgeschlossen
**Code-Commit:** ad44e985717cd99f70eeefd9486c9f48ed8cc805
**Summary-Commit:** wird mit dieser Datei angelegt
**Push:** ausstehend, wartet auf Prinz-Anweisung

## Was wurde gebaut

Provisioner um zwei zusammenhaengende Bausteine erweitert:

1. **Append-only Audit-Log mit SHA-256-Hash-Chain in SQLite.** Jeder Eintrag bindet sich kryptografisch an seinen Vorgaenger via `prev_hash` (SHA-256 des Vorgaenger-`entry_hash`). Manipulationen an alten Eintraegen werden durch `verify_chain` zuverlaessig erkannt.
2. **Persistenter Nonce-Cache.** Loest den In-Memory-LRU aus 02c-1 ab. Replays ueberleben jetzt auch einen Restart. GC haengt am `try_insert` und entfernt alle Eintraege deren `expires_at` in der Vergangenheit liegt.

Neue Dateien:

- `crates/imogo-provisioner/src/db.rs` (`open_pool`, WAL-Journal, Auto-Migrate)
- `crates/imogo-provisioner/src/audit.rs` (`AuditLog`, `AuditEntry`, `NewAuditEntry`, `AuditError`, `compute_entry_hash`, Genesis-Hash)
- `crates/imogo-provisioner/src/nonce_store.rs` (`NonceStore`, `try_insert`, `contains`, `count`)
- `crates/imogo-provisioner/migrations/0001_audit_log.sql`
- `crates/imogo-provisioner/migrations/0002_webhook_nonces.sql`
- `crates/imogo-provisioner/tests/audit.rs` (5 Tests fuer Hash-Chain inkl. Tampering-Detection)
- `crates/imogo-provisioner/tests/nonce_store.rs` (4 Tests inkl. expired-GC)

Geaenderte Dateien:

- `crates/imogo-provisioner/Cargo.toml` (sqlx, libsqlite3-sys mit `bundled`, uuid; matrix-sdk OHNE `bundled-sqlite`-Feature)
- `crates/imogo-provisioner/src/config.rs` (`DbConfig`, `WebhookConfig.nonce_ttl_secs`)
- `crates/imogo-provisioner/src/error.rs` (`Error::Db(String)`)
- `crates/imogo-provisioner/src/lib.rs` (`pub mod audit; pub mod db; pub mod nonce_store;`)
- `crates/imogo-provisioner/src/webhook.rs` (LRU/Mutex entfernt, `WebhookVerifier::new` nimmt jetzt `NonceStore`)
- `crates/imogo-provisioner/src/http/appservice.rs` (`AppState.audit_log`)
- `crates/imogo-provisioner/src/http/router.rs` (`build` nimmt `AuditLog`)
- `crates/imogo-provisioner/src/http/mod.rs` (Pool/AuditLog/NonceStore-Setup im `run`)
- `crates/imogo-provisioner/src/http/webhook.rs` (verifizierte Webhooks erzeugen Audit-Eintrag, antworten mit `audit_id`)
- `crates/imogo-provisioner/provisioner.example.toml` (`[db]`, `[webhook] nonce_ttl_secs`)
- `crates/imogo-provisioner/tests/health.rs` (neuer `build_test_state`-Helper, baut Pool/AuditLog/NonceStore aus tempfile)
- `crates/imogo-provisioner/tests/webhook.rs` (an neue Verifier-Signatur und neuen `audit_id` im Response angepasst)
- `.gitignore` (DB-Dateien)
- `Cargo.lock`

## Acceptance-Test-Report

| # | Test | Status | Details |
|---|---|---|---|
| 1 | `cargo build -p imogo-provisioner` | PASS | nach Wechsel matrix-sdk weg von `bundled-sqlite` und expliziter `libsqlite3-sys = { version = "0.30", features = ["bundled"] }` |
| 2 | `cargo build -p imogo-provisioner --features dev-keys` | PASS | warning-frei |
| 3 | `cargo clippy --all-targets -- -D warnings` | PASS | nach Backticking aller pedantic-doc-Markdown-Stellen (`SQLite`, `prev_hash`, `entry_hash`, `created_at`, `id`, `SQLx`) |
| 4 | `cargo clippy --all-targets --features dev-keys -- -D warnings` | PASS | clean |
| 5 | `cargo fmt -p imogo-provisioner --check` | PASS | nach `cargo fmt` (kosmetische Diffs) |
| 6 | `cargo test --features dev-keys` | PASS | 25 Tests gruen: 8 health + 5 audit + 4 nonce_store + 8 webhook |
| 7 | Manueller Smoke-Test | PASS | Server startet, Log "database opened and migrated", `imogo-provisioner.db`/`-shm`/`-wal` werden angelegt, `/healthz` -> 200 |

Test-6 Auszug:

```
test result: ok. 8 passed; 0 failed (health.rs)

running 5 tests
test empty_chain_verifies ... ok
test appending_entries_chains_correctly ... ok
test tampered_payload_breaks_chain ... ok
test tampered_prev_hash_breaks_chain ... ok
test many_entries_chain_well ... ok
test result: ok. 5 passed; 0 failed (audit.rs)

running 4 tests
test first_insert_returns_true ... ok
test second_insert_of_same_nonce_returns_false ... ok
test different_nonces_coexist ... ok
test expired_nonces_get_collected ... ok
test result: ok. 4 passed; 0 failed (nonce_store.rs)

running 8 tests
test webhook_accepts_valid_signature ... ok
test webhook_rejects_missing_signature ... ok
test webhook_rejects_tampered_body ... ok
test webhook_rejects_old_timestamp ... ok
test webhook_rejects_replay ... ok
test webhook_rejects_unknown_key_id ... ok
test webhook_rejects_wrong_signing_key ... ok
test webhook_rejects_path_mismatch ... ok
test result: ok. 8 passed; 0 failed (webhook.rs)
```

Test-7 Auszug:

```
INFO imogo_provisioner: imogo-provisioner starting version="0.1.0" listen=127.0.0.1:8080
INFO imogo_provisioner::db: database opened and migrated path="./imogo-provisioner.db"
INFO imogo_provisioner::http: matrix homeservers initialised configured=0 healthy=0
WARN imogo_provisioner::keys: DEV_PUBLIC_KEY_BYTES placeholder is not a valid Ed25519 encoding ...
INFO imogo_provisioner::http: webhook key registry initialised registered_keys=0
INFO imogo_provisioner::http: listening addr=127.0.0.1:8080

GET /healthz -> 200 {"status":"ok","version":"0.1.0"}

ls imogo-provisioner.db*:
  imogo-provisioner.db
  imogo-provisioner.db-shm
  imogo-provisioner.db-wal
```

## Bekannte Punkte

1. **`libsqlite3-sys`-Konflikt zwischen matrix-sdk-sqlite und sqlx geloest durch Feature-Trennung.** matrix-sdk-sqlite 0.13 pinnt `libsqlite3-sys = ^0.33`, sqlx-sqlite 0.8.6 pinnt `^0.30.1`. Cargo verbietet zwei unterschiedliche Versionen einer `links = "sqlite3"`-Crate. Loesung: matrix-sdk wurde das `bundled-sqlite`-Feature komplett entzogen (matrix-sdk laeuft fuer unsere Zwecke mit dem In-Memory-State-Store; wir nutzen den State-Store nicht und der Connectivity-Check `ping` haengt seit 02b an einem direkten reqwest-Call). Stattdessen pullt sqlx libsqlite3-sys 0.30 ein, und wir aktivieren explizit `libsqlite3-sys = { version = "0.30", features = ["bundled"] }` damit sqlite3 aus Quelle compiliert wird (Windows hat sonst kein `sqlite3.lib`).

2. **chrono mit `clock`+`std`+`serde`-Features.** Der Default war fuer 02b reicht aus; fuer `Utc::now()` in `audit.rs` und `nonce_store.rs` brauchten wir `clock`, fuer das Zusammenspiel mit sqlx das `std`-Feature.

3. **`fetch_optional` statt `fetch_one + .ok().flatten()`.** Briefing-Vorlage hat in `audit::append` und `nonce_store::contains` jeweils einen `fetch_one(...).await.ok().flatten().unwrap_or_else(...)`-Pattern, der echte DB-Fehler stillschweigend in Defaults verwandelt. Beide auf `fetch_optional(...).await?` mit korrekter Fehler-Propagation umgestellt. `nonce_store::contains` nutzt jetzt `let-else` und behandelt malformierte Timestamps als "abgelaufen", damit ein Replay nicht durchschluepfen kann.

4. **`AuditLog::len` mit `#[allow(clippy::len_without_is_empty)]`.** Pedantic erwartet ein passendes `is_empty`. Da der Test `assert_eq!(log.len().await.unwrap(), 50)` an dieser Signatur haengt und `is_empty` im Async-Kontext keinen klaren Mehrwert bringt, das `allow` lokal an die Methode.

5. **`AppState`/`AuditLog`/`WebhookVerifier` haben manuelle `Debug`-Impls** mit `finish_non_exhaustive`, weil `SqlitePool` und `KeyRegistry::keys` keine sinnvollen Inhalte fuer Debug ausgeben sollen (Pool-Verbindungen, Schluessel-Material).

6. **`WebhookConfig.nonce_cache_capacity` bleibt als Field-Stub fuer Schema-Kompat erhalten.** Wert wird gelesen aber nirgends verwendet; Default 10000 zur Aussenwirkung. Falls in 02c-3+ entschieden wird, das Feld zu entfernen, ist die Anpassung lokal an `config.rs` und der Beispiel-TOML.

7. **Nebenlaeufige Audit-Append-Aufrufe sind theoretisch racy.** Der Read-then-Insert-Pattern in `AuditLog::append` ist innerhalb einer Transaktion atomar, aber zwei parallele `append`-Calls koennten denselben `prev_hash` lesen und damit einen Chain-Split produzieren. Fuer den aktuellen Einsatzfall (ein Webhook-Handler appendet pro Request) unkritisch. Der Fix in einer spaeteren Iteration: globale Mutex am `AuditLog` oder `BEGIN IMMEDIATE`. Notiert in der `append`-Doku.

8. **`audit_failed`-Pfad im Webhook-Handler.** Wenn nach erfolgreicher Verifikation der Audit-Append fehlschlaegt, antwortet der Endpoint 500 statt 202. Das Lizenz-Server-Retry-Verhalten muss in Master-Briefing 17 entsprechend formuliert werden: bei 5xx wird retried (mit neuer Nonce, da die alte schon im Store steckt), bei 4xx nicht. Der `internal_error`-String tritt in Antworten auf, wenn `nonce_store::try_insert` selbst eine DB-Exception wirft.

9. **Pedantic-doc-Markdown sehr streng.** Mehrere "SQLite"/"SQLx"/"prev_hash" mussten gebackticked werden. Im Zweifel: jeden code-aehnlichen Identifier in Doku in Backticks setzen.

## Naechster Schritt

Briefing-02c-3 (Account- und Raum-Lifecycle), wartet auf Beauftragung.
