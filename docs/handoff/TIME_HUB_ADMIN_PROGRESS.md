# Time workspace / unified accounts / admin panel / Time bot — progress

Branch `work/time-hub-admin` from `main` `45d7e5d`. Plan: `docs/handoff/TIME_HUB_ADMIN_PLAN.md`.
Nothing here is deployed; production was not read or changed.

## Status

| # | Milestone | State |
|---|---|---|
| 0 | Investigation, owner questions, plan | done |
| 1 | Backend: migration 0034, workspace model and enforcement | pending |
| 2 | Backend: admin API (grants, sessions, guards, audit) | pending |
| 3 | Backend: Time gateway and bot identity endpoint | pending |
| 4 | Time: hub mode, bot checks, UI switches | pending |
| 5 | Web admin panel (Russian) | pending |
| 6 | Desktop: «Учёт времени» in the Hub | pending |
| 7 | Deployment artefacts, local end-to-end run | pending |

## Milestone 0 — investigation (2026-11-12)

- Read the task in full, `time/` (both copies are identical apart from `.gitattributes`), the backend
  auth stack, the desktop shell and Telegram WebView host, compose and Caddy.
- `HOPLITE_SSH_KEY_PATH` is not set: the shared read-only SSH key is not provisioned in this sandbox.
  Work continues locally only.
- Owner answered the four open questions (see plan, section 2): Telegram ID entered by the
  administrator; no bot yet; no Time data; nobody receives Time at migration.
- Tooling: Docker daemon available, 2 CPUs / 8 GiB (a resize was refused by the workspace ceiling).
  Rust runs only in the pinned image built from `desktop/tools/linux-dev.Dockerfile`
  (`rust:1.99.1-bookworm`), Cargo with `--locked`. Backend tests run in a local venv on SQLite with
  dummy secrets and no network services.

## Baselines before any change

| What | Command | Result |
|---|---|---|
| Backend auth subset | `pytest tests/test_admin_users_api.py tests/test_app_user_gate.py tests/test_security_authz_token_sessions.py` | 13 passed |
| Backend whole suite | `pytest tests` | 1226 passed, 1 skipped (optional oletools) — same as the 2.0.2 record |
| Time workspace | `cargo test --workspace --locked` (pinned image) | 38 passed: 9 server (one drives the bot against a stand-in Bot API), 28 core, 1 golden |

Time is "mostly ready" as a standalone program: its own tests pass and the bot loop works against
the stand-in. What it lacks for this task is everything about accounts (it has one shared password),
a server deployment, and protection of bot attendance beyond the group list.

## Open items / unrun checks

Listed here as they appear; see the final section of this file before relying on anything.
