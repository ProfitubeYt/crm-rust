> Continuation handoff: this file preserves the previous session's evidence. Read TIME_RESUME_STATUS.md and docs/tasks/RESUME_TIME_HUB_ADMIN_PROMPT.md for the current merged 2.0.3 baseline and unfinished work.

# CRM Hub: accounts, global administration, workspaces, Time bot — progress

Branch `work/time-hub-admin` from `main` `45d7e5d`. Plan: `docs/handoff/TIME_HUB_ADMIN_PLAN.md`.
Nothing here is deployed; production was not read or changed.

## Resume session — 2026-11-12, branch `work/time-hub-admin-resume`

Where the next session picks up: `docs/tasks/CONTINUE_TIME_HUB_ADMIN_PROMPT.md`. Everything
below this section is the previous session's record and is historical.

**Baseline.** `main` at `7c4e4f48a12791c9beddcb07fdf164048db6955b`: release 2.0.3 (`f5eede4`)
and the Time work (`22b8417`) merged. Nothing is deployed. Production was read once with the
SELECT-only role (pre-checks below) and not changed. The session ended early, on credits.

| Commit | What it holds |
|---|---|
| `feb2d2b` | WIP 1: desktop Hub entries and hosted pages, admin panel shell mode, production pre-checks, backend test runner |
| `924a2b7` | WIP 2: fixes from the code review, deployment overlay, e2e stack skeleton |
| the commit with this text | tests for the review fixes, `scripts/hub/desktop-tests.sh`, this record, the continuation prompt |

### Decisions

Desktop:

- `GET /api/auth/me` is read with three optional fields, `workspaces`, `workspace_roles`,
  `global_admin`. A server that does not send them (production 2.0.3) is a Checks server:
  Checks as before, Time and CRM administration never inferred. A field in a shape the build
  cannot read counts as not sent, so it cannot fail a sign-in.
- The Hub lists by `workspaces`. The Time role `employee` has no interface (the server leaves
  `time` out of `workspaces` for it); it only lets a person mark attendance through the bot.
- Time and the admin panel are hosted pages (`desktop/crates/tbs-app/src/screens/hosted/`):
  the page files are compiled in (`build.rs`, from `time/crates/server/assets` and
  `backend/admin_panel`), each page has its own scheme (`tbstime`, `tbsadmin`) and may reach
  only `/api/time` or `/api/admin` through the shell's signed-in API client. The page holds
  no password, token or cookie; of its headers only `Content-Type` goes to the server. It can
  tell the shell two things: `access-lost`, and `export` with a `YYYY-MM` month (Time only).
- The server decides access on every request (gateway, admin API). The client follows it:
  events from refused requests, `/api/auth/me` every 30 s and on entering the Hub or a workspace.
- One web-view folder per page, account and server under `<data>/web/`; removed on sign-out
  and on revocation, whether or not the page is open.
- The API client has a session epoch: an answer that belongs to a session which has since
  ended or been replaced (a late 401/403, renewed tokens, an overtaken sign-in) is dropped.

Deployment (prepared, not rehearsed, not approved):

- `deploy/time-hub/docker-compose.time.yml` is an overlay: one container `timelist`, one
  volume `timelist_data` (the SQLite file), no published port. The main compose file is not edited.
- `.env` on the server gets five values: `TIMELIST_INTERNAL_URL=http://timelist:8080`,
  `TIMELIST_API_TOKEN`, `TIME_HUB_KEY` (read by the backend through its `env_file`), and
  `TIME_BOT_TOKEN`, `TIME_BOT_CHAT_ID`, which the overlay passes to Time as
  `TELEGRAM_BOT_TOKEN`/`TELEGRAM_CHAT_ID` (the backend has a `TELEGRAM_BOT_TOKEN` of its own).
- `deploy/Caddyfile` gets one route, `/admin` and `/admin/*` to the backend. `/internal/*` is
  not routed to the backend; that it stays so has to be shown in the rehearsal.
- Backend image: `backend/Dockerfile.overlay` over the live image is enough, because
  `backend/requirements.txt` is unchanged since `f5eede4` (checked with `git diff`).
