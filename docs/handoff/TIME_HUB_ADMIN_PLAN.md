# CRM Hub: one account directory, global administration, workspaces (Checks, Time), Time bot — implementation plan

Branch `work/time-hub-admin`, started from `main` at `45d7e5d` (release 2.0.2, production schema
`20261112_0033`). Written before any shared-auth change and revised after the owner's scope correction
of 2026-11-12 (section 0). Task: `docs/tasks/TIME_HUB_ADMIN_PROMPT.md`.
Progress and evidence: `docs/handoff/TIME_HUB_ADMIN_PROGRESS.md`.

Nothing in this plan is deployed. Production deployment and the production schema/permission
migration need the owner's approval of section 9 first.

## 0. Scope correction (owner, 2026-11-12): administration is global

The administration panel is the central place of the whole CRM, not a part of Time. The owner creates
and manages every user there and then grants access to individual workspaces: Checks («Чеки»), Time
(«Учёт времени») and whatever is added later. What this changed in the branch:

| Before the correction (commits up to `35f8138`) | Now |
|---|---|
| "Administrator" was the Checks role `admin`: whoever ran Checks also created users | CRM administration is its own right, `users.global_admin` (migration `20261112_0035`), enforced by `require_global_admin`. A workspace role, the Checks administrator's included, does not give it; holding it opens no workspace. |
| Admin API carried `checks_access` and `time_role` fields | One generic shape: `workspaces: {workspace: role}`. The workspaces, their roles and titles come from one registry (`backend/services/workspace_access.py`). |
| `/api/system-settings` and `/api/security/sessions` were reached by the Checks role `admin` | Both belong to CRM administration: system-wide security switches and everybody's sessions. |
| `/api/auth/me` reported `checks_access`, `time_role` | `workspaces` (what the Hub lists), `workspace_roles` (every role held), `global_admin`. |
| Docs and names read as "Time admin panel" | Panel, API and navigation are named for the CRM: «Администрирование CRM», `/admin/`, `/api/admin/*`. |

Unchanged: the working Time integration (gateway, hub mode, bot checks), one `users` table as the only
account directory, the Checks permissions of every existing account.

Migration from the Time-only implementation: commits before this revision were never deployed and no
released client used the two replaced fields, so there is nothing to convert in production. A local
database already at `20261112_0034` moves on with `alembic upgrade head` (one more additive column).

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

**Backend (`backend/`, FastAPI + PostgreSQL + Redis)** — the only real account system:
`users` (argon2id/PBKDF2 hashes, role `admin|operator`, `is_active`, `allowed_tabs`, `allowed_folders`,
`allowed_sources`, `forbidden_periods`, `access_until`, TOTP, lockout, `telegram_id`,
`permissions_version`), JWT access + rotating refresh tokens, a Redis session registry with revocation,
login rate limits, launch gate, system-access gate (Caddy injects `X-System-Access` for `/api/*`).
`get_current_user` re-reads the user row on every request, so row changes are live.
Admin API `/api/admin/users*` (list, create, patch, reset password, reset/force 2FA, unlock, deactivate,
link/unlink Telegram ID) already exists and is used by the desktop and Electron clients and `auth_bot`.
Before this work the role `admin` meant two things at once: the senior role inside Checks (all data,
all tabs, scopes, period locks) and the right to administer accounts.

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
| Where users are managed | One global panel for the whole CRM | Section 0. CRM administration is separate from every workspace. |

## 3. Account-data ownership

- The backend `users` table is the single account directory of the CRM: identity, password, 2FA,
  sessions, CRM administration and workspace access. No workspace keeps credentials of its own; Time's
  shared password is switched off in hub mode.
- CRM administration (the panel and `/api/admin/*`) owns accounts and grants. A workspace owns its own
  data and its own settings: Checks its receipts, scopes and period locks; Time its employees, marks,
  calendar and journal.
