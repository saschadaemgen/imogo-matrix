# imogo-matrix

**Matrix-Kommunikationsschicht für die imogo-Plattform**

Dieses Repository enthält den Application Service `imogo-provisioner`, Bots, Server-Konfigurationen und Deployment-Scripts für die Matrix-basierte Kommunikationsschicht von [imogo](https://imogo.de).

imogo ist eine deutsche Buchhaltungs- und E-Rechnungs-Software für KMU, Handwerker und Solo-Selbstständige. Die Matrix-Komponenten in diesem Repository ermöglichen drei Anwendungsfälle:

- **Premium-Support 1:1** für lizenzierte imogo-Kundinnen und -Kunden
- **Moderierte Community** mit thematischen Räumen
- **Optionaler Endkundenkanal** für Anwender, die mit ihrer eigenen Kundschaft über Matrix kommunizieren möchten

Die Server-Infrastruktur basiert auf [Tuwunel](https://github.com/matrix-construct/tuwunel), einem Rust-basierten Matrix-Homeserver. Audio- und Videoanrufe laufen über LiveKit als matrixRTC-Backend.

---

## Repository-Struktur

```
imogo-matrix/
├── crates/
│   └── imogo-provisioner/    Application Service (AGPL-3.0-or-later)
├── bots/                       Matrix-Bots (AGPL-3.0-or-later)
│   ├── faq-bot/               FAQ-Bot für Community-Räume
│   ├── moderation-bot/        Moderationswerkzeuge
│   └── support-bot/           Support-Helfer-Bot
├── deploy/                     Deployment- und Setup-Material (MIT)
│   ├── tuwunel/               Tuwunel-Konfigurations-Templates
│   ├── nginx/                 nginx-Reverse-Proxy-Configs (Referenz)
│   ├── livekit/               LiveKit-Konfiguration
│   └── docker-compose/        Docker-Compose-Vorlagen
├── scripts/                    Setup-Scripts (MIT)
├── docs/                       Dokumentation (CC-BY-4.0)
├── LICENSE                     AGPL-3.0-or-later (Code)
├── LICENSE-MIT                 MIT (Operations-Material)
├── LICENSE-CC-BY-4.0           CC-BY-4.0 (Doku)
├── README.md                   Dieses Dokument (CC-BY-4.0)
└── CONTRIBUTING.md             Contribution-Regeln
```

---

## Lizenz-Übersicht

Dieses Repository nutzt eine **gestaffelte Lizenz-Strategie**, um den Charakter der jeweiligen Komponente abzubilden.

| Bereich | Pfad | Lizenz | SPDX |
|---|---|---|---|
| Provisioner Application Service | `crates/imogo-provisioner/` | GNU Affero General Public License v3.0 oder später | `AGPL-3.0-or-later` |
| Bots | `bots/` | GNU Affero General Public License v3.0 oder später | `AGPL-3.0-or-later` |
| Server- und Deployment-Konfiguration | `deploy/` | MIT License | `MIT` |
| Setup-Scripts | `scripts/` | MIT License | `MIT` |
| Dokumentation | `docs/`, `README.md`, `CONTRIBUTING.md` | Creative Commons Attribution 4.0 International | `CC-BY-4.0` |

Volltexte:

- AGPL-3.0-or-later siehe [LICENSE](LICENSE)
- MIT siehe [LICENSE-MIT](LICENSE-MIT)
- CC-BY-4.0 siehe [LICENSE-CC-BY-4.0](LICENSE-CC-BY-4.0)

### Warum AGPL für den Provisioner und die Bots

Der Provisioner ist ein netzwerkbasierter Dienst. Die AGPL stellt sicher, dass Modifikationen, die irgendwo öffentlich oder als Service betrieben werden, ebenfalls offengelegt werden müssen. Das schützt die Community und uns gleichermaßen vor "Take-without-Give-Back"-Forks.

### Warum MIT für Operations-Material

Server-Konfigurationen, Docker-Compose-Files und Setup-Scripts sind Standard-Operations-Material. Eine permissive Lizenz wie MIT senkt die Hürde für Selbsthoster und Mit-Operatoren, ohne dass dadurch Geschäfts-IP berührt würde.

### Warum CC-BY-4.0 für Dokumentation

Dokumentation ist kein Code, sie ist Wissen. CC-BY-4.0 ist die etablierte Lizenz für offene Dokumentation und erlaubt Weitergabe und Bearbeitung unter Namensnennung.

### Was NICHT in diesem Repository liegt

Das Hauptprodukt **imogo eRechnung** (die Tauri-Desktop-Anwendung), das **imogo Cloud-Backend** (Lizenz-Server, KI-Backend, Backup-Service) und alle **imogo-spezifischen Crates** (Buchhaltungslogik, ZUGFeRD-Generierung, GoBD-Archiv, DATEV-Export) sind **proprietäre Closed-Source-Software** und liegen in einem separaten, privaten Repository. Diese Komponenten werden vom Provisioner in diesem Repository nicht eingebunden, weil sie für die Matrix-Kommunikation nicht benötigt werden.

Geteilte Infrastruktur-Bibliotheken (`sdx-*`-Crates aus dem Hauptprojekt) sind unter der **dualen Lizenz `Apache-2.0 OR AGPL-3.0-or-later`** verfügbar. Das Provisioner-Crate dieses Repositories wählt beim Linken automatisch den AGPL-Pfad, sodass alles in sich konsistent ist.

---

## Server-Architektur

| Domain | Funktion | Föderation |
|---|---|---|
| `matrix.imogo.de` | B2B-Homeserver für Premium-Support und Community | Closed (nur eigene Server) |
| `matrix.endkunden.imogo.de` | B2C-Homeserver für Endkundenkanal | Open mit Blacklist |
| `matrix-rtc.imogo.de` | LiveKit-Backend für Audio und Video (matrixRTC) | n/a |
| `imogo.de` | Well-Known Discovery für Matrix | n/a |

Alle Server laufen in Deutschland bei Hetzner Online GmbH (Falkenstein/Nürnberg, ISO-27001-zertifiziert).

Die Föderationsstrategie ist asymmetrisch: zahlende Lizenzkundschaft kommuniziert in einer geschlossenen B2B-Welt, während der optionale B2C-Endkundenkanal offene Föderation erlaubt, damit Endkunden mit bereits vorhandenen Matrix-Accounts unkompliziert anschließen können. Bekannte Spam- und Anonymdienst-Server sind per Blacklist ausgeschlossen.

---

## Status

Dieses Repository befindet sich in der initialen Aufbau-Phase. Server-Infrastruktur ist live, Provisioner und Bots werden in den kommenden Sprints implementiert.

---

## Verwandte Repositories

- **imogo** (privat) - Hauptprodukt, Tauri-Desktop-Anwendung
- **imogo-web** (privat) - imogo.de Webseite und Webshop

---

## Kontakt

**Anbieter:** Sascha Daemgen IT and More Systems
Am Neumarkt 22, 45663 Recklinghausen, Deutschland
info@imogo.de
+49 2361 9702434

**Bei Fragen zu diesem Repository:** GitHub-Issues bevorzugt.

**Bei Sicherheitsmeldungen:** bitte nicht öffentlich. E-Mail an security@imogo.de mit verschlüsselter Übermittlung (PGP-Key auf imogo.de hinterlegt, sobald Webseite live).

---

## Mitwirken

Siehe [CONTRIBUTING.md](CONTRIBUTING.md) für Hinweise zu Code-Stil, Commit-Konventionen und Lizenz-Verständnis.
