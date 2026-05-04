#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Copyright (C) 2026 Sascha Daemgen, IT and More Systems
#
# Initiales VPS-Setup für imogo-Bots.
#
# Wird genau einmal pro VPS ausgeführt, ist aber idempotent: bestehende
# User, Verzeichnisse und Dateien werden respektiert. Lege folgende Dinge
# an:
#
#   - System-User imogobot mit /opt/imogo-bots als Home (kein Login-Shell)
#   - Verzeichnisse /opt/imogo-bots/moderation-bot/{data,backups}
#   - GitHub CLI (gh), falls nicht vorhanden
#   - systemd-Unit imogo-moderation-bot.service registrieren und enablen
#
# Aufruf: sudo bash deploy/scripts/setup-vps-imogobot.sh

set -euo pipefail

if [[ "$EUID" -ne 0 ]]; then
    echo "Dieses Skript muss als root ausgeführt werden." >&2
    exit 1
fi

# Skript-Verzeichnis ermitteln, damit relative Pfade auf die mitgelieferte
# systemd-Unit funktionieren, egal aus welchem CWD aufgerufen wird.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SYSTEMD_UNIT_SRC="$SCRIPT_DIR/../systemd/imogo-moderation-bot.service"
SYSTEMD_UNIT_DST="/etc/systemd/system/imogo-moderation-bot.service"

if [[ ! -f "$SYSTEMD_UNIT_SRC" ]]; then
    echo "systemd-Unit nicht gefunden: $SYSTEMD_UNIT_SRC" >&2
    exit 1
fi

# User imogobot anlegen, falls noch nicht vorhanden. --system gibt eine
# UID unter 1000, --shell /usr/sbin/nologin verhindert Login, --home-dir
# samt --create-home legt das Bot-Wurzel-Verzeichnis an.
if ! id imogobot >/dev/null 2>&1; then
    echo "Lege User imogobot an."
    useradd --system \
            --shell /usr/sbin/nologin \
            --home-dir /opt/imogo-bots \
            --create-home \
            imogobot
else
    echo "User imogobot existiert bereits, überspringe."
fi

# Verzeichnisstruktur. mkdir -p ist idempotent.
mkdir -p /opt/imogo-bots/moderation-bot/data
mkdir -p /opt/imogo-bots/moderation-bot/backups
chown -R imogobot:imogobot /opt/imogo-bots

# Restriktive Permissions. 750 für Wurzel und Bot-Dir bedeutet: imogobot
# darf alles, Gruppe darf lesen und betreten, sonstige nichts. 700 für
# data und backups bedeutet: nur imogobot, niemand sonst.
chmod 750 /opt/imogo-bots
chmod 750 /opt/imogo-bots/moderation-bot
chmod 700 /opt/imogo-bots/moderation-bot/data
chmod 700 /opt/imogo-bots/moderation-bot/backups

# GitHub CLI installieren, falls fehlt. Wird vom deploy-moderation-bot.sh
# benötigt, um Releases herunterzuladen. Repository-Setup folgt der
# offiziellen Anleitung von cli.github.com.
if ! command -v gh >/dev/null 2>&1; then
    echo "Installiere GitHub CLI."
    if ! command -v curl >/dev/null 2>&1; then
        apt-get install -y curl
    fi
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg
    chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg
    arch="$(dpkg --print-architecture)"
    echo "deb [arch=$arch signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list
    apt-get update
    apt-get install -y gh
else
    echo "GitHub CLI bereits installiert, überspringe."
fi

# systemd-Unit installieren oder aktualisieren.
install -m 0644 "$SYSTEMD_UNIT_SRC" "$SYSTEMD_UNIT_DST"
systemctl daemon-reload
systemctl enable imogo-moderation-bot.service

cat <<'EOF'

Setup abgeschlossen.

Nächste Schritte:
  1. mod-bot.toml nach /opt/imogo-bots/moderation-bot/ legen
     (mit echtem AS-Token, chown imogobot:imogobot, chmod 600)
  2. gh auth login (als root, wird für Deploy benötigt)
  3. Erstes Release-Tag setzen und pushen, um Binary zu produzieren
  4. deploy-moderation-bot.sh ausführen
  5. systemctl status imogo-moderation-bot prüfen

EOF
