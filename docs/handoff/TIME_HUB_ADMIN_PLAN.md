# Time workspace, unified accounts, online admin and Time bot — implementation plan

Branch `work/time-hub-admin`, started from `main` at `45d7e5d` (release 2.0.2, production schema
`20261112_0033`). Written before any shared-auth change. Task: `docs/tasks/TIME_HUB_ADMIN_PROMPT.md`.
Progress and evidence: `docs/handoff/TIME_HUB_ADMIN_PROGRESS.md`.

Nothing in this plan is deployed. Production deployment and the production schema/permission
migration need the owner's approval of section 9 first.

## 1. What exists (verified by reading the code)

**Time (`time/`, a second identical copy in `time/timemotion/`)** — one Rust program («Тайм-лист»):
`timelist-core` (formulas, message parser, auto-marks, calendar) and `timelist` (axum REST API, embedded
web UI of three static files, Telegram bot on long polling, SQLite file, Excel import/export).

- Accounts: none. One shared `TIMELIST_PASSWORD` (cookie session kept in process memory) and one
  `TIMELIST_API_TOKEN` (bearer). The audit journal takes a free-text name from `X-Timelist-User`.
- Employees are directory rows, not accounts. A Telegram sender is matched by `employees.tg_user_id`;
  while an employee has no ID bound, the bot also matches by Telegram display name.
- Bot commands: `/start`, `/help`, `/id`, `/me` (`/status`). Attendance comes from ordinary group
  messages. Groups are limited by `TELEGRAM_CHAT_ID`; empty means any group. Time zone is the
  `utc_offset_minutes` setting, default 300 (UTC+5). Updates are deduplicated by
  `UNIQUE (chat_id, tg_message_id)`; the polling offset is saved once per batch.
- Not deployed anywhere we know of; no database or workbook is in the repository.

**Checks backend (`backend/`, FastAPI + PostgreSQL + Redis)** — the only real account system:
`users` (argon2id/PBKDF2 hashes, role `admin|operator`, `is_active`, `allowed_tabs`, `allowed_folders`,
`allowed_sources`, `forbidden_periods`, `access_until`, TOTP, lockout, `telegram_id`,
`permissions_version`), JWT access + rotating refresh tokens, a Redis session registry with revocation,
login rate limits, launch gate, system-access gate (Caddy injects `X-System-Access` for `/api/*`).
`get_current_user` re-reads the user row on every request, so row changes are live.
Admin API `/api/admin/users*` (list, create, patch, reset password, reset/force 2FA, unlock, deactivate,
link/unlink Telegram ID) already exists and is used by the desktop and Electron clients and `auth_bot`.

**Desktop (`desktop/`, Rust/iced)** — every launch opens the start window (Hub) listing `Subsystem`s
(only «Чеки» today); one session; tokens never leave `tbs-core::api`; the Telegram section is a WebView
whose requests are proxied through `Api::proxy`. The local cache is one SQLite file per user and host.

## 2. Owner's decisions (answered 2026-11-12)

| Question | Answer | Consequence |
|---|---|---|
| Telegram linking flow | Administrator enters the Telegram ID | Reuse `users.telegram_id` and the existing link/unlink admin endpoints. No codes, no passwords in chats. |
| Bot and group | No bot yet | Owner-side steps are documented; token and group ID arrive only as server secrets. |
| Existing Time data | None, start empty | No data migration. Strict Telegram-ID matching has no history to affect. |
| Who gets Time at migration | Nobody | Migration grants nothing; Time is granted by hand in the admin panel. |

## 3. Account-data ownership

- The backend `users` table is the single owner of accounts, passwords, 2FA, sessions, roles and
  workspace access. Time keeps no credentials of its own in hub mode.
- Time owns attendance data only (employees, messages, marks, periods, calendar, its journal).
- A person is tied together by explicit steps only, never by name:
  account ⇄ Telegram ID (administrator, admin panel) and Telegram ID ⇄ employee (Time manager, the
  existing «Привязка сотрудников» screen). Time has no accounts, so there is nothing to merge.

## 4. Permission model

Two additive columns on `users` (migration `20261112_0034`):

