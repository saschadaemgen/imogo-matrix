# imogo FAQ-Bot

Reaktiver Matrix-Bot, der haeufig gestellte Fragen in den imogo-Community-Raeumen beantwortet.

## Trigger

- `!faq <frage>` als Slash-Command
- Direkter Mention: `@bot-faq:imogo.de` (oder kurz `@bot-faq`)
- Direktnachricht (DM)

## Spezial-Befehle

- `!faq liste` - Uebersicht aller FAQ-Stichworte
- `!faq help` - Bot-Hilfe
- `!faq version` - Versions-Info

## Konfiguration

Siehe `faq-bot.example.toml`. Reale Config liegt unter `faq-bot.toml` (git-ignored).

## FAQ-Datenbank

Pflegen unter `data/faqs.yaml`. Bei aktivem Watcher (`[faqs] watch = true`) laedt der Bot die Datei bei Aenderungen automatisch neu.

## Lizenz

AGPL-3.0-or-later. Siehe `LICENSE` im Repository-Root.
