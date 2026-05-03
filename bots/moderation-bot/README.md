# imogo Moderations-Bot

Eigenstaendiger Application Service fuer die imogo-Community-Raeume auf B2B (`matrix.imogo.de`). Der Bot meldet sich beim Start einmal via `m.login.application_service` an, holt sich einen Access-Token, und laeuft danach als regulaerer Matrix-Client mit `restore_session` (Pull-AS-Architektur).

## Trigger

Befehle haben das Praefix `!mod`. Der Bot reagiert nur in Raeumen, die per `!mod aktivieren` aktiviert oder per Auto-Discovery (siehe Config `bot.auto_discover_alias_pattern`) eingetragen sind.

## Befehle

```
!mod aktivieren [note]
!mod deaktivieren
!mod status

!mod ban-word add <wort> [substring|whole_word] [redact|warn|kick]
!mod ban-word remove <wort>
!mod ban-word list

!mod kick @user[:server] [reason]
!mod ban @user[:server] [reason]
!mod unban @user[:server]
!mod mute @user[:server] <dauer> [reason]
!mod unmute @user[:server]

!mod pin                      # als Reply auf eine Nachricht
!mod unpin                    # als Reply auf eine Nachricht

!mod help
```

Dauer-Format fuer `mute`: `30s`, `5m`, `2h`, `1d`. Maximalwert aus Config `bot.max_mute_seconds`.

## Power-Level-Schwellen

Pro Befehl konfigurierbar in `[bot]`. Default: 50 fuer alle Befehle (`pl_kick`, `pl_ban`, `pl_mute`, `pl_pin`, `pl_word_admin`). Auto-Moderation greift NICHT bei Usern mit Power Level >= 100 (Admin-Schutz).

## Auto-Moderation

Bei jeder Nachricht in einem aktivierten Raum wird die Nachricht gegen die Bann-Wort-Liste geprueft. Erste Matching-Aktion gemaess Severity:

- `redact`: Nachricht wird redacted
- `warn`: Bot postet eine Warnung
- `kick`: User wird aus dem Raum entfernt

Die Liste wird per `!mod ban-word add/remove/list` live editiert und in `data/moderation.db` persistiert.

## Beispieldatei `data/banned_words.example.yaml`

Die Datei wird vom Bot **nicht** gelesen. Sie dient nur als Vorlage und Erklaerung des YAML-Schemas, falls jemand eine Bulk-Import-Funktion nachruesten moechte.

## Konfiguration

Siehe `mod-bot.example.toml`. Reale Config liegt unter `mod-bot.toml` (git-ignored). Pflichtfeld: `matrix.as_token`, das bei der AS-Registrierung von Tuwunel ausgegeben wird.

## Application-Service-Registrierung

Siehe `deploy/tuwunel/registration-imogo-moderator.yaml`. Sascha registriert den Service einmalig im Tuwunel-Admin-Raum:

```
!admin appservices register
\`\`\`yaml
<Inhalt der registration-imogo-moderator.yaml>
\`\`\`
```

Tuwunel antwortet mit `as_token` und `hs_token`. Beide in den Passwort-Manager und das `as_token` zusaetzlich in die `mod-bot.toml`.

## Audit-Log

Jede moderierende Aktion (manuell oder automatisch) wird in `moderation_audit_log` mit SHA-256-Hash-Chain gespeichert. Verifikation per `audit::verify_chain(&pool)`.

## Lizenz

AGPL-3.0-or-later. Siehe `LICENSE` im Repository-Root.