- A person is tied together by explicit steps only, never by name:
  account ⇄ Telegram ID (CRM administrator, admin panel) and Telegram ID ⇄ employee (Time manager, the
  existing «Привязка сотрудников» screen). Time has no accounts, so there is nothing to merge.

## 4. Permission model

Three things are decided about an account, each on the server and each on its own:

| Layer | Stored in | Decides | Enforced by |
|---|---|---|---|
| The account | `is_active`, `access_until`, lockout, session registry | May it be used at all | `get_current_user` / `get_current_app_user` |
| CRM administration | `users.global_admin` (0035) | Accounts, grants, everybody's sessions, system security settings | `require_global_admin` |
| Workspace access | a role per workspace (below) | Which workspaces open, and as what | path → workspace check in `get_current_user`; inside Checks the existing role/tab/scope rules; inside Time the route allowlist |

Neither of the last two implies the other. A CRM administrator with no workspace opens the panel and
nothing else. A Checks administrator or a Time manager without `global_admin` gets `403
global_admin_required` on every administration route. An administrator may grant themselves a
workspace; it is audited like any other grant.

Columns on `users`, all additive:

| Column | Type | Meaning |
|---|---|---|
| `checks_access` (0034) | `BOOLEAN NOT NULL DEFAULT TRUE` | May use Checks. The default keeps every existing account, and any account created by old code or old clients, a Checks account. The Checks role stays in `role`. |
| `time_role` (0034) | `VARCHAR(20) NULL`, check `employee/viewer/manager` | `NULL` = no Time access. Never set by a migration. |
| `global_admin` (0035) | `BOOLEAN NOT NULL DEFAULT FALSE` | CRM administrator. Set by the migration for exactly the accounts whose role is `admin` (they administered accounts before); `FALSE` for anything created by code that predates it. |

**Workspace registry** (`services/workspace_access.py`, `WORKSPACES`). Each entry names the workspace,
its roles (title and explanation in Russian), which roles open it in the Hub, the API prefixes that
belong to it, and the column holding the role. Sign-in, `/api/auth/me`, the path check, the admin API
(`workspaces: {id: role}`, `/api/admin/meta`), effective access and the panel read the registry.

| Workspace | Roles (weakest first) | Notes |
|---|---|---|
| `checks` «Чеки» | `operator`, `admin` («Администратор Чеков») | Default workspace: every `/api/*` path nobody else claims. Data restrictions (tabs, folders, sources, forbidden periods) unchanged. |
| `time` «Учёт времени» | `employee`, `viewer`, `manager` | `/api/time/*`. An employee only marks attendance through the bot and is not listed in the Hub. |

| `time_role` | Bot attendance | Time UI/API |
|---|---|---|
| `employee` («Сотрудник») | yes | none (not listed in the Hub) |
| `viewer` («Просмотр») | yes | read-only: month, day, messages, directories, export |
| `manager` («Руководитель») | yes | everything the shared password allowed |

**Adding a workspace later** takes: one entry in `WORKSPACES`; one nullable column for its role (the
same kind of migration as `time_role`); its routes under its own prefix; a tile in the desktop Hub.
The account directory, sign-in, sessions, the administration API, audit and the panel need no change
(`test_another_workspace_is_a_registry_entry` holds this).

**Compatibility with released clients (desktop 2.0.2, Electron):**

- They send `role` and never `workspaces` or `global_admin`. Creating an account that way yields a
  Checks account with that role and nothing else, as before; `role: admin` is the Checks role only,
  because such a client has no way to say it means a CRM administrator.
- Changing an administrator's `role` from `admin` to `operator` through such a client also removes
  `global_admin`: before the split that is what a demotion meant, and the client cannot show or take
  away a right it does not know. Only the change does it; a role re-sent unchanged does not.
- `403 global_admin_required` replaces `403 admin_only` on the administration routes for an account
  that lacks the right. After migration 0035 no existing administrator is in that position.

The global 2FA policy and the expiry exemption apply to both kinds of administrator. Grants and
`global_admin` do not bump `permissions_version`: sessions and Excel connections survive, and the next
request already sees the new row. Deactivation, password reset and Checks role/tab changes keep
today's behaviour (version bump, every session ends).

