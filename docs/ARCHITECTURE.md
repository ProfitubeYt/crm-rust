# Architecture

What the system is made of, where each part lives and what a developer would
otherwise rediscover the hard way. Build and test commands are in the
[root README](../README.md), [desktop/README.md](../desktop/README.md) and
[time/README.md](../time/README.md). Production facts are in
[DEPLOYMENT.md](DEPLOYMENT.md), open work in [OPEN_TASKS.md](../OPEN_TASKS.md).

## Components and canonical paths

| Path | What it is |
|---|---|
| `desktop/` | The product people use: «TBS inform CRM», a Rust desktop client for Windows. `crates/tbs-core` (API client, cache, exports; no interface), `crates/tbs-app` (the iced program), `installer/` and `tools/` (Inno Setup, Linux window checks). |
| `backend/` | Python FastAPI server with PostgreSQL and Redis: accounts and sessions, Checks (receipts, parsers, annotations, dictionaries), Excel synchronization (`api/routes/excel_sync.py`, VBA sources in `tools/excel_sync`), Telegram/TDLib, the TBS Auth bot, the CRM administration panel (`admin_panel/`), the Time gateway, Celery workers, Alembic migrations. |
| `time/` | The Time server («Тайм-лист», attendance from a Telegram work group): Rust, axum, SQLite, its own bot. Its hosted page is `time/crates/server/assets`. **`time/` is the only Time tree.** |
| `android/` | SMS ingest application. |
| `deploy/` | `Caddyfile`, `time-hub/` (Time compose overlay, read-only pre/post-check SQL) and server-side helpers. |
| `scripts/hub/` | Local test runners and the end-to-end stack (`e2e/`: PostgreSQL, Redis, Caddy, backend, the real Time server, a stand-in Telegram). |
| `.github/workflows/` | `windows-client.yml` (native Windows build, tests, setup) and `time-image.yml` (backend tests, Time image). Both start by hand. |

The backend stays Python. Rust is the desktop client and the Time server.

## How the parts talk

```
desktop client ──TLS──> Caddy ──> backend (FastAPI) ──bearer──> Time (internal only)
                                   │ users, sessions, workspaces      │ SQLite volume
                                   └── /internal/time/* <──hub key────┘ bot ──> Telegram
```

- **One account directory.** The backend's `users` table is the only place with
  credentials. No workspace keeps its own; Time's shared password is off in Hub mode.
- **Three independent decisions per account**, all on the server: may the account
  be used at all; is it a CRM administrator (`users.global_admin`); which
  workspaces it opens and as what (Checks: `operator`/`admin` in `role` with
  `checks_access`; Time: `time_role` `employee`/`viewer`/`manager`). CRM
  administration opens no workspace, and no workspace role reaches an
  administration route. The last active CRM administrator cannot be removed.
- **Workspace registry:** `backend/services/workspace_access.py`. Sign-in,
  `/api/auth/me`, the path check, the admin API and the panel read it. A request
  path that no workspace claims counts as Checks, so a new Checks endpoint is
  protected by default. Adding a workspace is one registry entry, one nullable
  role column, routes under its own prefix and a tile in the Hub.
- **Paths with their own authentication** carry the workspace check explicitly: the
  Excel protocol, the Telegram WebSocket, background agent runs.
- **Backend → Time:** the gateway `/api/time/*` requires a session, the Time
  workspace and a role allowed for that exact route (allowlist, default deny). It
  calls Time with a private token and sets the journal's user name itself. Time is
  never published by Caddy.
- **Time bot → backend:** `/internal/time/telegram/{id}`, outside `/api` so Caddy
  cannot route to it, with its own key. The bot asks for every message, uncached;
  if the backend is unreachable nothing is accepted and the update is retried.
  Identity is Telegram's numeric sender ID, never a display name; only configured
  groups are read, and an empty list accepts none.
- **Caddy** adds the `X-System-Access` header for `/api/*` and `/mini-app*`.

## The desktop client

- Every launch opens the start window (Hub) and lists only the workspaces the
  server grants. One sign-in serves all of them; tokens stay inside `tbs-core::api`.
- **Checks** is native iced with a left rail.
- **Time** and **CRM administration** are hosted pages: the files of
  `time/crates/server/assets` and `backend/admin_panel` are compiled into the
  client and shown in WebView2 under one slim native bar (home, workspace name,
  connection mark, account, sign-out) with no rail. The page holds no credential:
  its requests go through the client's proxy, limited to that workspace's API
  prefix. The same panel is also served by the backend at `/admin/`.
- **Telegram** is native (being finished on 2026-11-16, see [HANDOFF.md](HANDOFF.md)). A WebView2 appears there only to show a third-party
  Telegram Mini App.
- The Telegram authorization Mini App is the inline page served by
  `backend/api/main.py` at `/mini-app`; it is not part of the desktop client.
