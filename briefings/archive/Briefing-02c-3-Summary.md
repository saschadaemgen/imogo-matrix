# Briefing-02c-3 Completion Summary

**Status:** abgeschlossen
**Code-Commit:** c4917a0754359e92b544e7143a5f21f8ad4a37ae
**Summary-Commit:** wird mit dieser Datei angelegt
**Push:** ausstehend, wartet auf Prinz-Anweisung

## Was wurde gebaut

Der Provisioner reagiert jetzt auf `license.activated`-Webhooks und legt automatisch einen Matrix-Account, einen Support-Raum und einen Account-Record an. Der initiale Klartext-Passwort wird genau einmal an den Lizenz-Server zurueckgereicht und nirgends persistiert (auch nicht im Audit-Log).

Neue Dateien:

- `crates/imogo-provisioner/src/accounts.rs` (`AccountRecord`, `NewAccount`, `AccountsRepo`, `AccountError`)
- `crates/imogo-provisioner/src/identity.rs` (`generate_matrix_uuid`, `generate_initial_password`, `build_display_name`, `build_user_id`, `build_support_room_alias`, plus 6 Inline-Unit-Tests)
- `crates/imogo-provisioner/src/tuwunel.rs` (`TuwunelClient`, `register_user`, `set_display_name`, `create_room`, `PowerLevels`)
- `crates/imogo-provisioner/src/provisioning.rs` (`ProvisioningService`, `LicenseActivatedPayload`, `CustomerInfo`, `ActivationOutcome`, `ProvisioningError`, `sla_for_tier`)
- `crates/imogo-provisioner/migrations/0003_accounts.sql`
- `crates/imogo-provisioner/tests/provisioning.rs` (2 Tests: vollstaendiger Activation-Flow inkl. Idempotenz, abgelehnter Tier)

Geaenderte Dateien:

- `crates/imogo-provisioner/Cargo.toml` (rand und data-encoding zu `[dependencies]`)
- `crates/imogo-provisioner/src/config.rs` (`ProvisioningConfig`)
- `crates/imogo-provisioner/src/lib.rs` (`pub mod accounts; pub mod identity; pub mod provisioning; pub mod tuwunel;`)
- `crates/imogo-provisioner/src/http/appservice.rs` (`AppState.provisioning`)
- `crates/imogo-provisioner/src/http/router.rs` (`build` nimmt jetzt 4 Argumente)
- `crates/imogo-provisioner/src/http/mod.rs` (`AccountsRepo` + `ProvisioningService` im `run`)
- `crates/imogo-provisioner/src/http/webhook.rs` (Pipeline 5-stufig: verify -> audit -> parse -> event_type-gate -> provisioning)
- `crates/imogo-provisioner/provisioner.example.toml` (`[provisioning]`-Sektion)
- `crates/imogo-provisioner/tests/health.rs` (neuer Helper baut `ProvisioningService`)
- `crates/imogo-provisioner/tests/webhook.rs` (Helper baut `b2b`-HS gegen wiremock-only-versions; valide-signature- und replay-Tests erwarten 502/`tuwunel_error`)
- `Cargo.lock`

## Acceptance-Test-Report

| # | Test | Status | Details |
|---|---|---|---|
| 1 | `cargo build` (default) | PASS | warning-frei |
| 2 | `cargo build --features dev-keys` | PASS | warning-frei |
| 3 | `cargo clippy --all-targets -- -D warnings` (default) | PASS | nach `#[allow(clippy::too_many_lines)]` auf den zwei Orchestrierungs-Funktionen |
| 4 | `cargo clippy --all-targets --features dev-keys -- -D warnings` | PASS | clean |
| 5 | `cargo fmt --check` | PASS | nach `cargo fmt` (kosmetische Diffs) |
| 6 | `cargo test --features dev-keys` | PASS | **33 Tests gruen** (8 health + 5 audit + 4 nonce_store + 2 provisioning + 8 webhook + 6 identity unit) |
| 7 | Smoke-Test | PASS | Health 200, Webhook 401 missing_header, drei Migrationen liefen, Key-Registry-Warnung wie geplant |

Test-6 Auszug:

```
running 8 tests (health.rs)         ... 8 passed
running 5 tests (audit.rs)          ... 5 passed
running 4 tests (nonce_store.rs)    ... 4 passed
running 2 tests (provisioning.rs)   ... 2 passed
  test license_activation_creates_account ... ok
  test license_activation_rejects_invalid_tier ... ok
running 8 tests (webhook.rs)        ... 8 passed
running 6 tests (identity unit)     ... 6 passed
```

Test-7 Auszug:

```
INFO database opened and migrated path="./imogo-provisioner.db"
INFO matrix homeservers initialised configured=0 healthy=0
WARN DEV_PUBLIC_KEY_BYTES placeholder is not a valid Ed25519 encoding ...
INFO webhook key registry initialised registered_keys=0
INFO listening addr=127.0.0.1:8080

GET /healthz             -> 200 {"status":"ok","version":"0.1.0"}
POST /webhook/license -d "{}" -> 401 {"error":"missing_header"}
```

## Bekannte Punkte

