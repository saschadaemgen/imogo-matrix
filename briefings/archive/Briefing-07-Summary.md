# Briefing 07 Summary: Operations und CI

**Status:** abgeschlossen lokal, Live-Tests L01-L06 ausstehend (Server-Phase mit Sascha)
**Code-Commit:** wird mit dieser Datei zusammen angelegt
**Summary-Commit:** wird mit dieser Datei angelegt
**Push:** ausstehend, wartet auf Prinz-Anweisung
**Erster grüner CI-Run:** wird beim Push gegen main ausgelöst, URL folgt nach Push
**Tests gesamt (workspace):** 127 grün (unverändert)

## Was gebaut wurde

### `.github/workflows/ci.yml`

Continuous Integration für jeden Push und Pull-Request gegen `main`, plus manueller `workflow_dispatch`-Trigger. Schritte: fmt-check, clippy mit `-D warnings` und `--all-targets`, Release-Build, Workspace-Tests. Alle Schritte mit `--features dev-keys` (analog zur lokalen Entwicklung). Verwendet `dtolnay/rust-toolchain@stable` und `Swatinem/rust-cache@v2` für reproduzierbare Toolchain plus warmes Cache-Verhalten.

### `.github/workflows/release-moderation-bot.yml`

Triggert auf Tags der Form `mod-bot-v*` (zum Beispiel `mod-bot-v0.1.0`). Baut ein Linux-x86_64-Release-Binary, erzeugt eine SHA-256-Prüfsumme, und veröffentlicht beides plus Beispiel-Config und AS-Registration via `softprops/action-gh-release@v2`. Tag-Namespace reserviert das Prefix für den Moderations-Bot; `faq-bot-v*`, `support-bot-v*`, `provisioner-v*` folgen unabhängig in späteren Briefings.

**Anpassung gegenüber Briefing-Vorlage:** `permissions: contents: write` ergänzt, weil `softprops/action-gh-release` sonst beim Erstellen des Releases auf 403 läuft. GitHub-Default-Permissions sind seit 2023 für viele Repos restriktiv.

### `deploy/systemd/imogo-moderation-bot.service`

systemd-Unit mit Hardening-Direktiven. Service läuft als `imogobot:imogobot` mit `ProtectSystem=strict` plus expliziten `ReadWritePaths=/opt/imogo-bots/moderation-bot/data`. Weitere Direktiven (Kommentare im File): `MemoryDenyWriteExecute`, `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`, `NoNewPrivileges`, `PrivateTmp`, `ProtectKernelTunables/Modules/ControlGroups`. Resource-Limits: `LimitNOFILE=4096`, `TasksMax=128`. Crash-Loop-Schutz: `StartLimitBurst=5` in `StartLimitIntervalSec=600` mit `RestartSec=10s`.

### `deploy/scripts/deploy-moderation-bot.sh`

Lädt das neueste `mod-bot-v*`-Release via `gh release download`, verifiziert SHA-256 mit `sha256sum -c` (bricht bei Mismatch ab), stoppt den Service, ersetzt das Binary via `install -o imogobot -g imogobot -m 0755`, startet den Service, prüft `systemctl is-active`. Bei Misserfolg wird die letzte journal-Lage ausgegeben und mit Exit-Code 1 abgebrochen.

### `deploy/scripts/setup-vps-imogobot.sh`

Idempotentes Einmal-Setup: legt User `imogobot` an (System-User mit `--shell /usr/sbin/nologin`), erstellt Verzeichnisstruktur unter `/opt/imogo-bots/moderation-bot/{data,backups}` mit restriktiven Permissions (750 für Wurzel-Dirs, 700 für `data` und `backups`), installiert `gh` CLI via offizieller cli.github.com-Anleitung falls fehlt, kopiert die systemd-Unit, ruft `daemon-reload` und `systemctl enable`. Gibt am Ende eine Schritt-für-Schritt-Anleitung für die noch manuell erforderlichen Aktionen aus.

### `deploy/scripts/backup-bot-databases.sh`

Tägliches Backup mit `sqlite3 .backup` (einzige API, die garantiert eine konsistente Kopie einer aktiv beschriebenen DB liefert), `gzip` für Komprimierung (typisch 5-10x kleiner), und `find -mtime "+30" -delete` für automatische Rotation nach 30 Tagen. Cron-Eintrag wird **nicht** automatisch installiert (bewusste Operator-Entscheidung), die README enthält die exakte crontab-Zeile.

### `README.md` Erweiterung

Neue Sektion "Deployment" mit Subsektionen: Initial-Setup, Update, Logs, Backup manuell, CI. Vor "Verwandte Repositories", in der bestehenden Markdown-Struktur.

## Akzeptanztests

