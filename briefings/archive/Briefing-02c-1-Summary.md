# Briefing-02c-1 Completion Summary

**Status:** abgeschlossen
**Code-Commit:** 785e564c145015cc235751f56cc11e99044a788a
**Summary-Commit:** wird mit dieser Datei angelegt
**Push:** ausstehend, wartet auf Prinz-Anweisung

## Was wurde gebaut

Provisioner um die Empfangsschicht für signierte Lizenz-Server-Webhooks erweitert: Ed25519-Verifikation, Timestamp-Frische, Nonce-Replay-Schutz im LRU-Cache, Key-Registry mit `dev-keys`-Feature-Gate.

Neue Dateien:

- `crates/imogo-provisioner/src/keys.rs` (`KeyRegistry`, `RegisteredKey`, optionaler Dev-Key über `dev-keys` Feature)
- `crates/imogo-provisioner/src/webhook.rs` (`WebhookVerifier`, `WebhookVerifyError`, `VerifiedRequest`, `build_signing_string`, Header-Konstanten)
- `crates/imogo-provisioner/src/http/webhook.rs` (`license_webhook`-Handler, `WebhookAck`/`WebhookError`)
- `crates/imogo-provisioner/tests/webhook.rs` (8 Tests, gated auf `#![cfg(feature = "dev-keys")]`, je Test eigenes Keypair)

Geänderte Dateien:

- `crates/imogo-provisioner/Cargo.toml` (deps: ed25519-dalek, sha2, base64, hex, lru, zeroize, reqwest war schon da; `[features] default = []`, `dev-keys = []`)
- `crates/imogo-provisioner/src/config.rs` (`WebhookConfig` mit `max_timestamp_skew_secs`, `nonce_cache_capacity`)
- `crates/imogo-provisioner/src/lib.rs` (`pub mod keys; pub mod webhook;`)
- `crates/imogo-provisioner/src/http/appservice.rs` (`AppState.webhook_verifier`)
- `crates/imogo-provisioner/src/http/router.rs` (`/webhook/license` POST-Route, `build` nimmt jetzt zusätzlich `WebhookVerifier`)
- `crates/imogo-provisioner/src/http/mod.rs` (`run` baut `KeyRegistry::with_compiled_in_keys()` und `WebhookVerifier`)
- `crates/imogo-provisioner/provisioner.example.toml` (`[webhook]`-Sektion)
- `crates/imogo-provisioner/tests/health.rs` (an neue `router::build`-Signatur angepasst, `WebhookVerifier::new(KeyRegistry::default(), 1024, 300)`)

## Acceptance-Test-Report

| # | Test | Status | Details |
|---|---|---|---|
| 1 | `cargo build -p imogo-provisioner` | PASS | warning-frei nach Anpassung des `mut`-Bindings hinter `cfg_attr` |
| 2 | `cargo build -p imogo-provisioner --features dev-keys` | PASS | warning-frei |
| 3 | `cargo clippy --all-targets -- -D warnings` | PASS | clean |
| 4 | `cargo clippy --all-targets --features dev-keys -- -D warnings` | PASS | clean |
| 5 | `cargo fmt -p imogo-provisioner --check` | PASS | nach `cargo fmt` (kosmetische Diffs) |
| 6 | `cargo test -p imogo-provisioner --features dev-keys` | PASS | 16 Tests grün (8 aus 02b + 8 aus 02c-1) |
| 7 | Manueller Smoke-Test | PASS | Webhook ohne Header -> 401 `{"error":"missing_header"}`; `/healthz` -> 200; Warnung "DEV_PUBLIC_KEY_BYTES placeholder is not a valid Ed25519 encoding" wird sauber geloggt; `registered_keys=0` wie erwartet |

Test-6 Ausgabe (relevante Zeilen):