| Column | Type | Meaning |
|---|---|---|
| `checks_access` | `BOOLEAN NOT NULL DEFAULT TRUE` | May use the Checks workspace. The database default keeps every existing account, and any account created by old code or old clients, a Checks account. |
| `time_role` | `VARCHAR(20) NULL`, check `employee/viewer/manager` | `NULL` = no Time access. Never set by the migration. |

Workspace access is a layer above the existing rules, not a replacement: inside Checks the role, tabs,
folders, sources, forbidden periods, scopes, period locks and Excel grants work exactly as before.

Time roles (the project had one shared password, so these are the minimum that centralised accounts need):

| `time_role` | Bot attendance | Time UI/API |
|---|---|---|
| `employee` («Сотрудник») | yes | none (not listed in the Hub) |
| `viewer` («Просмотр») | yes | read-only: month, day, messages, directories, export |
| `manager` («Руководитель») | yes | everything the shared password allowed |

`role = admin` stays what it is: account administration plus the admin role inside Checks. An
administrator uses a workspace only if it is granted like for anyone else, and can grant it to
themselves (audited). The four combinations are plain data: Checks-only (default), Time-only
(`checks_access = false`, `time_role` set), both, neither.

Workspace changes do not bump `permissions_version`: sessions and Excel connections survive, and the
next request already sees the new row. Deactivation, password reset and role/tab changes keep today's
behaviour (version bump, every session ends).

## 5. Architecture and trust boundaries

```
desktop Hub / admin panel ──TLS──> Caddy ──> backend (FastAPI)  ──bearer──> timelist (Rust, internal only)
                                              │  users, sessions, workspaces        │ SQLite volume
                                              └── /internal/time/* <──hub key───────┘ bot ──> Telegram
```

1. **Client → backend.** Unchanged login/JWT/session. `get_current_user` classifies the request path:
   hub-level (`/api/auth`, `/api/2fa`, `/api/admin`, `/api/security/app`, `/api/security/system-status`,
   `/api/security/sessions`), Time (`/api/time`), everything else Checks; a missing workspace is
   `403 {"error":"workspace_forbidden","workspace":…}`. Default is Checks, so a new Checks endpoint is
   covered without anyone remembering to protect it.
2. **Paths that do their own authentication** get the same check explicitly: the Excel protocol
   (`excel_sync._check_user`), the Telegram WebSocket (connect and every loop turn), background agent
   runs (`services/ai_agent/authorization.py`). Machine credentials that belong to no account
   (SMS ingest key, feed/export keys, internal API key) are unchanged.
3. **Backend → Time (gateway `/api/time/*`).** Requires an active session, the Time workspace and a
   role allowed for that exact route (allowlist, default deny). The backend calls Time with
   `TIMELIST_API_TOKEN` over the compose network and sets `X-Timelist-User` itself, so the journal name
   cannot be spoofed. Time's own password login is switched off in hub mode. Time is never published
   by Caddy.
4. **Time bot → backend (`/internal/time/telegram/{id}`).** Outside `/api`, so Caddy cannot route to it;
   it additionally requires a dedicated `TIME_HUB_KEY`, a private source address and no proxy headers.
   The bot asks it for every message and command, uncached: the sender must be linked to an account
   that is active, not expired and has a `time_role`. Backend unreachable ⇒ nothing is accepted and the
   update is retried later (the offset does not advance), so neither a forged nor a lost mark results.
5. **Telegram → bot.** Identity is the numeric sender ID Telegram supplies, never a name. In hub mode:
   only the configured groups are read (an empty list accepts none); bot messages and anonymous
   senders are ignored; a message with a Telegram ID is no longer attributed by display name.
   Duplicates are harmless (upsert) and the offset is saved after every update.
6. **Desktop.** The Time page is embedded like the Telegram one: its files are compiled in, its
   requests go through `Api::proxy` limited to `/api/time/*`, it holds no credential.

## 6. What is reused, what is added

Reused as is: Time's core, API, bot parser, attendance rules, time zone, UI, Excel exchange; the
backend's users, hashing, 2FA, rate limits, sessions, audit log, admin endpoints, `create_admin.py`;
the desktop's start window, session handling and WebView host pattern.