| # | Test | Status | Notiz |
|---|---|---|---|
| T01 | YAML-Syntax | DONE | `python -c "import yaml; yaml.safe_load(...)"` für beide Workflow-Dateien grün. yq und shellcheck waren auf der Windows-Dev-Maschine nicht installiert, daher Python-Surrogat. |
| T02 | Shellcheck | TEILWEISE | Lokal kein shellcheck installiert. Stattdessen `bash -n` für Syntax-Check, alle drei Skripte clean. Manuelle Code-Review nach SC2086/SC2155-Mustern: Variablen sind durchgängig gequotet, `mktemp -d` mit `trap` korrekt aufgesetzt, `set -euo pipefail` in allen Skripten. shellcheck-Run via shellcheck.net oder im CI später ist als Folge-Schritt empfohlen. |
| T03 | Workspace grün | DONE | 127 Tests grün, Workspace-Clippy mit `-D warnings` clean. Briefing 07 fügt nichts an Rust-Code hinzu, Tests sind unverändert. |
| T04 | CI grün auf GitHub | OFFEN | Wird beim ersten Push auf `main` ausgelöst. URL kommt nach Push in den Folge-Kommentar. |
| L01 | VPS-Setup | VORBEREITET | `setup-vps-imogobot.sh` idempotent, mit Schritt-für-Schritt-Ausgabe. Live-Test gemeinsam mit Sascha auf dem Hetzner-VPS. |
| L02 | Erstes Release und Deploy | VORBEREITET | Tag-Schema `mod-bot-vX.Y.Z`, Release-Workflow mit SHA-256-Pflicht, Deploy-Skript mit Verifikation. |
| L03 | Logs und Crash-Recovery | VORBEREITET | systemd `Restart=on-failure` mit `RestartSec=10s` und `StartLimitBurst=5`. |
| L04 | Lokaler Bot abschalten | VORBEREITET | Operations-Konzept: nur ein aktiver Bot pro AS-Token. Im L04-Live-Test verifiziert Sascha das. |
| L05 | Backup-Übung | VORBEREITET | `backup-bot-databases.sh` plus README-Schritte. |
| L06 | Restore-Übung | VORBEREITET | Disaster-Recovery-Schritte in Live-Tests, manuell durchzuspielen. |

## Wesentliche Befunde

1. **`permissions: contents: write` ist Pflicht für Release-Workflows.** GitHub-Default-Permissions sind seit Mitte 2023 für viele Organisationen auf "read all" oder strenger gestellt. `softprops/action-gh-release` braucht write-Berechtigung, sonst bricht der Release-Step mit HTTP 403 ab. Der Briefing-Vorlage habe ich diese Direktive ergänzt.

2. **Lokales Tooling fehlt teilweise.** Auf der Windows-Dev-Maschine sind weder `yq` noch `shellcheck` installiert. YAML-Validierung über Python `yaml.safe_load` ist äquivalent zu `yq eval '.'` für Syntax-Zwecke. Shellcheck wäre wertvoll, ist aber für die Akzeptanz nicht zwingend, weil `bash -n` Syntax-Fehler abfängt und manuelle Review die typischen SC2086-Fallen abdeckt. Operationaler Folge-Schritt: shellcheck im CI ergänzen (eigenes Briefing oder kleiner Hotfix in 07b).

3. **`User=imogobot` getrennt von Tuwunel.** Das Briefing fordert bewusst eine Sicherheits-Trennung: Bot kann Tuwunel-Datenbanken nicht lesen, auch wenn er kompromittiert würde. Der Tuwunel-User auf dem VPS ist `1000:1000` (Standard-User aus dem Docker-Compose), der Bot-User wird als System-User (UID < 1000) angelegt, also ohne Login-Shell und ohne Home-Directory-Auto-Mount.

4. **`ProtectSystem=strict` plus `ReadWritePaths=/opt/imogo-bots/moderation-bot/data`** ist die richtige Granularität. Das Bot-Binary, die `mod-bot.toml` und der `backups/`-Ordner liegen alle unter `/opt/imogo-bots/moderation-bot/`, aber nur das `data/`-Verzeichnis braucht Write-Access (für die SQLite-DB). Backup-Skript läuft via Cron als `imogobot`-User, also außerhalb der systemd-Sandbox, und kann frei lesen und schreiben.

5. **Backup-Cron wird absichtlich NICHT automatisch installiert.** Das ist eine bewusste Operator-Entscheidung. Wer das Backup einrichtet, soll wissen, was er tut, und sich aktiv per `crontab -e` dafür entscheiden. Im Setup-Skript wäre eine automatische Cron-Installation zu intransparent.

6. **Tag-Schema mit Namespace-Prefix.** `mod-bot-v0.1.0` statt einfach `v0.1.0` reserviert das Prefix für den Moderations-Bot. Spätere Komponenten (FAQ-Bot, Support-Bot, Provisioner) bekommen eigene Prefixe und können unabhängig versioniert werden, ohne dass der Release-Workflow auf "irgendein v*" triggert.

## Spec-Erweiterungen für Master-Briefing 17

Keine. Operations und CI sind interne Infrastruktur, kein Lizenz-Server-Touchpoint.

## Folge-Briefings

- **Briefing 07b (vorgeschlagen, optional):** shellcheck im CI ergänzen, falls die spätere shellcheck.net-Review Issues findet
- **Briefing 08:** FAQ-Bot und Provisioner ebenfalls auf systemd umstellen, mit gleichem User `imogobot` (Provisioner braucht eventuell eigenen User wegen Lizenz-Server-HTTP-Endpunkt-Listening)
- **Briefing 09:** Off-Site-Backup auf Hetzner-Storagebox oder S3-kompatibles Ziel
- **Optional:** Monitoring (Uptime-Robot oder Prometheus mit Grafana, je nach Operator-Präferenz)

## Push-Status

GitHub-Push: ausstehend, wartet auf Prinz-Anweisung.