```
running 8 tests
test healthz_returns_ok ... ok
test readyz_with_no_homeservers_is_ok ... ok
test readyz_with_reachable_homeserver_is_ok ... ok
test transactions_endpoint_rejects_missing_token ... ok
test transactions_endpoint_rejects_wrong_token ... ok
test transactions_endpoint_accepts_correct_token ... ok
test transactions_endpoint_unknown_homeserver_returns_404 ... ok
test user_exists_returns_404_with_correct_token ... ok

test result: ok. 8 passed; 0 failed

running 8 tests
test webhook_rejects_unknown_key_id ... ok
test webhook_rejects_missing_signature ... ok
test webhook_rejects_old_timestamp ... ok
test webhook_rejects_wrong_signing_key ... ok
test webhook_rejects_path_mismatch ... ok
test webhook_rejects_tampered_body ... ok
test webhook_accepts_valid_signature ... ok
test webhook_rejects_replay ... ok

test result: ok. 8 passed; 0 failed
```

Test-7 Ausgabe:

```
POST /webhook/license ohne Header:
  HTTP 401, body {"error":"missing_header"}

GET /healthz:
  HTTP 200, body {"status":"ok","version":"0.1.0"}

Logs:
  INFO matrix homeservers initialised configured=0 healthy=0
  WARN keys: DEV_PUBLIC_KEY_BYTES placeholder is not a valid Ed25519 encoding;
       no dev key registered. Replace bytes or inject a key at runtime via
       KeyRegistry::insert.
  INFO webhook key registry initialised registered_keys=0
  INFO listening addr=127.0.0.1:8080
  WARN license webhook rejected error="missing required header: x-imogo-timestamp"
```

## Bekannte Punkte

1. **`license_server_dev_key()` gibt `Option<RegisteredKey>` zurück, nicht `RegisteredKey`.** Die Briefing-Vorlage sah einen `unwrap_or_else`-Fallback auf Null-Bytes vor, der mit hoher Wahrscheinlichkeit (Null ist kein gueltiger Ed25519-Punkt) bei Binary-Start eine Panik ausgeloest hätte. Stattdessen: `from_bytes(...).ok().map(...)` und in `with_compiled_in_keys` ein `if let Some(k)` mit Warn-Log im Else-Pfad. Die Tests injizieren ohnehin eigene Schlüssel über `KeyRegistry::insert` und brauchen den Platzhalter nicht. Der Smoke-Test bestätigt: Binary startet, der Warn-Log feuert wie geplant, `registered_keys=0`, Webhook lehnt jeden Aufruf mangels Header oder mangels passender Key-Id ab.

2. **`mut`-Binding in `with_compiled_in_keys` per `#[cfg_attr]` gegated.** Ohne `dev-keys`-Feature ist der Insertion-Block leer und das `mut` würde mit `unused_mut` warnen. Behoben durch `#[cfg_attr(not(feature = "dev-keys"), allow(unused_mut))]` direkt am `let`-Binding, sauberer als ein globaler `#[allow]`.

3. **Manuelle `Debug`-Implementation für `WebhookVerifier`.** Die Briefing-Vorlage nur mit `#[derive(Clone)]`, aber `AppState` hat `#[derive(Clone, Debug)]` und braucht daher `Debug` auf allen Feldern. Die manuelle Impl zeigt `max_timestamp_skew_secs` und `registered_keys` ohne den LRU-Cache zu locken und ist damit log-/print-sicher.

4. **`tokio` mit zusätzlichem `sync`-Feature.** `tokio::sync::Mutex` braucht das `sync`-Feature, das wir explizit aktiviert haben (vorher implizit über die anderen Features).

5. **`#[allow(clippy::too_many_arguments)]` auf `WebhookVerifier::verify`.** Die Funktion hat 7 Parameter (`&self`, method, path, 4 header, body); pedantic clippy schlägt sonst an. Alternativ könnte man die 4 Header in eine eigene Struct fassen, das wäre aber unnötiger Wrapper-Aufwand für interne API.

6. **`i64::try_from(secs)` statt `as i64`.** Vermeidet pedantic-Lints `cast_possible_truncation` und `cast_possible_wrap` und ist ohnehin korrekter; bei einem Time-Wert jenseits von 2038-i64::MAX bricht die Verifikation mit `TimestampOutOfRange` sauber ab.