Added: migration 0034; workspace service and enforcement; admin API extensions (workspace grants,
per-user sessions and revocation, last-administrator guards, audit listing); Time gateway and bot
identity endpoint; Time hub mode (config, bot checks, change counter for polling instead of SSE, UI
switches); Russian web admin panel served by the backend at `/admin/`; desktop workspace «Учёт
времени»; Dockerfile/compose/Caddy changes and this plan.

## 7. Integration points with the concurrent receipt-enrichment session

That session owns the application dictionary, receipt details and the Description column. This work
does not touch parsers, receipt processing, transactions routes or annotation services. Shared files
touched, each by a few lines:

- `backend/database/models.py` — two columns in `User`.
- `backend/services/user_service.py`, `backend/api/dependencies.py`, `backend/api/routes/admin.py`,
  `backend/api/routes/auth.py` — snapshot fields, enforcement, admin fields.
- `backend/api/routes/excel_sync.py` (`_check_user`), `backend/api/routes/telegram_client.py`
  (WebSocket auth), `backend/services/ai_agent/authorization.py` — one workspace check each.
- `backend/api/main.py` — router registration and the `/admin` static mount.
- Alembic: `20261112_0034` follows `20261112_0033`. If the other session also adds a revision on 0033,
  whichever merges second changes its `down_revision`; the two are independent (this one alters only
  `users`).

## 8. Acceptance criteria

1. Checks-only, Time-only, both, neither: the Hub lists exactly the granted workspaces; "neither"
   gets a clear explanation; no second login when switching.
2. Direct HTTP calls to a workspace that is not granted fail with 403, including exports, Excel
   protocol, WebSocket and agent runs.
3. Revoking a workspace, deactivating an account or revoking sessions takes effect for a running
   client on its next request; the workspace disappears from the Hub without reinstalling.
4. A deactivated or unlinked account cannot mark attendance through the bot; foreign groups are ignored;
   duplicates and restarts do not double-count.
5. An ordinary user cannot call the admin API; the last administrator cannot be removed; passwords can
   be reset, never read; every admin action is audited without secrets.
6. Existing Checks roles, scopes, Excel sync and the whole existing backend suite still pass.
7. One user's cached data is not shown to another, and is removed when Checks is revoked.
8. Evidence is labelled: unit tests, local integration against real services, simulators, and what
   could not be run (real Windows, real Telegram, production).

## 9. Deployment and permission-migration plan — FOR APPROVAL, not executed

Detailed commands live in `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md` (written with the deployment
artefacts). Outline:

1. **Pre-checks (read-only):** `alembic current` = `20261112_0033`; `SELECT role, is_active, count(*)
   FROM users GROUP BY 1,2`; container health; disk space.
2. **Backups:** `pg_dump` + globals, `.env`, compose, Caddyfile, image IDs → `/opt/parcer-backups/<tag>`.
3. **Secrets (owner, on the server, never in chat/git):** `TIMELIST_API_TOKEN`, `TIME_HUB_KEY`
   (random), `TIME_BOT_TOKEN` (from @BotFather), `TIME_BOT_CHAT_ID` (after `/id` in the group).
4. **Schema:** `alembic upgrade 20261112_0034` — two columns and one check on `users`, no row is
   rewritten, nobody gains or loses access. Post-check: every `checks_access` true, no `time_role`.
5. **Backend image** with the new code for backend, workers, `auth_bot`; **timelist** as a new
   container with its own volume; Caddy gets one extra route (`/admin*` → backend) by reload.
   PostgreSQL, Redis, Caddy data and `funpay-bot` are not recreated.
6. **Health:** `/health`, `/admin/`, login, `/api/time/meta`, bot `getMe` status.
7. **Owner actions:** create the bot, disable privacy mode, add it to the group, grant Time roles and
   enter Telegram IDs in the admin panel, bind employees in «Учёт времени».
8. **Rollback:** stop `timelist`, restore the previous backend image and Caddyfile. The two columns
   are compatible with the old code and stay; the database is not restored, so no receipt is lost.
   Schema downgrade is a separate, optional step.

## 10. Production access for this session

`HOPLITE_SSH_KEY_PATH` is not set in this sandbox, so the shared read-only key is not provisioned here.
Nothing was read from production; the inventory queries above are part of the approved-run pre-checks.
All verification is local, on isolated databases and a stand-in Bot API.