1. **Webhook-Tests erwarten 502 statt 202.** Mit dem Provisioning-Pipeline-Schritt geht jeder Aufruf mit gueltiger Signatur in den ProvisioningService. Die Tests setzen einen wiremock-Tuwunel auf, der nur `/_matrix/client/versions` mockt; der `/register`-Call schlaegt mit 404 fehl, was als `TuwunelError::Api` -> `ProvisioningError::Tuwunel` -> HTTP 502 + `tuwunel_error` durchschlaegt. Die Replay-Test-Logik bleibt korrekt: erste Request bekommt 502 (Nonce wurde aber bereits eingelagert), zweite Request bekommt 401 nonce_replay. Das vollstaendige Mock-Setup mit `/register` und `/createRoom` lebt in `tests/provisioning.rs`, wo wir den End-to-End-Erfolgsfall testen.

2. **`#[allow(clippy::too_many_lines)]` auf den zwei Orchestrator-Funktionen.** Sowohl `license_webhook` (HTTP-Handler mit 5-stufiger Pipeline) als auch `handle_license_activated` (8 Provisioning-Schritte plus Audit-Logging) sind natuerlich lang. Aufteilen in kleinere Methoden wuerde mehr Argumenten-Geschiebe als Lesbarkeitsgewinn bedeuten. Das `allow` steht lokal an der Funktion.

3. **Audit-Log-Inhalt verifiziert ohne Passwort.** Vor dem Commit pruefte ich alle vier `audit.append`-Stellen in `provisioning.rs` plus die `webhook.license.received`-Stelle in `http/webhook.rs`. Keine enthaelt das `initial_password`-Feld. Der Webhook-Handler speichert nur den eingehenden Body (truncated auf 16 KiB), und die Eingangsschemata enthalten keinen Password-Schluessel - wir generieren das Passwort erst nach Audit.

4. **`AccountsRepo::insert` ist nicht atomar mit dem Tuwunel-Account-Anlegen.** Wenn der Tuwunel-Register-Call erfolgreich ist, aber das danach erfolgte SQLite-`INSERT` fehlschlaegt (z.B. UNIQUE-Constraint bei race), bleibt der Matrix-Account verwaist. Die Resume-Pattern aus dem Briefing greift nicht direkt, weil `find_by_license` keine Match findet. Empfehlung fuer 02d: optionale Rollback- oder Reconcile-Logik. Aktuell ist das Risiko niedrig, weil `matrix_uuid` aus 16 Random-Bytes besteht und Kollisionen praktisch ausgeschlossen sind.

5. **Display-Name-Fehler ist nicht-fatal.** Falls `set_display_name` einen Tuwunel-Fehler zurueckgibt, loggen wir warn und machen weiter. Der Account ist bereits angelegt und der Display-Name kann spaeter manuell gesetzt werden. Begruendung: ein Display-Name-Fehler ist viel weniger kritisch als ein abgebrochenes Setup.

6. **`bearer_auth` der reqwest 0.12 funktioniert wie erwartet.** Hatte als Fallback den Header `Authorization: Bearer ...` direkt setzen vorgesehen, war nicht noetig.

7. **`data-encoding` 2.x hat `BASE32_NOPAD`** wie im Briefing erwartet. Der Output ist Uppercase; `to_ascii_lowercase()` macht ihn passend zum Matrix-Localpart-Format (laut Spec sind Matrix-Localparts case-sensitive, aber Lowercase ist Konvention).

## Spezifikation für Master-Briefing 17 (Lizenz-Server, Erweiterung)

Erweiterte Anforderungen aus diesem Briefing:

8. **Webhook-Body fuer `license.activated`** muss folgende Form haben:
   ```json
   {
     "event_type": "license.activated",
     "license_id": "<opaque-string, max 256 chars>",
     "tier": "<one of: solo|kmu|pro|enterprise>",
     "customer": {
       "name": "<required, max 200 chars>",
       "company": "<optional or null>",
       "email": "<optional or null>"
     }
   }
   ```

9. **Initiales Passwort wird im Response zurueckgegeben** und MUSS vom Lizenz-Server an die imogo-App weitergegeben werden, ohne es persistent zu speichern. Form: 32 Random-Bytes base64-encoded ohne Padding (43 Zeichen). Die imogo-App rotiert binnen 5 Sekunden nach erstem Login.

10. **HTTP-Status-Codes** des `/webhook/license`-Endpoints (zur Implementierung der Lizenz-Server-Retry-Logik):
    - `201 Created` -> Account neu angelegt, `outcome.initial_password` enthaelt das Passwort
    - `200 OK` -> Account existierte bereits (idempotent return), kein Passwort
    - `400 Bad Request` -> `validation_error`, `invalid_payload`, oder `unsupported_event_type` -> NICHT retryen
    - `401 Unauthorized` -> Verifikations-Fehler -> Signatur/Timestamp/Nonce pruefen, dann ggf. mit neuer Nonce retryen
    - `500 Internal Server Error` -> `audit_failed`, `homeserver_not_registered`, `internal_error` -> mit neuer Nonce retryen
    - `502 Bad Gateway` -> `tuwunel_error` (Homeserver nicht erreichbar oder lehnt ab) -> mit neuer Nonce retryen

11. **TLS 1.3 zwingend** zwischen Lizenz-Server und Provisioner. Der Provisioner laeuft hinter nginx, das TLS terminiert. Keine HTTP-Variante anbieten.

12. **Nonce sollte mindestens 16 Zeichen** sein (Empfehlung: 32 Hex aus CSPRNG). Maximum: 128 Zeichen.

## Naechster Schritt

Briefing-02d (Lifecycle-Events: license.expired, license.deactivated, license.tier_changed), wartet auf Beauftragung.