7. **Pfad-Extraktion mit `path_and_query().map_or_else(|| uri.path(), PathAndQuery::as_str)`.** Klare axum-0.8-konforme Form, vermeidet `unwrap_or` und stellt sicher, dass beim Fehlen von `path_and_query()` nur der Pfad in den Signing-String fließt. Tests ohne Query-String sind damit weiterhin abgedeckt.

8. **Auto-Link-Artefakte aus dem Briefing korrigiert.** Diverse `[e.to](http://e.to)_string()`, `[token.as](http://token.as)_bytes()`, `[verified.nonce.as](http://verified.nonce.as)_str()`-Stellen wurden in echten Rust-Code aufgelöst.

## Spezifikation für Master-Briefing 17 (Lizenz-Server)

Der Lizenz-Server muss in Master-Season-3 folgende Anforderungen erfüllen, damit er kompatibel zum imogo-matrix Provisioner Webhook ist:

1. **Signatur-Algorithmus:** Ed25519 nach RFC 8032.

2. **Signing-String-Format:** 5 Zeilen, durch genau ein `\n` getrennt, ASCII-only, kein Trailing-Newline:
   - Zeile 1: HTTP-Method, uppercase (`POST`, `PUT`, ...)
   - Zeile 2: URL-Pfad inklusive Query-String (z.B. `/webhook/license`, oder `/webhook/license?env=prod`)
   - Zeile 3: Unix-Timestamp in Sekunden, als ASCII-Dezimalzahl (kein Padding, kein Suffix)
   - Zeile 4: Nonce, beliebige ASCII-Zeichen ohne `\n`, max. 128 Zeichen
   - Zeile 5: SHA-256-Hex des kompletten Request-Body, lowercase, 64 Zeichen

3. **Header (alle vier sind Pflicht):**
   - `X-Imogo-Timestamp`: Unix-Timestamp in Sekunden (gleiche Repräsentation wie in Signing-String Zeile 3)
   - `X-Imogo-Nonce`: zufälliger Wert pro Request (UUID, Hex-String, beliebig sonst)
   - `X-Imogo-Signature`: Base64-encoded (`STANDARD_NO_PAD`, also kein `=`-Padding) Ed25519-Signatur, ergibt einen 86-Zeichen-String
   - `X-Imogo-Key-Id`: Identifier des verwendeten Public Keys, muss zur Provisioner-Konstante passen (initial `dev-license-server-2026`, später Produktions-Key-Id)

4. **Replay-Schutz auf Sender-Seite:** Nonce muss eindeutig pro Key-Id sein, mindestens innerhalb von 24 Stunden. Empfohlene Form: 32 Hex-Zeichen aus einem CSPRNG.

5. **Clock-Skew-Toleranz:** Provisioner toleriert ±300 Sekunden zur eigenen Uhr (konfigurierbar via `[webhook] max_timestamp_skew_secs`). Lizenz-Server soll seine Uhren mit NTP synchron halten.

6. **Key-Verteilung:** Public Key wird statisch im Provisioner-Quellcode in `crates/imogo-provisioner/src/keys.rs` als `[u8; 32]`-Konstante hinterlegt. Eine Rotation erfolgt durch parallele Eintragung von altem und neuem Public Key während eines Übergangsfensters; jeder Key hat eine eigene `key_id`.

7. **HTTP-Antworten:**
   - Erfolgreich verifiziert: HTTP 202 mit Body `{"status":"verified","key_id":"...","nonce":"..."}`
   - Verifikation gescheitert: HTTP 401 mit Body `{"error":"<reason>"}`. `<reason>` ist eines von: `missing_header`, `malformed_header`, `timestamp_out_of_range`, `nonce_replay`, `unknown_key_id`, `bad_signature`. Der Lizenz-Server soll die Reason nicht für Logik nutzen, nur für Logs.

8. **Endpoint-URL:** `POST <provisioner-base>/webhook/license`. Content-Type `application/json` empfohlen, der Provisioner verifiziert nur Bytes, der Type ist Cosmetik.

## Nächster Schritt

Briefing-02c-2 (Audit-Log mit Hash-Chain), wartet auf Beauftragung.