- The Time image has to be built off the server (a Rust release build; the server has about
  2.9 GB of memory) and loaded there with `docker save` / `docker load`.
- Migrations 0034 and 0035 only add: `checks_access` true for everyone, `time_role` NULL for
  everyone, `global_admin` true where `lower(role) = 'admin'`. Rollback is the previous image,
  the previous Caddyfile and stopping `timelist`; the columns stay and no database is restored.

### What is done, and how far it was checked

| Item | State |
|---|---|
| Hub shows Checks / Time / CRM administration by grant; no-workspace state; old backend keeps Checks | done, unit tests |
| Time and admin panel as hosted pages; narrow page-to-shell contract; per-account folders | done, unit tests; **never opened in a real window** |
| Revocation, deactivation, expired session, sign-out, user switch close the page and drop caches | done, unit tests |
| Admin panel shell mode (`backend/admin_panel/admin.js`), Time page hook (`time/crates/server/assets/app.js`) | done; **no automated test, not run in a browser this session** |
| Fixes from the code review (below) | done; see "Tests actually run" for the last commit |
| Production pre-checks `deploy/time-hub/prechecks.sql` | done, run once read-only |
| `deploy/time-hub/postchecks.sql`, compose overlay, Caddy route, `.env.example` | written, **not rehearsed** |
| `scripts/hub/e2e/` (`compose.yml`, `e2e.env`, `fake_telegram.py`) | written, **never run**; `prepare.py`, `checks.py`, `run.sh` not written |
| Migration rehearsal on PostgreSQL, end-to-end run, real-window run, Windows build | **not done** |
| `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md` | **not written** |

### Tests actually run (2026-11-12, UTC)

- Whole backend suite, 11:45, Python 3.11 image pinned in `backend/Dockerfile`, pinned
  requirements, SQLite in memory, dummy secrets, on the tree that became `feb2d2b`:
  **1346 passed, 1 skipped**, 266 s. A second run through `scripts/hub/backend-tests.sh` was
  cut by the shell timeout at about 85% with no failure shown; that is not a result. The suite
  was not run after `.env.example` and `deploy/Caddyfile` were edited.
- Desktop, `cargo test --workspace --locked` in `tbs-linux-dev:pinned`, on `feb2d2b` (twice,
  12:06 and 12:12): **340 app tests and 136 core tests passed**. `live_backend` and
  `live_writes` report one passed test each by returning early without a server; they are not
  integration coverage. An early run the same day showed 300 app tests; it was not a clean
  baseline of `7c4e4f4` (the clean one was refused by the Docker bind-mount rule and not repeated).
- Desktop on `924a2b7`: `cargo test --workspace --locked --no-run` compiled, exit 0.
- Desktop on the last commit: a whole-workspace run passed **348 app tests** and stopped at one failing core test. That test was new and its second half was itself wrong (its stand-in server kept answering 401 after a renewal, so the client rightly ended the session); the half was removed and the core tests were rerun alone: **141 passed**. The two `live_*` test files were not run again; they are not coverage.
- Time image: `docker build -t timelist:local time` succeeded (about eight minutes on two CPUs).
  The container was not started; whether its `HEALTHCHECK` (`curl /api/meta`) passes in Hub
  mode, where the API wants the gateway's token, is **unverified**.
- Production, read-only over the pinned SSH tunnel: schema `20261112_0033`; none of
  `checks_access`, `time_role`, `global_admin` exists; two active accounts with role `admin`
  and one active `operator`; no other role values. Migration 0035 would therefore make exactly
  those two accounts CRM administrators. Their names are in the session's report to the owner,
  not in git; `prechecks.sql` prints them again. The shared role cannot read 2FA state or
  Telegram IDs; the two queries for an administrator are at the top of `prechecks.sql`.
- No real Telegram bot, no browser, no native window, no Windows build, no production change.

### Code review of `7c4e4f4..feb2d2b` (independent, read-only)

No critical finding; the page/shell boundary held (prefix checks, header filtering, method
list, message contract, Time cannot reach `/api/admin` and the panel cannot reach `/api/time`).
Fixed in `924a2b7`:

