# Briefing-02a Completion Summary

**Status:** abgeschlossen
**Code-Commit:** 34ab7d187d49be7c7b095e4d07d82305ffa385b2
**Summary-Commit:** wird mit dieser Datei angelegt
**Push:** ausstehend, wartet auf Prinz-Anweisung

## Was wurde gebaut

Provisioner-Crate `crates/imogo-provisioner/` als lauffähiger axum-HTTP-Server-Skelett, ohne Matrix-Anbindung.

Neue Dateien:

- `crates/imogo-provisioner/src/lib.rs` (Bibliotheks-Wurzel, exportiert Module, `VERSION`-Konstante)
- `crates/imogo-provisioner/src/config.rs` (figment-basiertes Loading: Defaults > `provisioner.toml` > Env)
- `crates/imogo-provisioner/src/error.rs` (`thiserror`-Enum `Error::{Config, Telemetry, Io}`, `figment::Error` ist geboxt)
- `crates/imogo-provisioner/src/telemetry.rs` (`tracing-subscriber` mit `EnvFilter`, optional JSON, `OnceLock`-idempotent)
- `crates/imogo-provisioner/src/http/mod.rs` (`run`-Entry-Point, Trace- und Timeout-Layer, Graceful-Shutdown auf Ctrl-C / SIGTERM)
- `crates/imogo-provisioner/src/http/router.rs` (axum-Router mit `/healthz` und `/readyz`)
- `crates/imogo-provisioner/src/http/health.rs` (Handler liefern `{"status":"ok","version":"0.1.0"}`)
- `crates/imogo-provisioner/provisioner.example.toml` (Vorlage; reale `provisioner.toml` ist git-ignored)
- `crates/imogo-provisioner/tests/health.rs` (Integration-Tests für `/healthz` und `/readyz` auf ephemerem Port)

Geänderte Dateien:

- `crates/imogo-provisioner/Cargo.toml` (Dependencies tokio, axum, tower, tower-http, serde, figment, thiserror, anyhow, tracing, tracing-subscriber, dev-dep reqwest; `[lints] workspace = true`)
- `crates/imogo-provisioner/src/main.rs` (Platzhalter durch echten Entry-Point ersetzt)
- `.gitignore` (Eintrag `crates/imogo-provisioner/provisioner.toml` ergänzt)
- `Cargo.lock` (durch `cargo build` aktualisiert)

## Acceptance-Test-Report

| # | Test | Status | Details |
|---|---|---|---|
| 1 | `cargo build -p imogo-provisioner` | PASS | "Finished dev profile" ohne Warnings (nach Migration weg von deprecated `TimeoutLayer::new`) |
| 2 | `cargo clippy --all-targets -- -D warnings` | PASS | clean nach Behebung von 5 pedantic-Lints |
| 3 | `cargo fmt --check` | PASS | exit 0, keine Diffs |
| 4 | `cargo test -p imogo-provisioner` | PASS | 2 Integration-Tests grün: `healthz_returns_ok`, `readyz_returns_ok` |
| 5 | Manueller Smoke-Test | PASS (mit Plattform-Hinweis) | `GET /healthz` und `GET /readyz` auf 8080 liefern HTTP 200 mit Body `{"status":"ok","version":"0.1.0"}` |
| 6 | Env-Override `IMOGO_PROVISIONER_HTTP__LISTEN=127.0.0.1:9090` | PASS | Server bindet auf 9090, Log "listening addr=127.0.0.1:9090", 8080 nicht mehr offen, `/healthz` auf 9090 = 200 |

Detail-Auszüge:

**Test 5 - Logs des laufenden Servers:**

```
INFO imogo_provisioner: imogo-provisioner starting version="0.1.0" listen=127.0.0.1:8080
INFO imogo_provisioner::http: listening addr=127.0.0.1:8080
```

**Test 5 - Endpoint-Antworten:**