**Bootstrap.** `backend/scripts/create_admin.py <username>` (password asked without echo, or
`CRM_ADMIN_PASSWORD`) creates the first CRM administrator from the server's shell;
`--promote <username>` is the way back in if none is left. Both are written to the audit journal. The
last active CRM administrator cannot be disabled or stripped by any path (`last_admin`).

## 5. Architecture and trust boundaries

```
desktop Hub / admin panel ──TLS──> Caddy ──> backend (FastAPI)  ──bearer──> timelist (Rust, internal only)
                                              │  users, sessions, workspaces        │ SQLite volume
                                              └── /internal/time/* <──hub key───────┘ bot ──> Telegram
```

1. **Client → backend.** Unchanged login/JWT/session. `get_current_user` classifies the request path:
   hub-level (`/api/auth`, `/api/2fa`, `/api/admin`, `/api/security/app`, `/api/security/system-status`,
   `/api/security/sessions`, `/api/system-settings`), a registered workspace's prefix (`/api/time`),
   everything else Checks; a missing workspace is `403 {"error":"workspace_forbidden","workspace":…}`.
   Default is Checks, so a new Checks endpoint is covered without anyone remembering to protect it.
   Hub-level administration routes then require `global_admin`.
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
6. **Admin panel (`/admin/`).** Three static files served by the backend with a strict
   Content-Security-Policy; they hold no data and no secret. Everything shown comes from
   `/api/admin/*` with the signed-in administrator's token, authorised on the server per call.
7. **Desktop.** The Time page is embedded like the Telegram one: its files are compiled in, its
   requests go through `Api::proxy` limited to `/api/time/*`, it holds no credential. The Hub offers
   «Администрирование CRM» to a CRM administrator only, and the server decides again on every call.
8. **`auth_bot`.** Its user commands stay behind the owner's `AUTH_ADMIN_IDS` allowlist and create
   Checks accounts only. Its system-wide switches (`/codes_off`) now require the linked account to be
   a CRM administrator.

## 6. What is reused, what is added

Reused as is: Time's core, API, bot parser, attendance rules, time zone, UI, Excel exchange; the
backend's users, hashing, 2FA, rate limits, sessions, audit log, admin endpoints; the desktop's start
window, session handling and WebView host pattern.

Added: migrations 0034 and 0035; the workspace registry and enforcement; `require_global_admin`; admin
API extensions (generic workspace grants, `global_admin`, effective access, per-user sessions and
revocation, last-administrator guard, audit listing, reference data from the registry); Time gateway
and bot identity endpoint; Time hub mode (config, bot checks, change counter for polling instead of
SSE, UI switches); the Russian CRM administration panel served by the backend at `/admin/`; desktop
workspace «Учёт времени» and the administration entry; a safer `create_admin.py`;
Dockerfile/compose/Caddy changes and this plan.

## 7. Integration points with the concurrent receipt-enrichment session

That session owns the application dictionary, receipt details and the Description column. This work
does not touch parsers, receipt processing, transactions routes or annotation services. Shared files
touched, each by a few lines:

- `backend/database/models.py` — three columns in `User`.
- `backend/services/user_service.py`, `backend/api/dependencies.py`, `backend/api/routes/admin.py`,
  `backend/api/routes/auth.py` — snapshot fields, enforcement, admin fields.
- `backend/api/routes/security.py` (three session routes), `backend/api/routes/system_settings.py`,
  `backend/api/routes/two_factor.py`, `backend/services/auth_bot_handler.py` (two filters) — the
  CRM-administrator check where the Checks role used to stand in for it.
- `backend/api/routes/excel_sync.py` (`_check_user`), `backend/api/routes/telegram_client.py`
  (WebSocket auth), `backend/services/ai_agent/authorization.py` — one workspace check each.