- High: with a launch password, a page that had not been started stayed on «Загрузка…».
- Medium: after losing CRM administration the settings kept showing accounts and sessions.
- Medium: a web view and its signals had no owner; a late view or a late "loaded" could attach
  to the next opening. Each opening now has a number carried by the view and its signals.
- Medium: "forget" emptied the web view only if the page was open; the folders are now removed.
- Low: answers about the account applied out of order; a refusal or a token renewal of an
  ended session applied to the next one; an overtaken sign-in kept as a sign-in; the workbook
  offered after its page was closed; a grant in an unreadable shape failed the sign-in;
  navigation allowed to any same-origin address; proxied answers without `nosniff`.

Left open, on purpose or for lack of time:

- The page's message handler does not check the sender's address (cannot be verified on
  WebView2 here; navigation is held to one document and frames are blocked by the page's CSP).
- The sign-in form does not number its attempts: an answer that arrives after "Back" still
  signs in (same person, same credentials). The cross-account case is closed in the API client.
- The Time page does not post `access-lost` for the plain-text refusal `inactive_user`; the
  shell handles that refusal itself.
- Proxied bodies are buffered whole without a size cap.
- Older behaviour, not changed: with `LAUNCH_GATE_ENABLED`, `/api/auth/refresh` is not exempt
  from the gate, so a saved session is dropped at every launch.

### E2E design, so it is not worked out again

`scripts/hub/e2e/compose.yml` mirrors the server's service names (`postgres`, `redis`,
`backend`, `caddy`) so that `deploy/time-hub/docker-compose.time.yml` applies on top unchanged:

    docker compose --project-directory . --env-file scripts/hub/e2e/e2e.env \
        -f scripts/hub/e2e/compose.yml -f deploy/time-hub/docker-compose.time.yml up -d

Its default network is internal (no internet for the backend, Time or the fake Telegram);
Caddy alone publishes `127.0.0.1:18080`. The backend runs from the pinned Python image with the
`hub-backend-venv` volume (created by `scripts/hub/backend-tests.sh`), without TDLib. Known
loose ends in the file: `SYSTEM_ACCESS_TOKEN` and `TIME_BOT_CHAT_ID` sit in the shared backend
environment and belong to the `tool` service only; the `run` volume is root-owned, so the
config step has to run as root.

Still to write:

