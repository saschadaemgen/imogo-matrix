# imogo-matrix - Claude Code Instructions

## Project

imogo-matrix is the Matrix-based communication layer for the imogo e-invoicing platform. It contains the `imogo-provisioner` Application Service, helper bots, server configuration templates, and deployment scripts. This repository complements the main imogo product (Tauri desktop app, separate private repository) and the imogo-web repository (imogo.de website).

- Target market: customers and end-customers of the imogo platform (Germany, B2B and B2C)
- Purpose: Matrix-based premium support, moderated community, optional end-customer channel
- Server: Tuwunel (Rust-based Matrix homeserver, Apache-2.0)
- Federation strategy: asymmetric. matrix.imogo.de (B2B) is closed. matrix.endkunden.imogo.de (B2C) is open with blacklist.
- Audio/Video: LiveKit + matrixRTC at matrix-rtc.imogo.de
- Hosting: Hetzner Online GmbH, Germany (Falkenstein/Nürnberg, ISO 27001)
- Author: Sascha Daemgen, IT and More Systems, Recklinghausen
- Repository: github.com/saschadaemgen/imogo-matrix (private until first production code is reviewed by counsel)

## License Strategy (binding)

This repository uses a tiered license strategy:

| Path | License | SPDX |
|---|---|---|
| `crates/imogo-provisioner/` | GNU Affero General Public License v3.0 or later | `AGPL-3.0-or-later` |
| `bots/` | GNU Affero General Public License v3.0 or later | `AGPL-3.0-or-later` |
| `deploy/` | MIT License | `MIT` |
| `scripts/` | MIT License | `MIT` |
| `docs/`, root markdown files | Creative Commons Attribution 4.0 International | `CC-BY-4.0` |

Shared infrastructure crates (`sdx-*`) come from the main imogo repository under dual license `Apache-2.0 OR AGPL-3.0-or-later`. The provisioner crate selects the AGPL path automatically when linking, keeping everything consistent.

The main imogo product (Tauri desktop application) and all `imogo-*` business-logic crates remain proprietary closed source in their own private repository. They are NOT part of this repository and are NOT linked from here.

## Design Philosophy

imogo-matrix is the open-source piece of the otherwise proprietary imogo platform. It is the part that touches user communication and therefore benefits most from public scrutiny and transparency. The AGPL ensures that anyone running a modified version as a network service must release the modifications.

Key principles:

- ALWAYS use the latest, non-deprecated APIs from matrix-rust-sdk, matrix-sdk-appservice, axum, tokio
- NEVER use deprecated methods or legacy patterns
- NEVER compromise on E2E encryption, federation isolation, or DSGVO compliance
- The provisioner is the only entry point for license-driven account lifecycle. License logic itself lives in the main imogo cloud backend (separate repository).
- Bots are isolated processes, each in its own crate, each with its own Matrix account
- Reference implementation: matrix-rust-sdk examples, Synapse module patterns, Element Web

## Components

