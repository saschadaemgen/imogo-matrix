# Contributing to imogo-matrix

Vielen Dank für Euer Interesse an imogo-matrix. Dieses Dokument beschreibt, wie Contributions in diesem Repository ablaufen, welche Regeln gelten und welche Lizenz-Konsequenzen ein Beitrag hat.

---

## Lizenz-Verständnis bei Contributions

Wenn Ihr Code, Konfiguration, Scripts oder Dokumentation zu diesem Repository beitragt, akzeptiert Ihr automatisch die folgende Lizenz-Regelung:

- **Code-Beiträge** zu `crates/imogo-provisioner/` und `bots/` werden unter **GNU Affero General Public License v3.0 oder später** (AGPL-3.0-or-later) lizenziert.
- **Operations-Material-Beiträge** zu `deploy/` und `scripts/` werden unter **MIT License** lizenziert.
- **Dokumentations-Beiträge** zu `docs/` und Markdown-Dateien werden unter **Creative Commons Attribution 4.0 International** (CC-BY-4.0) lizenziert.

Mit dem Einreichen einer Pull Request bestätigt Ihr, dass:

1. Der Beitrag von Euch verfasst wurde oder dass Ihr ausreichende Rechte habt, ihn unter der jeweiligen Lizenz beizusteuern.
2. Ihr der Lizenzierung des Beitrags unter der oben genannten Lizenz zustimmt.
3. Ihr verstanden habt, dass die Beiträge im öffentlichen Repository sichtbar werden und nicht nachträglich exklusiv zurückgezogen werden können.

Es ist kein separater CLA (Contributor License Agreement) zu unterschreiben. Die Lizenz-Zustimmung erfolgt implizit durch die Pull Request.

---

## Code-Stil und Konventionen

### Sprache im Code

- **Code-Kommentare, Variablennamen, Commit-Messages, technische Dokumentation:** Englisch
- **User-facing-Texte (Bot-Antworten, Fehlermeldungen für Endnutzer):** Deutsch primär, Englisch als zweite Sprache, später weitere
- **Diese CONTRIBUTING.md, README.md:** Deutsch

### Rust-Stil

- `cargo fmt` vor jedem Commit
- `cargo clippy --all-targets --all-features` ohne Warnungen
- Idiomatisches Rust 1.94 oder höher
- Tokio als Async-Runtime
- `axum` für HTTP-Server
- `matrix-sdk` und `matrix-sdk-appservice` für Matrix-Integration
- Eigene Crate-Struktur unter `crates/`, modular, gut testbar
- Doc-Kommentare (`///`) für alle öffentlichen Items

### Commits

- **Format:** Conventional Commits (https://www.conventionalcommits.org/)
- Beispiele: `feat(provisioner): add tier sync endpoint`, `fix(bot-faq): correct German umlaut handling`, `docs(readme): clarify license sections`
- **Signing:** alle Commits SSH-signiert
- **Force-Push:** verboten auf `main`
- **Push:** auf `main` nur über reviewte Pull Request

### Branches

- `main` ist der stabile Branch
- Feature-Branches: `feat/<kurzbeschreibung>`
- Fix-Branches: `fix/<kurzbeschreibung>`
- Documentation-Branches: `docs/<kurzbeschreibung>`

### Pull Requests

- Klare Beschreibung was sich ändert und warum
- Verlinkung zu relevanten Issues
- Tests für neue Funktionen
- Acceptance-Test-Report im PR-Body, sobald Tests laufen
- Mindestens ein Review vor Merge

---

## Tests

- `cargo test` muss grün sein vor jedem Commit
- Integration-Tests in `tests/`-Verzeichnis pro Crate
- Mock-Matrix-Server für Application-Service-Tests (z.B. `matrix-sdk-appservice` Test-Helpers)
- End-to-End-Tests gegen eine echte Tuwunel-Instanz im CI-Setup, sobald CI eingerichtet ist

---

## Sicherheit

Findet Ihr eine Sicherheitslücke, meldet sie **nicht öffentlich** als GitHub-Issue. Stattdessen:

- E-Mail an **security@imogo.de**
- Verschlüsselte Übermittlung mit dem PGP-Key, der auf imogo.de hinterlegt ist (sobald Webseite live)
- Verantwortliche Offenlegung mit angemessener Frist zur Behebung

Wir reagieren innerhalb von 7 Tagen mit einer ersten Einschätzung.

---

## Verhaltenskodex

Beiträge in jedem Format (Issues, Pull Requests, Diskussionen) folgen einem einfachen Grundsatz:

- **Respektvoller Umgang.** Sachliche Kritik an Code ist erwünscht, persönliche Angriffe nicht.
- **Konstruktiv.** Wer ein Problem meldet, hilft idealerweise auch bei der Lösung mit.
- **Geduldig.** Dieses Projekt wird mit begrenzten Ressourcen gepflegt. Reaktionszeiten sind kein Service-Level.

Verstöße führen zu einer Verwarnung. Bei Wiederholung Ausschluss aus dem Projekt.

---

## Kontakt für Contributors

- **GitHub Issues:** für Bug Reports und Feature Requests
- **Pull Requests:** für Code-Änderungen
- **E-Mail an info@imogo.de:** für allgemeine Fragen, die kein öffentliches Issue benötigen

---

## Vielen Dank

Jeder Beitrag, der die Matrix-Kommunikation für imogo-Nutzerinnen und -Nutzer besser macht, ist willkommen. Auch kleine Verbesserungen an Konfiguration oder Dokumentation sind wertvoll.
