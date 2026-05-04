#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Copyright (C) 2026 Sascha Daemgen, IT and More Systems
#
# Tägliches Backup der imogo-Bot-Datenbanken.
#
# Nutzt `sqlite3 .backup`, das einzige sichere Verfahren für eine
# konsistente Kopie einer SQLite-Datenbank, die möglicherweise gerade
# vom Bot beschrieben wird (`cp` würde bei aktivem Schreibvorgang
# eine korrupte Kopie liefern). Anschliessend wird mit gzip komprimiert
# (SQLite-DBs sind oft 5- bis 10-fach komprimierbar) und alte Backups
# älter als 30 Tage werden geloescht.
#
# Aufruf manuell: sudo -u imogobot bash deploy/scripts/backup-bot-databases.sh
# Oder per Cron (siehe README, manuell zu installieren):
#   0 3 * * * /opt/imogo-bots/scripts/backup-bot-databases.sh \
#       >> /var/log/imogo-bot-backup.log 2>&1

set -euo pipefail

BACKUP_DIR="/opt/imogo-bots/moderation-bot/backups"
DB_PATH="/opt/imogo-bots/moderation-bot/data/moderation.db"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RETENTION_DAYS=30

if [[ ! -f "$DB_PATH" ]]; then
    echo "Keine Datenbank unter $DB_PATH gefunden, überspringe Backup." >&2
    exit 0
fi

mkdir -p "$BACKUP_DIR"
BACKUP_FILE="$BACKUP_DIR/moderation-${TIMESTAMP}.db"

# `.backup` ist die einzige API, die garantiert keinen Lock hält und
# keine inkonsistente Kopie produziert. Anführungszeichen sind wichtig,
# weil der Pfad sonst von SQLite als Befehlsargument interpretiert würde.
sqlite3 "$DB_PATH" ".backup '$BACKUP_FILE'"

gzip "$BACKUP_FILE"

# Retention: alles älter als RETENTION_DAYS Tage löschen. -mtime "+N"
# matcht Dateien, deren letzte Änderung mehr als N Tage zurückliegt.
find "$BACKUP_DIR" -name 'moderation-*.db.gz' -mtime "+$RETENTION_DAYS" -delete

echo "Backup erstellt: ${BACKUP_FILE}.gz"