1. **imogo-provisioner** (`crates/imogo-provisioner/`) - Application Service that creates Matrix accounts on license activation, manages tier-based room access, handles expiry and read-only modes
2. **faq-bot** (`bots/faq-bot/`) - Community helper bot for FAQ responses
3. **moderation-bot** (`bots/moderation-bot/`) - Pin, mute, kick tools for community moderation
4. **support-bot** (`bots/support-bot/`) - Helper for premium support rooms (SLA tracking, file routing)
5. **deploy/** - Tuwunel and nginx configuration templates, Docker Compose files, LiveKit config (reference, no secrets)
6. **scripts/** - Setup and operational scripts (server bootstrap, key rotation, backup checks)

## Development Environment

**IMPORTANT: development happens on Windows (PowerShell), like the rest of imogo.**

- OS: Windows 11
- IDE: VS Code
- Shell: PowerShell 7
- Rust: 1.94.1 (Windows native via rustup)
- Cargo edition: 2024
- Async runtime: tokio
- HTTP framework: axum
- Matrix SDK: matrix-rust-sdk (latest stable)
- Application Service crate: matrix-sdk-appservice
- Git: 2.49 with SSH-signing via key `github_sd`
- GitHub CLI: 2.76
- Build command: `cargo build --workspace` (in PowerShell)

Docker is used for: local Tuwunel testing, integration tests against a real homeserver. The production servers run on Hetzner and are managed separately, NOT from this repo's CI.

## Workspace Structure

```
C:\Projects\imogo-matrix
├── CLAUDE.md                   # This file
├── README.md                   # Public repository readme (CC-BY-4.0)
├── CONTRIBUTING.md             # Contribution guide (CC-BY-4.0)
├── LICENSE                     # AGPL-3.0-or-later (full text, applies to code)
├── LICENSE-MIT                 # MIT (applies to deploy/ and scripts/)
├── LICENSE-CC-BY-4.0          # CC-BY-4.0 (applies to docs/)
├── Cargo.toml                  # Workspace manifest
├── .gitignore
├── .editorconfig               # (later)
├── .vscode/                    # (later)
├── .claude/
│   └── settings.local.json     # Claude Code permissions
├── .github/                    # (later)
│   └── workflows/
├── crates/
│   └── imogo-provisioner/      # Application Service (AGPL-3.0-or-later)
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
├── bots/                       # AGPL-3.0-or-later
│   ├── faq-bot/
│   ├── moderation-bot/
│   └── support-bot/
├── deploy/                     # MIT
│   ├── LICENSE.md              # Sub-license marker
│   ├── tuwunel/
│   ├── nginx/
│   ├── livekit/
│   └── docker-compose/
├── scripts/                    # MIT
│   └── LICENSE.md              # Sub-license marker
├── docs/                       # CC-BY-4.0
│   └── LICENSE.md              # Sub-license marker
└── briefings/
    └── archive/                # Completed briefing summaries and test reports
```

## Rules (NON-NEGOTIABLE)

### Git

- Conventional Commits ONLY: `feat(scope): description`, `fix(scope): description`
- Valid types: feat, fix, docs, test, refactor, ci, chore, build, style, perf
- Valid scopes: provisioner, bot-faq, bot-mod, bot-support, deploy, scripts, docs, ci, deps
- All commits SSH-signed with key `github_sd`
- NEVER push to remote without explicit permission - all work stays local by default
- NEVER use `git push --force` or `git push -f`
- NEVER change version numbers (Cargo.toml `version = "0.1.0"` etc.) without explicit permission

### Code Style - Rust

- Target Rust edition 2024
- Use `tracing` crate for logging - NEVER `println!` for production output
- Handle all errors explicitly with `anyhow` at boundaries or `thiserror` in libraries - NEVER use `.unwrap()` in library code
- Use `zeroize` for all sensitive material (access tokens, registration secrets, signing keys)
- Run `cargo fmt` before every commit
- Run `cargo clippy --all-targets --all-features -- -D warnings` before every commit
- Write doc comments (`///`) for all public items
- NEVER use placeholder or demo data - ask if values are unknown
- License header (SPDX) at top of every Rust source file:
  ```rust
  // SPDX-License-Identifier: AGPL-3.0-or-later
  // Copyright (C) 2026 Sascha Daemgen, IT and More Systems
  ```

### Code Style - Configuration and Scripts

- TOML for application config (Tuwunel, Cargo)
- YAML for Docker Compose and LiveKit
- PowerShell for Windows scripts, bash for Linux operations on the server
- All scripts idempotent where possible
- No secrets in committed files. Use `.env.example` as template, real `.env` is git-ignored.

### Code Style - General

- NEVER use em dashes - use regular hyphens or rewrite the sentence
- All code, comments, commits, variable names, technical documentation in English
- User-facing text in bots (room messages, command responses) in German primary, English secondary
- NEVER use placeholder or demo data in production code - ask if values are unknown
- NEVER commit secrets, API keys, registration tokens, signing keys, or passwords
- Add TODO comments with ticket/issue references where applicable

### Architecture

- The provisioner is a Matrix Application Service registered against the homeserver via `registration.yaml`
- Application Service tokens are loaded from environment variables, never hardcoded
- Each bot is an independent process with its own Matrix account, its own access token, its own crate
- Communication between provisioner and the main imogo cloud (license server, separate repository) happens via signed HTTP requests with Ed25519 verification
- The provisioner does NOT run on the same server as the homeserver. It runs as a separate service, typically on the same VPS but in its own container.
- NEVER bypass the homeserver's matrix-rust-sdk client - all Matrix interaction goes through the SDK
- NEVER store user credentials. The provisioner generates one-time login tokens and hands them to the imogo desktop client.

### Security (Non-Negotiable)

- E2E encryption (Olm/Megolm) is enabled by default for all rooms created by the provisioner
- Server-Server federation TLS 1.3+ only
- Application Service registration secret is rotated on every deployment
- Bot access tokens stored in OS keyring (during development) or Hetzner Secret Manager (in production), never in repository
- Audit log entry for every provisioner action (account create, room invite, tier change, deactivate)
- Hash-chained audit log so tampering is detectable
- Rate limiting on the provisioner's REST API (license server is the only legitimate caller)
- CORS locked down: only the imogo cloud backend's IP range. No wildcards.
- All inbound webhooks from the license server are Ed25519-signed and verified before processing

### Federation Policy

- matrix.imogo.de (B2B): closed federation, allowlist contains only matrix.imogo.de itself and matrix.endkunden.imogo.de
- matrix.endkunden.imogo.de (B2C): open federation with blacklist of known spam/anonymous-service homeservers
- Federation policy is set in the Tuwunel TOML config in `deploy/tuwunel/`
- The provisioner does NOT manage federation policy. It only manages users and rooms.
- When in doubt about whether a domain should be allowed: deny by default, ask Sascha.

### License Lifecycle

- License activation (paid) -> imogo cloud backend -> provisioner webhook -> account creation + room invite
- License expiration -> 90 days read-only mode -> account deactivation
- License downgrade (e.g. Pro -> Solo) -> tier-based room permissions adjusted
- License upgrade -> additional rooms granted
- Customer data export possible at any point via Matrix client's standard export feature
- Account deactivation does NOT delete encrypted message history. The user retains the ability to decrypt their own past messages with their own keys.

### Data Locations

In production (Hetzner VPS):

- Tuwunel B2B database: `/var/lib/tuwunel-imogo/`
- Tuwunel B2C database: `/var/lib/tuwunel-endkunden/`
- LiveKit data: `/var/lib/livekit/`
- Provisioner data: `/var/lib/imogo-provisioner/` (audit log, idempotency cache)
- Logs: `/var/log/imogo-matrix/`
- Configurations: `/etc/imogo-matrix/`
- Secrets: not in repository, managed via systemd-creds or external secret store

In development (Windows local):

- Cargo target: `C:\Projects\imogo-matrix\target\` (gitignored)
- Local test database: `C:\Users\<user>\AppData\Local\imogo-matrix-dev\` (if ever needed)

## Build and Test

```powershell
# In PowerShell, from C:\Projects\imogo-matrix

# Build everything
cargo build --workspace

# Build only the provisioner
cargo build -p imogo-provisioner

# Run all tests
cargo test --workspace

# Lint
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check

# Format
cargo fmt
```

## Briefing Workflow

Claude Code receives structured briefings via markdown files. Each briefing has the format `Briefing-NN-Topic.md` and contains:

1. Goal and context
2. Files to read before making changes
3. Task list in execution order
4. Detailed implementation with exact file contents where needed
5. Acceptance tests with verifiable checkpoints (Claude Code runs these autonomously)
6. Scope boundaries (what NOT to do)
7. Commit message in Conventional Commits format

After completing a briefing, Claude Code:

1. Runs ALL acceptance tests autonomously and produces a test report
2. Writes the test report and a completion summary into `briefings/archive/Briefing-NN-Summary.md`
3. Stages and commits the changes locally with SSH-signed commit
4. Waits for explicit "push" instruction before pushing to remote

## Acceptance Test Discipline

This is the same rule as in the main imogo repository:

- Claude Code runs acceptance tests WITHOUT waiting for the human to click through anything that can be automated
- Test report is mandatory BEFORE the completion summary
- If a test fails, Claude Code investigates, fixes, re-runs, and only then summarizes
- If a test cannot be automated (e.g. requires a real running Tuwunel federation handshake), Claude Code documents that explicitly and proposes a manual verification step

## Known Issues

No known issues yet. Update this section whenever a workaround is introduced.

## Current State (Matrix-Season 1 - Starting)

### Completed (Server Infrastructure, by Sascha)

- [x] VPS provisioned at Hetzner (194.164.197.247, Debian Trixie)
- [x] Tuwunel B2B running at matrix.imogo.de (port 6167)
- [x] Tuwunel B2C running at matrix.endkunden.imogo.de (port 6168)
- [x] LiveKit + lk-jwt-service running at matrix-rtc.imogo.de
- [x] nginx reverse proxy with Let's Encrypt auto-renewal
- [x] Well-Known discovery configured at imogo.de
- [x] Admin accounts created on both servers
- [x] Token-based registration enabled on B2B server, registration locked on B2C server
- [x] First test users on B2B server

### In Progress

- [ ] Repository initialization with license structure (Briefing 01)

### Not Started

- [ ] Provisioner Application Service (Briefing 02 onwards)
- [ ] License server webhook integration
- [ ] FAQ bot
- [ ] Moderation bot
- [ ] Support bot
- [ ] Federation allowlist verification on B2B
- [ ] Federation blacklist setup on B2C
- [ ] Production deployment automation
- [ ] Backup automation
- [ ] Monitoring (Prometheus/Grafana)
- [ ] CI/CD pipeline (GitHub Actions)

## Key Resources

- matrix-rust-sdk: https://github.com/matrix-org/matrix-rust-sdk
- matrix-sdk-appservice docs: https://docs.rs/matrix-sdk-appservice
- Tuwunel: https://github.com/matrix-construct/tuwunel
- Matrix Application Service spec: https://spec.matrix.org/latest/application-service-api/
- Matrix Client-Server spec: https://spec.matrix.org/latest/client-server-api/
- LiveKit docs: https://docs.livekit.io
- matrixRTC MSC4143: https://github.com/matrix-org/matrix-spec-proposals/pull/4143
- Element Call: https://github.com/element-hq/element-call
- axum docs: https://docs.rs/axum
- tokio docs: https://docs.rs/tokio
- AGPL-3.0 full text: https://www.gnu.org/licenses/agpl-3.0.txt

## Related Projects

**imogo** (private, github.com/saschadaemgen/imogo):
- Main e-invoicing desktop application (Tauri + Rust + Svelte 5)
- Provides the `sdx-*` shared crates under dual `Apache-2.0 OR AGPL-3.0-or-later`
- License server in `cloud/license-server/` is the only legitimate caller of this repository's provisioner webhook
- Same author, same team, same security principles

**imogo-web** (private, github.com/saschadaemgen/imogo-web):
- imogo.de marketing website, webshop, customer portal
- Hosts the AVV click-acceptance flow that triggers end-customer channel activation
- Hosts kontakt.imogo.de end-customer onboarding landing page

**SimpleGoX** (github.com/nicokimmel/SimpleGoX):
- Multi-Messenger desktop platform
- Shares `sdx-*` crates with imogo
- Same Tauri + Rust + Svelte 5 stack
- Same hardware platform (Phase 3)

## Multi-Chat Project Architecture

The imogo project is developed across three parallel Claude chats. Each chat has its own role and its own repository. This repository (imogo-matrix) is owned by the **Matrix-Chat (Neo)**.

The other two chats are:

- **Master-Chat (Prinzessin Luna)**: main imogo repository (Tauri app, cloud backend)
- **Webseite-Chat (persona TBD)**: imogo-web repository (imogo.de website)

For cross-chat coordination see the project shared documentation: `Projekt-Regeln.md`, `imogo-Gesamtkonzept.md`, `Update-Memo-*.md`. The shared documentation lives in a project folder visible to all three chats, NOT in this repository.

When working on a briefing, Claude Code does not need to coordinate with other chats. The briefings are pre-coordinated by Neo before being handed over.
