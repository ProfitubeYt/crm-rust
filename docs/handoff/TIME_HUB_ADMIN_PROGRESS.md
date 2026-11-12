> Continuation handoff: this file preserves the previous session's evidence. Read TIME_RESUME_STATUS.md and docs/tasks/RESUME_TIME_HUB_ADMIN_PROMPT.md for the current merged 2.0.3 baseline and unfinished work.

# CRM Hub: accounts, global administration, workspaces, Time bot — progress

Branch `work/time-hub-admin` from `main` `45d7e5d`. Plan: `docs/handoff/TIME_HUB_ADMIN_PLAN.md`.
Nothing here is deployed; production was not read or changed.

## Status

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