- The local cache is one SQLite file per user and host; a revoked workspace
  disappears from the Hub on the next request and its cached content is cleared.
- Interface text is Russian.

### Identity that must not change

The executable, the data folder, the keyring entry and the setup's file name keep
**`tbsparcer`** (`TBSparcer-Native-Setup-<version>.exe`). Installed clients find
their settings, saved session and updates by these names. The update check reads
releases of `asintiko/parcer20-updates` and accepts only an asset with that setup
name. The setup installs per user without elevation; in-program updates rely on it.

### First-install provisioning

A first installation needs the private `client-access.json` (system-access
configuration) inside the setup. It must never be committed, attached to a public
release or left in a CI artifact. A setup built without it (for example by CI) can
only update a client that already has its own file.

## Sign-in and sessions

- Password sign-in with JWT access and rotating refresh tokens, a Redis session
  registry with revocation, lockout after five wrong passwords, login rate limits.
  The user row is re-read on every request, so changes are live.
- **TBS Auth** is the Telegram side: accounts are linked to a Telegram ID by an
  administrator; the `auth_bot` service is its bot, whose user commands sit behind
  the owner's `AUTH_ADMIN_IDS` allowlist and whose system-wide switches require a
  linked CRM administrator. Scope OTP, Telegram QR login and account-password TOTP
  are three distinct mechanisms in the code. The owner chose Telegram TBS Auth, not
  authenticator enrollment (2026-11-15); scope OTP does not make Telegram a second
  factor on every password login.
- Grants and `global_admin` do not bump `permissions_version`: sessions and Excel
  connections survive and the next request sees the new row. Deactivation, password
  reset and Checks role/tab changes end every session.
- Excel workbooks hold no key: each computer gets its own revocable grant, kept in
  Windows Credential Manager. See [EXCEL_SYNC.md](EXCEL_SYNC.md).

## Test layers — and what each does not prove

| Layer | Proves | Does not prove |
|---|---|---|
| `cargo test` (core, app), backend `pytest` on SQLite | Logic | Anything about PostgreSQL, a window or a real server |
| Live suites (`live_ui`, `live_backend`, `live_writes`) | Integration, **only** when pointed at a local backend | Without a server they return early and report success: not coverage. Never point them at production; they write. |
| `scripts/hub/e2e` | Real backend, PostgreSQL, Redis, Caddy and Time together; migration up/down; release compatibility | Windows, real Telegram, production |
| Linux window (Xvfb, WebKitGTK, `desktop/tools/window-check.py`) | Layout and flows in a real window | WebView2, Windows fonts, DX12, Credential Manager |
| Cross type-check or cross build for `x86_64-pc-windows-gnu`, Wine | That it compiles/links | That it runs on Windows. A Linux check is not a Windows build. |
| `windows-client.yml` on a hosted runner | Native build, tests, setup, silent install, first start | It does not sign in, so no hosted page and no WebView2 behaviour |
| A Windows desktop, signed in | The release | — |

Say which layer a result comes from. Pictures from Linux use other fonts.

## Constraints worth knowing before changing things

- Release builds have no test hooks: `TBS_AUTOLOGIN` and friends exist only with
  `--features devtools`.
- `build.rs` embeds the icon and version resources only for a Windows target and
  needs `windres` (MinGW's cross `windres` on Linux). `secret::tests` in `tbs-core`
  is compiled `not(windows)`, so Windows runs two core tests fewer than Linux.
- A click that opens a hosted page used to land in the page as well; the page
  ignores a pointer click that has no pointer press of its own. Keep that guard.
- A hosted page that polls (`/api/time/meta` every five seconds) counts against
  `RATE_LIMIT_PER_MINUTE` per office address.
- Compare PostgreSQL timezone-aware timestamps only after normalizing to UTC, and
  build bot Unix times independently of the host's zone: both produced real faults
  (a 500 instead of "locked"; tokens already expired on a UTC+5 Windows host).
- Released 2.0.x clients send `role` and know nothing of `global_admin`: demoting
  an administrator through one also removes CRM administration. New clients offer
  Time and administration only when the server says so.
- Migrations are additive and numbered after the deployed head (`20261112_0035`).
  Release 2.0.3 must keep running on a newer schema: that is the first rollback layer.
- The «Операция» column: the transaction API infers a type, workbook snapshots use
  the stored one. Do not claim parity without checking both.
- Excel: generated workbooks keep `vbaProject.bin` and untouched ZIP parts
  byte-for-byte and never contain a plaintext password or bearer token. Real macro
  execution can only be verified in desktop Excel on Windows.
- Receipt enrichment: do not fill absent receipt fields with unrelated values and
  do not learn rules from rule-generated values. The recorded historical replay of
  Details inference was 75 of 79 correct; inference is not certainty.
- Use synthetic data in tests and examples. Real credentials, customer workbooks,
  receipts, dumps and credential-bearing setups are never git content.