```
healthz status: 200
{"status":"ok","version":"0.1.0"}

readyz status: 200
{"status":"ok","version":"0.1.0"}
```

**Test 6 - Logs mit Override:**

```
INFO imogo_provisioner: imogo-provisioner starting version="0.1.0" listen=127.0.0.1:9090
INFO imogo_provisioner::http: listening addr=127.0.0.1:9090
9090 healthz: 200
{"status":"ok","version":"0.1.0"}
8080: Connection timed out (nicht gebunden, wie erwartet)
```

## Bekannte Punkte

1. **Deprecation-Korrektur in tower-http 0.6.** `tower_http::timeout::TimeoutLayer::new` ist in 0.6 mit `#[deprecated]` markiert. Briefing-Vorlage verwendete diese API. Entsprechend CLAUDE.md-Regel "NEVER use deprecated methods or legacy patterns" wurde auf die Nachfolger-API umgestellt:

   ```rust
   tower_http::timeout::TimeoutLayer::with_status_code(
       StatusCode::REQUEST_TIMEOUT,
       Duration::from_secs(config.http.request_timeout_secs),
   )
   ```

2. **Pedantic-Clippy-Lints behoben (Workspace-Setup hat `clippy::pedantic = "warn"` plus `-D warnings`).** Folgende Anpassungen gegenüber der Briefing-Vorlage:
   - `clippy::doc_markdown` in `config.rs`: `env_filter` zusätzlich gebackticked.
   - `clippy::result_large_err` in `error.rs`: `figment::Error` (208 Bytes) wurde geboxt; ein manueller `From<figment::Error>` -Impl liefert die Auto-Konvertierung.
   - `clippy::double_must_use` in `http/router.rs`: `#[must_use]` auf `build()` entfernt, da `axum::Router` bereits `#[must_use]` ist.
   - `clippy::derivable_impls` in `config.rs`: manueller `impl Default for Config` durch `#[derive(Default)]` ersetzt; `HttpConfig` und `LogConfig` behalten ihre Hand-Impls (Nicht-Triv-Defaults).
   - `clippy::module_name_repetitions` in `http/health.rs`: lokales `#[allow(clippy::module_name_repetitions)]` auf `pub struct Health`, da der Name explizit so im Briefing vorgegeben war.
   - Public Result-Funktionen `Config::load`, `telemetry::init`, `http::run` mit `# Errors`-Dokublöcken ergänzt.

3. **Auto-Link-Artefakt in Briefing-Codeblock korrigiert.** Im Briefing stand `Error::Telemetry([e.to](http://e.to)_string())` durch Markdown-Auto-Linking. Implementiert wurde wie offensichtlich gemeint: `Error::Telemetry(e.to_string())`.

4. **Graceful-Shutdown-Logzeile auf Windows nicht non-interaktiv prüfbar.** `tokio::signal::ctrl_c()` hängt unter Windows an `SetConsoleCtrlHandler`. Ein aus Bash mit `kill -INT` versendeter Signal-Versuch löst dieses Handler-Set nicht zuverlässig aus, weil der Prozess in einem Detached-Subsystem ohne Konsolen-Anbindung läuft. Die HTTP-Endpoints und das Startup-Logging wurden vollständig verifiziert (Server lauscht, /healthz und /readyz liefern 200, korrekte Body-Struktur). Der Log-Eintrag "received Ctrl-C, shutting down" sollte interaktiv im Terminal mit `cargo run -p imogo-provisioner` und gedrücktem Strg-C noch einmal vom Prinzen geprüft werden. Code-seitig ist die Logik unverändert aus der Briefing-Vorlage.

5. **CRLF-Hinweise von git harmlos.** Git zeigt beim `add` "LF will be replaced by CRLF" - das ist die Windows-Standardeinstellung und kein Fehler.

## Nächster Schritt

Briefing-02b (Application Service Integration), wartet auf Beauftragung.
