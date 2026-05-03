# Briefing-02e Completion Summary

**Status:** abgeschlossen
**Code-Commit:** b36fac65d20e95455372abd34de7099c84131993
**Summary-Commit:** wird mit dieser Datei angelegt
**Push:** ausstehend, wartet auf Prinz-Anweisung

## Was wurde gebaut

Endkundenkanal als zweite, parallele Welt zum B2B-Provisioning. Drei Bausteine:

### Capability-Token-Modell

- JWT mit JOSE `alg: EdDSA`, `kid` aus zweiter Key-Registry (`CapabilityKeyRegistry`)
- Claims: `iss=imogo-license-server`, `sub=<license_id>`, `matrix_user_id`, `caps`, `iat`, `exp`, `jti`
- Replay-Schutz ueber persistierte `capability_jti_cache`-Tabelle (analog `webhook_nonces`, GC bei jedem Insert)
- Validierung: `kid` lookup, Signatur, `iss`, `exp`/`iat`/leeway, IAT max 24h alt, geforderte Capability vorhanden, `jti` neu

### B2C-Service mit zwei API-Endpoints

- **`POST /v1/b2c/rooms`** (Bearer-token, requires `b2c.create_room`): legt Raum auf B2C-Tuwunel an, generiert `qr_token` mit TTL (Default 90 Tage, max 365), persistiert in `b2c_rooms`-Tabelle, antwortet mit `qr_url` zum Drucken auf der Rechnung.
- **`POST /v1/b2c/redeem`** (anonym, Auth ueber den QR-Token selbst): legt durchnummerierten Gast-Account `@gast-<rechnungsnr>-<lfd>` an, login_appservice fuer Access-Token, invite in den Raum, persistiert in `b2c_guests`-Tabelle, antwortet mit Login-Token fuer den Browser-Matrix-Client.

### Datenbankschema

Migration 0005 erzeugt drei Tabellen: `b2c_rooms`, `b2c_guests` (FK zu rooms), `capability_jti_cache`.

### Tuwunel-Erweiterungen

Drei neue HTTP-Methoden im `TuwunelClient`: `invite_user`, `login_appservice` (m.login.application_service mit identifier+user fuer Token-Issue), und schon zuvor `update_power_levels`/`update_room_topic`/`deactivate_user` aus 02d.

### Audit-Eintraege

Neue Event-Typen: `b2c.room.created`, `b2c.room.create_failed` (fire-and-forget Spawn im Fehlerfall), `b2c.guest.joined`, `b2c.token.expired`, `b2c.token.invalid`. Login-Tokens werden niemals geloggt.

### Neue Dateien