- `prepare.py config`: `root-access.server.json` with the hash of `SYSTEM_ACCESS_TOKEN`
  (`services.root_access_config_service.hash_password_pbkdf2`, shape as in
  `backend/tests/hub_support.py`), and the Caddyfile made from `deploy/Caddyfile` with only the
  site address replaced by `:80` (the real one would ask Let's Encrypt for the production name).
- `prepare.py legacy-schema`, run with the code of `f5eede4` (`git archive f5eede4 backend`):
  `create_all`, accounts shaped like production (two active admins, one operator, one inactive
  admin), then `alembic stamp 20261112_0033`.
- `checks.py`, through Caddy and directly: `/health`; the system-access gate without Caddy;
  `/admin` 308, `/admin/` with its CSP; `/internal/time/telegram/{id}` not reachable through
  Caddy, refused without the key or with proxy headers, answered directly with the key; a
  pre-migration admin signs in and is a CRM administrator; accounts created through
  `/api/admin/users` for Checks only, Time viewer, Time manager, both, neither, CRM
  administrator without workspaces, Time employee, disabled; for each the `/api/auth/me` fields
  and the matrix Checks API / `/api/time/meta` / a Time write / `/api/admin/meta` / export;
  revocation and re-grant on a live token; deactivation; `revoke-sessions`; refresh rotation;
  lockout after wrong passwords; a locked month refuses edits (409).
- Bot, through `fake_telegram.py` (`TELEGRAM_API_BASE=http://faketg:8081`): a linked employee's
  message in the work group is stored (🤷 until bound, ✍ after `POST /api/time/tg/users/{id}/link`);
  an unknown sender, a revoked or disabled account get 🤷 and nothing is stored; another group
  is ignored; `/id` answers in private.
- `run.sh`: up, migration rehearsal, start, checks, down.

### Commands and sandbox rules

- Desktop tests: `scripts/hub/desktop-tests.sh` (the wrapper itself was never run; the
  `docker run` line inside it is the one every run above used).
- Backend tests: `scripts/hub/backend-tests.sh [pytest arguments]`; about five minutes.
- Time image: `docker build -t timelist:local time`.
- Inline interpreter heredocs are refused: put the script in a file. A shell command is killed
  at its timeout (two minutes by default, ten at most): run long work as `docker run -d`, then
  `docker wait` and `docker logs`. Docker bind mounts only from the checkout. Long tool output
  is replaced by a summary: read at most a hundred lines at a time.
- Read-only production access: `docs/handoff/SHARED_SSH_ACCESS.md`. The key was supplied as a
  chat attachment and kept outside the worktree; a new sandbox needs it again. It expires on
  2026-11-25. It gives no shell and no right to deploy.

### Owner: secrets, decisions, approvals

- The Time bot does not exist yet: token from @BotFather, privacy mode off, the work group's ID.
- `TIMELIST_API_TOKEN` and `TIME_HUB_KEY` are to be generated on the server, never in chat or git.
- Whether both accounts that 0035 makes CRM administrators should keep that right, and
  whether they have 2FA set up (the panel sends an administrator without it to the desktop sign-in).
- Approval of the concrete plan before any migration, new image, new container or Caddy
  reload. Somebody with deploy rights has to carry it out: the shared key cannot.
- A Windows machine for the release build and installer; the version is still 2.0.3.

## Status (previous session, historical)

| # | Milestone | State | Commit |
|---|---|---|---|
| 0 | Investigation, owner questions, plan | done | `f81d7c5` |
| 1 | Backend: migration 0034, workspace access on the account, enforcement on every request | done | `bfbd068` |
| 2 | Backend: admin API (grants, per-user sessions, guards, audit), Time gateway, bot identity endpoint | done | `bfbd068` |
| 3 | Time: hub mode, bot checks per message, UI in gateway mode, container image | done | `35f8138` |
| 4 | Scope correction: CRM administration separate from workspaces (migration 0035, registry, generic API) | done | `b47d06f` |
| 5 | CRM administration panel (Russian), `/admin/` | done | this commit |
| 6 | Desktop: «Учёт времени» and «Администрирование CRM» in the Hub | pending | — |
| 7 | Deployment artefacts, local end-to-end run, deployment and rollback plan for approval | pending | — |

## Milestone 5 — the CRM administration panel (2026-11-12)

`backend/admin_panel/` (`index.html`, `admin.css`, `admin.js`, no build step, no dependency) served by
`backend/api/admin_panel.py` at `/admin/`. It is a client of `/api/admin/*` and nothing more: it holds
no data, and every call it makes is authorised on the server.

What the owner can do in it: find accounts (search; filters by state, by workspace, without any
workspace, CRM administrators); create an account; edit name, expiry, session lifetime; appoint or
remove a CRM administrator; grant or revoke each workspace with its role; edit the Checks data
restrictions of an operator (tabs, folders, sources, closed periods); set a new password (with a
generator and the server's rules shown); require or reset 2FA; see and end sessions; link a Telegram
ID; lift a sign-in lock; disable and enable; read each account's effective access and its journal;
see the workspaces with their roles and holders; read the account journal.

How it keeps the administrator's session: the access token lives in the page's memory only; the
session is continued by the backend's existing HttpOnly refresh cookie, which script cannot read.
Nothing is written to `localStorage`. Behind the launch gate it asks for the launch password first
and keeps only that gate token for the tab. The page is served with
`default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; form-action 'none';
frame-ancestors 'none'`; it has no inline script or style and builds the page from text nodes.

### Evidence

| What | How | Result |
|---|---|---|
| Files and policy | `tests/test_admin_panel_static.py` (4 tests): only the three named files are served, traversal and other names 404, headers and CSP, the page obeys its own policy, files are public while `/api/admin/meta` is 401 / 403 `global_admin_required` | passed |
| Whole backend suite | `pytest tests` | **1330 passed** |
| The panel in a browser | headless Chrome 157 against the real application's HTTP layer with PostgreSQL 15 and Redis 7 in local containers; ten seeded test accounts. The lifespan was replaced so TDLib, history loading and the agent scheduler did not start, and the system-access gate was off because no Caddy stood in front | below |

Observed in the browser run (`/tmp/hoplite/tools/panel-e2e.sh`, `panel-e2e-launch.sh`; a local
simulation with test data, not production):

- A Checks administrator and a Time manager sign in with correct passwords and are refused:
  «У этой учётной записи нет прав администратора CRM»; the session just opened is closed again.
- A CRM administrator with no workspace opens the panel; after a reload the session resumes with
  nothing in web storage and no cookie visible to script.
- Create with a generated password (6/6 rules): the row is `operator | checks f | time manager |
  global_admin f | argon2id`; the password is not in the journal.
- New password: asks first, then the new password signs in and the old sessions are gone.
- A change reaches a signed-in client: with one token, Checks `200` → revoke → `403
  workspace_forbidden` → grant CRM administration → admin API `200`, Checks still `403` → restore →
  admin API `403`, Checks `200`; `permissions_version` unchanged.
- End sessions: the token gets `401`. Disable: token `401`, sign-in `inactive_user`; enable: signs in.
- Own card: the administrator box and the disable button are disabled; direct requests for the same
  are refused by the server (`cannot_demote_self`, `cannot_disable_self`).
- An account stripped of CRM administration but still Checks administrator and Time manager: admin
  API, sessions and system settings `403`, Checks scopes `200`.
- Behind the launch gate: the API answers `launch_required`, the page still loads, the panel asks
  for the launch password, counts wrong attempts, then signs in and survives a reload.
- No page errors and no console output during the runs; dark scheme and a 430 px viewport render.

## Milestone 4 — administration is global (2026-11-12, after the owner's correction)

The owner corrected the scope: the panel is the central place of the whole CRM, administration must
not depend on Time or on any workspace, and workspace managers must not become administrators. What
changed, and how each part was checked:

- **`users.global_admin`** (migration `20261112_0035`) is the CRM administrator. It is filled for
  exactly the accounts whose role is `admin` and for nobody else. `require_global_admin` guards all
  of `/api/admin/*`, `/api/security/sessions*` and `/api/system-settings`.
- **`role` is now only the Checks role.** Everything it did inside Checks is untouched. Every use of
  `require_admin_user` was reviewed: the 19 routes that are CRM administration (14 under
  `/api/admin`, 3 session routes, 2 system-settings routes) moved to the new guard; the rest are
  Checks operations and stayed. The two Checks journal listings (`/api/audit`,
  `/api/security/audit`) leave the account-administration records out for anyone who is not a CRM
  administrator.
- **Workspace registry** in `backend/services/workspace_access.py`: Checks and Time with their roles,
  titles, Hub visibility, API prefixes. The admin API takes `workspaces: {id: role}` and reports
  `workspace_roles`, `workspaces`, `global_admin`, `effective_access`; `/api/admin/meta` lists the
  registry. The earlier `checks_access` / `time_role` request and response fields are gone (they
  existed only in this branch).
- **Older clients:** a role-only request still creates and edits Checks accounts; demoting an
  administrator through such a client also removes `global_admin`; nothing they send can grant it.
- **2FA policy and the expiry exemption** cover both kinds of administrator; `auth_bot`'s system-wide
  switches require a CRM administrator.
- **Bootstrap:** `scripts/create_admin.py` creates or promotes a CRM administrator from the server's
  shell, asks for the password without echo, and writes to the audit journal.

Migration from the Time-only implementation: nothing in production to convert (it was never
deployed). A local database at `20261112_0034` continues with `alembic upgrade head`.

### Evidence (all local; no production, no real Telegram)

| What | How | Result |
|---|---|---|
| Whole backend suite | `pytest tests`, Python 3.11, SQLite in memory, fake session store, dummy secrets | **1326 passed**, 0 failed, 0 skipped |
| Administration boundary | `tests/test_admin_workspace_api.py` (25 tests): every administration route of the running app is refused to an operator with every tab, to a Checks administrator who is also Time manager, and to a Time-only manager; a CRM administrator with no workspace administers and is refused in Checks and Time | passed |
| Grants reach a running client | same file and `tests/test_workspace_access.py`: grant/revoke of `global_admin` and of each workspace takes effect on the next request of an already issued token | passed |
| Lock-out protection | last active CRM administrator cannot be disabled or stripped through API or service; self-disable and self-strip refused | passed |
| Extensibility | `test_another_workspace_is_a_registry_entry`: a third workspace added to the registry appears in `/api/admin/meta`, owns its API prefix and is enforced, with no other change | passed |
| Migrations on SQLite | `tests/test_workspace_access_migration.py`: 0034 and 0035 up, twice, down, twice | passed |
| Migrations on PostgreSQL 15.19 | disposable container; schema built by the 2.0.2 models and stamped 0033, then `alembic upgrade head`, the 2.0.2 code run against the new schema, `downgrade 20261112_0033`, `upgrade head` | passed (below) |

PostgreSQL round trip, abridged output:

```
Running upgrade 20261112_0033 -> 20261112_0034, Workspace access on user accounts.
Running upgrade 20261112_0034 -> 20261112_0035, CRM administration as a right of its own on the account.
checks_access|boolean|NO|true      global_admin|boolean|NO|false      time_role|character varying|YES|
owner|admin|t|t   operator|operator|t|f   gone|operator|f|f   former-admin|admin|f|t
global_admin is true exactly for the accounts whose role was admin
every pre-existing column of every account is unchanged
an admin made by the old code is a Checks administrator, not a CRM administrator
removing the only active CRM administrator is refused: last_admin
columns left after downgrade: 0
```

Known consequence, stated in the plan: a schema downgrade forgets who was made or unmade a CRM
administrator; after a new upgrade the right is derived from the Checks role again.

## Milestones 1–3 — evidence recorded when they were committed

| What | Result |
|---|---|
| Backend suite at `35f8138` | 1312 passed, 1 skipped (optional `oletools`, since installed) |
| Checks routes without the Checks workspace | every route of the running application that resolves a user and belongs to Checks (more than 150) refuses with `workspace_forbidden`, exports and Excel management included |
| Time workspace (`cargo test --workspace --locked`, pinned image) | 17 server tests (bot against a stand-in Bot API: revocation, deactivation, foreign groups, duplicates, restart), 28 core, 1 golden |
| Time container | `time/Dockerfile` builds (`timelist:local`) |
| Desktop baseline before any desktop change | `cargo test --workspace --locked`: 122 core + 302 app passed; the live tests return early without a backend and are not integration evidence |

## Milestone 0 — investigation (2026-11-12)

- Read the task in full, `time/` (both copies are identical apart from `.gitattributes`), the backend
  auth stack, the desktop shell and Telegram WebView host, compose and Caddy.
- `HOPLITE_SSH_KEY_PATH` is not set: the shared read-only SSH key is not provisioned in this sandbox.
  Work continues locally only.
- Owner's answers (plan, section 2): Telegram ID entered by the administrator; no bot yet; no Time
  data; nobody receives Time at migration; one global panel for the whole CRM.
- Tooling: Docker daemon available, 2 CPUs / 8 GiB (a resize was refused by the workspace ceiling).
  Rust runs only in the pinned image built from `desktop/tools/linux-dev.Dockerfile`
  (`rust:1.99.1-bookworm`), Cargo with `--locked`. Backend tests run in a local venv on SQLite with
  dummy secrets and no network services.

Baselines before any change: backend auth subset 13 passed; whole backend suite 1226 passed, 1
skipped — same as the 2.0.2 record; Time workspace 38 passed.

## Open items / unrun checks

- The panel was exercised in headless Chrome only; no other browser, and not behind the production
  Caddy (which adds `X-System-Access` and needs the `/admin*` route from the deployment plan).
- The panel does not set up 2FA for an administrator who is required to have it and has none yet;
  it says so and points to the desktop sign-in, where that flow exists.
- Desktop: no change yet; the Hub does not list «Учёт времени».
- `cargo clippy` and `cargo fmt --check` for Time: the tools are absent from the pinned image.
- Windows build of the desktop, a real Telegram bot and production were not exercised.
- Sign-in records (`login_success`, `login_failed`) of every account are still listed in the Checks
  journal for a Checks administrator, as they always were.
