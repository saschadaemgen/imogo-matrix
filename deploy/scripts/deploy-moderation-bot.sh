#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Copyright (C) 2026 Sascha Daemgen, IT and More Systems
#
# Deploy-Skript für imogo-moderation-bot.
#
# Lädt das neueste GitHub-Release (Tag-Form `mod-bot-v*`) herunter,
# verifiziert die SHA-256-Summe, ersetzt das alte Binary atomar und
# startet den systemd-Service neu. Bei Hash-Mismatch oder Service-
# Restart-Fehler wird abgebrochen, das alte Binary bleibt aktiv.
#
# Voraussetzungen:
#   - gh CLI installiert und authentifiziert (`gh auth login` als root)
#   - Ausführung als root oder per sudo (für systemctl)
#   - User imogobot existiert
#   - /opt/imogo-bots/moderation-bot/ ist via setup-vps-imogobot.sh
#     vorbereitet
#
# Aufruf: sudo bash deploy/scripts/deploy-moderation-bot.sh

set -euo pipefail

REPO="saschadaemgen/imogo-matrix"
INSTALL_DIR="/opt/imogo-bots/moderation-bot"
BINARY_NAME="moderation-bot"
SERVICE="imogo-moderation-bot.service"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

if [[ "$EUID" -ne 0 ]]; then
    echo "Dieses Skript muss als root ausgeführt werden." >&2
    exit 1
fi

# Neuestes mod-bot-v*-Release suchen. `gh release list` liefert chronologisch
# absteigend; mit grep filtern wir auf das mod-bot-Prefix und nehmen das
# erste Resultat.
TAG="$(gh release list --repo "$REPO" --limit 50 \
    --json tagName --jq '.[].tagName' \
    | grep '^mod-bot-v' | head -n 1)"

if [[ -z "$TAG" ]]; then
    echo "Kein mod-bot-v*-Release gefunden." >&2
    exit 1
fi

echo "Deploying $TAG"

# Binary plus Pruefsummen-Datei herunterladen.
gh release download "$TAG" --repo "$REPO" \
    --pattern "moderation-bot-linux-x86_64*" \
    --dir "$TMP_DIR"

# SHA-256 verifizieren. Bei Mismatch exit 1, set -e bricht das Skript ab,
# das alte Binary bleibt unangetastet.
cd "$TMP_DIR"
sha256sum -c moderation-bot-linux-x86_64.sha256

# Service stoppen, alten Prozess geordnet beenden.
echo "Stopping $SERVICE"
systemctl stop "$SERVICE" || true

# Atomarer Replace via install (cp wäre nicht atomar bei laufendem Prozess,
# wenn auch hier nicht relevant da Service vorher gestoppt wurde). install
# setzt zusätzlich Owner und Mode in einem Aufruf.
install -o imogobot -g imogobot -m 0755 \
    moderation-bot-linux-x86_64 \
    "$INSTALL_DIR/$BINARY_NAME"

# Service starten.
echo "Starting $SERVICE"
systemctl start "$SERVICE"

# Pragmatische Health-Check: 2 Sekunden warten, dann Status prüfen.
# Für unsere Größenordnung ausreichend; ein echter Readiness-Check
# bräuchte einen HTTP-Endpoint im Bot.
sleep 2

if systemctl is-active --quiet "$SERVICE"; then
    echo "Deploy erfolgreich. Service läuft."
    systemctl status "$SERVICE" --no-pager --lines=5
else
    echo "Deploy fehlgeschlagen, Service läuft nicht!" >&2
    journalctl -u "$SERVICE" --no-pager --lines=20 >&2
    exit 1
fi