- `backend/api/main.py` — router registration, including the `/admin` panel routes.
- Alembic: `20261112_0034` and `20261112_0035` follow `20261112_0033`. If the other session also adds
  a revision on 0033, whichever merges second changes its `down_revision`; they are independent (these
  alter only `users`).

## 8. Acceptance criteria

1. Checks-only, Time-only, both, neither: the Hub lists exactly the granted workspaces; "neither"
   gets a clear explanation; no second login when switching.
2. Direct HTTP calls to a workspace that is not granted fail with 403, including exports, Excel
   protocol, WebSocket and agent runs.
3. Revoking a workspace, deactivating an account or revoking sessions takes effect for a running
   client on its next request; the workspace disappears from the Hub without reinstalling.
4. A deactivated or unlinked account cannot mark attendance through the bot; foreign groups are ignored;
   duplicates and restarts do not double-count.
5. CRM administration is separate from workspaces in both directions: a CRM administrator with no
   workspace administers and opens nothing else; no workspace role (Checks administrator, Time
   manager) reaches any administration route. The last CRM administrator cannot be removed; passwords
   can be reset, never read; every admin action is audited without secrets.
6. From the panel the owner can create, search, edit, enable and disable accounts, set and reset
   passwords, end sessions, grant and revoke each workspace with its role and data permissions, and
   see each account's effective access.
7. Existing Checks roles, scopes, Excel sync and the whole existing backend suite still pass; no
   existing account gains or loses anything at migration.
8. One user's cached data is not shown to another, and is removed when Checks is revoked.
9. A workspace can be added through the registry without another user directory or a rebuilt panel.
10. Evidence is labelled: unit tests, local integration against real services, simulators, and what
    could not be run (real Windows, real Telegram, production).

## 9. Deployment and permission-migration plan — FOR APPROVAL, not executed

Detailed commands live in `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md` (written with the deployment
artefacts). Outline:

1. **Pre-checks (read-only):** `alembic current` = `20261112_0033`; `SELECT role, is_active, count(*)
   FROM users GROUP BY 1,2` (the `admin` rows are who becomes a CRM administrator); container health;
   disk space.
2. **Backups:** `pg_dump` + globals, `.env`, compose, Caddyfile, image IDs → `/opt/parcer-backups/<tag>`.
3. **Secrets (owner, on the server, never in chat/git):** `TIMELIST_API_TOKEN`, `TIME_HUB_KEY`
   (random), `TIME_BOT_TOKEN` (from @BotFather), `TIME_BOT_CHAT_ID` (after `/id` in the group).
4. **Schema:** `alembic upgrade head` (0034, 0035) — three columns and one check on `users`. No
   existing column of any row is rewritten; `global_admin` is filled for the accounts whose role is
   `admin` and for nobody else. Post-check: every `checks_access` true, no `time_role`, `global_admin`
   true exactly where `role = 'admin'`.
5. **Backend image** with the new code for backend, workers, `auth_bot`; **timelist** as a new
   container with its own volume; Caddy gets one extra route (`/admin*` → backend) by reload.
   PostgreSQL, Redis, Caddy data and `funpay-bot` are not recreated.
6. **Health:** `/health`, `/admin/`, login, `/api/admin/meta`, `/api/time/meta`, bot `getMe` status.
7. **Owner actions:** review the CRM administrators in the panel and keep only the intended ones;
   create the bot, disable privacy mode, add it to the group; grant workspaces and enter Telegram IDs
   in the admin panel; bind employees in «Учёт времени».
8. **Rollback:** stop `timelist`, restore the previous backend image and Caddyfile. The three columns
   are compatible with the old code and stay; the database is not restored, so no receipt is lost.
   Schema downgrade is a separate, optional step and forgets who was made or unmade a CRM
   administrator: after it, the right is read from the Checks role again.

## 10. Production access for this session

`HOPLITE_SSH_KEY_PATH` is not set in this sandbox, so the shared read-only key is not provisioned here.
Nothing was read from production; the inventory queries above are part of the approved-run pre-checks.
All verification is local, on isolated databases and a stand-in Bot API.
