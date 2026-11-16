# TBS inform CRM

A desktop CRM for Windows written in Rust, with a Python backend and a Rust
attendance server. Repository: `ProfitubeYt/crm-rust`, default branch `main`.

## Components

| Path | What it is | Language |
|---|---|---|
| `desktop/` | The client people use: Hub, Checks, Telegram, Time and CRM administration. `crates/tbs-core` (API, cache, exports), `crates/tbs-app` (the program), `installer/`, `tools/` | Rust (iced) |
| `backend/` | The server: accounts and sessions, Checks and parsers, Excel synchronization with its VBA sources, Telegram/TDLib, the TBS Auth bot, the administration panel, the Time gateway, migrations | Python (FastAPI) |
| `time/` | The Time server and its page. The only Time tree | Rust (axum, SQLite) |
| `android/` | SMS ingest application | Kotlin |
| `deploy/`, `docker-compose.yml` | Caddy routes, the Time compose overlay, pre/post-check SQL, server helpers | |
| `scripts/hub/` | Test runners and the local end-to-end stack | |

There is no React, Electron, Node or npm build in this repository. The Time and
administration pages and the Telegram authorization Mini App are plain
HTML/CSS/JavaScript files that the Rust client embeds or the servers serve.

## Build and test

Desktop client, on Windows (Rust `x86_64-pc-windows-gnu`, MinGW with `windres`,
Inno Setup for the setup), from `desktop/`:

    cargo test --workspace --locked
    cargo build -p tbs-app --release --locked
    pwsh tools/build-installer.ps1

Desktop client, on Linux in the pinned container (tests and the Windows
cross-build; it proves the logic, not the Windows program):

    scripts/hub/desktop-tests.sh
    scripts/hub/desktop-tests.sh build -p tbs-app --locked --target x86_64-pc-windows-gnu

Backend (Python 3.11, SQLite in memory, no network):

    scripts/hub/backend-tests.sh

Time, from `time/`:

    cargo test --workspace
    docker build -t timelist:local .

Backend, PostgreSQL, Redis, Caddy and Time together: `scripts/hub/e2e/run.sh`.

GitHub Actions, both started by hand: `windows-client.yml` (native Windows
tests, release build, setup, install and first start) and `time-image.yml`
(backend tests and the Time image).

## Configuration

- `.env.example`: every server variable. Copy to `.env`; never commit it.
- `security/root-access.server.example.json`: the server's bootstrap access file.
- `security/client-access.example.json`: the client's first-install file. The real
  `security/client-access.json` is private: git ignores it, the setup includes it
  when it is present, and such a setup is never published or uploaded by CI.
- `time/.env.example`: the Time server.

## Documents

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): how the parts fit, test layers, constraints.
- [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md): production layout, rollout, backups, rollback.
- [docs/HANDOFF.md](docs/HANDOFF.md): current state and the prompt for the next session.
- [OPEN_TASKS.md](OPEN_TASKS.md): what is not built yet.
- [desktop/README.md](desktop/README.md), [time/README.md](time/README.md),
  [docs/EXCEL_SYNC.md](docs/EXCEL_SYNC.md): details of each part.

The repository as it was before the cleanup of 2026-11-16 (the React/Electron
client, older notes and plans) is the tag `pre-cleanup-2026-11-16`.
