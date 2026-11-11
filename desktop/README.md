# TBS inform CRM, native client

The desktop client in Rust: iced for the interface, WebView2 only for the
Telegram section, whose page is built from `frontend/` and embedded at compile
time. The Electron client in `frontend/` stays until the switch.

The program is called TBS inform CRM wherever a person reads its name. The
executable, the data folder, the keyring entry and the setup's file name keep
`tbsparcer`: installed clients find their settings and their updates by them.

- `crates/tbs-core`: API, cache, exports, no interface. A future CRM can reuse it.
- `crates/tbs-app`: the program.
- `installer/`, `tools/build-installer.ps1`: the setup.

## Requirements

Rust on the `stable-x86_64-pc-windows-gnu` toolchain, MinGW with `windres`,
Node, Inno Setup (`iscc`). Keep `CARGO_TARGET_DIR` outside the repository.

## Build and test

    npm --prefix ../frontend run build:telegram
    cargo run -p tbs-app
    cargo test -p tbs-core
    cargo test -p tbs-app --features devtools

`--features devtools` adds the `TBS_AUTOLOGIN`, `TBS_GOTO`, `TBS_OPEN`, `TBS_SHOT`
and `TBS_CMD` hooks. Release builds do not have them.

`live_writes` (`TBS_E2E_WRITES=1`) and the `live_ui` flow (`TBS_E2E_LOGIN=<login.json>`)
change data. Run them against a local copy of the database, never the server.

## Setup

    pwsh tools/build-installer.ps1

The result is `C:\Dev\tmp\tbs-installer\TBSparcer-Native-Setup-<version>.exe`. It
installs per user, without elevation, which an update started from inside the
program relies on. It contains `frontend/client-access.json` with the system
access token, so it must be handled like the Electron setup and never committed.

The client looks for updates in `asintiko/parcer20-updates` and takes only an
asset named `TBSparcer-Native-Setup-*.exe`, so the Electron releases there do not
reach it. The Telegram section needs the WebView2 runtime on the machine.