- `crates/imogo-provisioner/migrations/0005_b2c.sql`
- `crates/imogo-provisioner/src/b2c.rs` (`B2cService`, `CreateRoomRequest`/`Response`, `RedeemRequest`/`Response`, `B2cError`, `normalise_invoice_number`)
- `crates/imogo-provisioner/src/capability.rs` (`CapabilityVerifier`, `CapabilityClaims`, `CapabilityError`, JWT-Decode plus jti-Replay-Schutz)
- `crates/imogo-provisioner/src/http/b2c.rs` (`create_room`, `redeem`, Error-Mappings)
- `crates/imogo-provisioner/tests/b2c.rs` (5 Tests inkl. JWT-Issuing in PKCS#8 v2)

### Geaenderte Dateien

- `crates/imogo-provisioner/Cargo.toml` (jsonwebtoken 9, urlencoding war schon da)
- `crates/imogo-provisioner/src/config.rs` (`B2cConfig`)
- `crates/imogo-provisioner/src/keys.rs` (`CapabilityKeyRegistry` parallel zu `KeyRegistry`)
- `crates/imogo-provisioner/src/lib.rs` (`pub mod b2c; pub mod capability;`)
- `crates/imogo-provisioner/src/tuwunel.rs` (`invite_user`, `login_appservice`)
- `crates/imogo-provisioner/src/http/appservice.rs` (`AppState.b2c`, `AppState.capability_verifier`)
- `crates/imogo-provisioner/src/http/router.rs` (`build` jetzt 6 Args, zwei neue B2C-Routen, `b2c as b2c_handler` Alias)
- `crates/imogo-provisioner/src/http/mod.rs` (`B2cService` und `CapabilityVerifier` im `run` konstruiert)
- `crates/imogo-provisioner/provisioner.example.toml` (`[b2c]`-Sektion)
- `crates/imogo-provisioner/tests/health.rs` und `tests/webhook.rs` (Helper laden `B2cService` und `CapabilityVerifier` mit ein)
- `Cargo.lock`

## Acceptance-Test-Report

| # | Test | Status | Details |
|---|---|---|---|
| 1 | `cargo build` (default) | PASS | warning-frei nach Klarstellung des `user_id`-Locals |
| 2 | `cargo build --features dev-keys` | PASS | warning-frei |
| 3 | `cargo clippy --all-targets -- -D warnings` (default) | PASS | nach 4 Anpassungen (let-else, Backticks fuer EdDSA und SubjectPublicKeyInfo) |
| 4 | `cargo clippy --all-targets --features dev-keys -- -D warnings` | PASS | nach `caps: &[&str]` (needless_pass_by_value) im Test-Helper |
| 5 | `cargo fmt --check` | PASS | nach `cargo fmt` |
| 6 | `cargo test --features dev-keys` | PASS | **46 Tests gruen** (8 health + 5 audit + 5 b2c + 8 lifecycle + 4 nonce_store + 2 provisioning + 8 webhook + 6 identity-unit) |
| 7 | Smoke-Test | PASS (mit Befund) | Health 200; B2C-Endpoints reagieren wie erwartet (siehe Bekannte Punkte 4) |

Test-6 Auszug (b2c.rs):

```
running 5 tests
test invoice_number_normalisation ... ok
test missing_capability_is_rejected ... ok
test invalid_qr_token_returns_not_found ... ok
test token_replay_is_rejected ... ok
test create_room_then_redeem_works ... ok

test result: ok. 5 passed; 0 failed
```

Test-7 Auszug:

```
GET  /healthz                                   -> 200 {"status":"ok","version":"0.1.0"}
POST /v1/b2c/rooms (valid body, no auth)        -> 401 {"error":"bad_auth_header"}
POST /v1/b2c/rooms (valid body, bogus bearer)   -> 401 {"error":"token_decode_error"}
POST /v1/b2c/redeem {"qr_token":"x"}            -> 401 {"error":"invalid_or_expired_token"}

INFO capability key registry initialised registered_keys=1
INFO database opened and migrated path="./imogo-provisioner.db"
```

## Bekannte Punkte

1. **`DecodingKey::from_ed_der` ist in jsonwebtoken 9.x irrefuehrend benannt.** Trotz `_der`-Suffix erwartet die Funktion fuer Ed25519 KEINE `SubjectPublicKeyInfo`-Wrapper, sondern den nackten 32-Byte-Public-Key (jsonwebtoken reicht die Bytes direkt an `ring::signature::UnparsedPublicKey` weiter, das fuer `ED25519` 32 Rohbytes will). Erste Implementierung mit SPKI-Wrapper schlug mit `InvalidSignature` fehl. Loesung: `DecodingKey::from_ed_der(&key.key.to_bytes())` ohne Wrap.

   Fuer `EncodingKey::from_ed_der` (Signing-Seite, im Test) gilt das Gegenteil: hier braucht ring tatsaechlich PKCS#8 v2 (mit Seed plus optionalem Public Key). Der Test-Helper `ed25519_to_pkcs8_der` baut dieses Wrapping korrekt.

2. **`#[allow(clippy::too_many_arguments)]` auf `router::build`.** Sechs Argumente nach 02e: registry, webhook_verifier, audit_log, provisioning, b2c, capability_verifier. Eine Builder-Pattern-Refaktorierung wurde verworfen, weil die Funktion nur an drei Stellen aufgerufen wird.

3. **Power-Level-Modell B2C v1: nur Provisioner-Bot ist Admin.** Der Handwerker ist NICHT als Power-100 im Raum. Begruendung: er ist B2B-User auf einem anderen Server und muesste via Foederation eingeladen werden. Das geht zwar technisch, ist aber in v1 ausserhalb des Scopes (siehe Briefing-Scope-Grenzen). In v2 wird der Handwerker direkt nach Raum-Erstellung per Foederation eingeladen und sein Power-Level auf 100 gesetzt. Aktuell sind alle Schreibrechte beim Bot konsolidiert.

4. **axum `Json<T>`-Extractor-Reihenfolge fuehrt zu 422 statt 401 bei syntaktisch leerem Body.** Beim Smoke-Test gab `POST /v1/b2c/rooms` mit Body `{}` einen 422 (axum lehnt JSON-Body-Deserialisierung VOR der Auth-Pruefung ab). Mit syntaktisch validem Body und fehlendem/falschem Bearer kommt korrekt 401 zurueck. Die Briefing-Erwartung ("401 bei leerem Body") ist deshalb nicht ganz zutreffend, aber das Verhalten ist semantisch korrekt: ein leerer Body ist KEIN gueltiges `CreateRoomRequest`, also ist 422 die richtige Antwort. Auth-Pfad wurde mit dem zweiten Curl explizit verifiziert.

5. **`spawn_audit` als fire-and-forget fuer Fehler-Audit-Eintraege.** `B2cService::create_room` schreibt bei Tuwunel-Fehlschlag einen `b2c.room.create_failed`-Audit-Eintrag asynchron via `tokio::spawn`, statt den Fehler-Pfad mit einem zweiten `await?` zu komplizieren. Konsequenz: das Audit kann theoretisch nach dem Response geschrieben werden. Fuer Audit-Zwecke akzeptabel, wir haben Reihenfolge-Garantien innerhalb eines einzelnen Audit-Append (Hash-Chain bleibt intakt), nur die zeitliche Reihenfolge zur HTTP-Response ist nicht garantiert.

6. **`CAP_DEV_PUBLIC_KEY_BYTES`-Platzhalter mit Glueck gueltige Ed25519-Punkte.** Im Smoke-Test wurde `registered_keys=1` geloggt (im Gegensatz zur Webhook-Key-Registry, deren Platzhalter ungueltig ist und 0 Keys ergibt). Die Bytes wurden zufaellig gewaehlt, sind aber gerade ein gueltiger Punkt. Fuer Production wird das Byte-Array sowieso ersetzt (Master-Briefing 17). Tests injizieren ihre eigenen Schluessel und sind unabhaengig vom Platzhalter.

7. **Tuwunel `m.login.application_service`-Login** ist die Standard-AS-Login-Methode aus der Matrix-Spec. Falls Tuwunel das Format anders erwartet (z.B. `inhibit_login: true` im Register-Call gibt schon einen Token zurueck), kann der Login-Step in `redeem` weggelassen werden. Aktuell zwei Calls: register (ohne Login) plus expliziter Login.

## Spezifikation für Master-Briefing 17 (Lizenz-Server, Erweiterung)

19. **Capability-Token-Format** (zusaetzlich zu den Webhook-Anforderungen aus 02c-1, 02d):
    - Algorithmus: `EdDSA` (Ed25519)
    - JOSE-Header: `alg=EdDSA`, `typ=JWT`, `kid=<id-aus-CapabilityKeyRegistry>` (initial: `license-server-cap-dev-2026`, spaeter `license-server-cap-2026` oder neuer)
    - Claims (alle Pflicht ausser wo notiert):
      - `iss`: konstant `"imogo-license-server"`
      - `sub`: opaque license id (entspricht `license_id` aus dem Activation-Webhook)
      - `matrix_user_id`: vollqualifizierter B2B-Matrix-User-ID des Lizenznehmers
      - `caps`: Array mit Capabilities, aktuell unterstuetzt: `b2c.create_room`, geplant: `b2c.list_my_rooms`
      - `iat`: Unix-Timestamp Sekunden, max. 24h alt
      - `exp`: Unix-Timestamp Sekunden, Provisioner toleriert 60s Skew
      - `jti`: UUID v4, eindeutig pro Token (Replay-Schutz)
    - Empfohlene Lifetime: 15 Minuten (kurzer Token, oft refreshed)
    - Refresh-Strategie: Lizenz-Server haelt langlebige Refresh-Tokens (separat, z.B. 30 Tage), die imogo-App tauscht sie gegen kurze Capability-Tokens via `/license-server/capabilities`-Endpoint (auf Lizenz-Server-Seite zu spezifizieren)

20. **Zweiter Public-Key-Slot** im Provisioner: `CapabilityKeyRegistry`. Operationally getrennt vom Webhook-Key, damit Rotation und Trust-Scope unabhaengig steuerbar sind. Aktuelle dev `kid`: `license-server-cap-dev-2026`. Production-Rollout: neue `kid` `license-server-cap-2026` oder hoeher, beide parallel akzeptiert waehrend Rotation.

21. **HTTP-Status-Codes B2C-API:**
    - `POST /v1/b2c/rooms`:
      - `201 Created` -> Raum + QR-Token erfolgreich angelegt
      - `400 Bad Request` -> `validation_error` (invalid invoice_number / invoice_subject / TTL out of range)
      - `401 Unauthorized` -> Capability-Token-Fehler (`bad_auth_header`, `token_decode_error`, `unknown_key_id`, `invalid_token`, `token_expired`, `token_iat_too_old`, `token_replay`, `missing_capability`)
      - `422 Unprocessable Entity` -> Body kein gueltiges JSON oder fehlende Felder (axum-Default vor Auth)
      - `500 Internal Server Error` -> Datenbank- oder Konfigurations-Fehler
      - `502 Bad Gateway` -> Tuwunel hat den `createRoom`-Call abgelehnt
    - `POST /v1/b2c/redeem`:
      - `200 OK` -> Gast-Account angelegt, Login-Token in Response
      - `401 Unauthorized` -> `invalid_or_expired_token` (Token unbekannt oder abgelaufen)
      - `409 Conflict` -> `guest_limit_exceeded` (mehr als `guest_index_max` Scans auf demselben QR)
      - `500 Internal Server Error` -> `homeserver_not_registered` oder `internal_error`
      - `502 Bad Gateway` -> Tuwunel-Fehler (register/login/invite)

22. **TLS 1.3 zwingend** zwischen Lizenz-Server, imogo-App, Webseite und Provisioner. Der Provisioner laeuft hinter nginx, das TLS terminiert.

## Naechster Schritt

Briefing-03 (FAQ-Bot), wartet auf Beauftragung.
