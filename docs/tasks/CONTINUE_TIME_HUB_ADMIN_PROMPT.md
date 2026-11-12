# Continue: Time / Hub / global CRM administration

Repository: https://github.com/josukekleinjewite51-ui/fix . Work on branch
`work/time-hub-admin-finish` (created from `main` at `4e644eb`; `main` is unchanged). Start
from the branch's head; do not replay earlier commits or start from an older baseline.

**Nothing is deployed. Production is release 2.0.3 at schema `20261112_0033`, untouched.**

Read first, in this order:

1. `docs/handoff/TIME_HUB_ADMIN_PROGRESS.md`, the top section "UI and release session": the
   owner's latest decisions, what the last run did and what it did not verify. Then "Finish
   session" for the tests, the rehearsal and the window run before it.
2. `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md`: the plan the owner approved, with conditions.
3. `docs/tasks/RESUME_TIME_HUB_ADMIN_PROMPT.md`: the original task and its limits.

## The owner's decisions that stand

- Production deployment of the plan **is approved, after** the Time UI adaptation, preflight
  checks, tests and fresh verified backups. No part of it has been executed.
- Both `admin` and `stlee` keep global CRM administration. No credential reset, nobody signed
  out, Time not granted to everyone, Checks access and restrictions preserved.
- Check whether both administrators have 2FA set up without revealing secrets or disabling
  requirements; keep a working desktop enrollment and recovery path; nobody may be locked out.
- Time screen: no hamburger; the desktop program's outer navigation as **one slim top bar,
  no left column** (← Главный экран, workspace name, online state, account, Выйти) for Time
  and CRM administration; Checks keeps its rail. Compact labelled tabs, no duplicate name or
  menu, no wasted margins, timesheet first, fits narrow windows, resizing, DPI, both themes.
  Attendance logic untouched. A viewer gets no usable edit control. Standalone browser use
  preserved. Verify in a running hosted window; label Linux, Wine and Windows evidence honestly.
- Bot `@tbstimebot`: token valid at the owner's check. It is supplied separately as a
  protected secret (`TIME_BOT_TOKEN` in the owner's `C:/Users/heper/.config/parcer-secrets/time-bot.env`).
  **Never commit or print it**, never overwrite the Checks bot's token. Note: the owner's chat
  message of 2026-11-12 14:59 UTC also contained the token in clear text; the agent did not
  use or store it; treat it as exposed and ask the owner to reissue it (`/revoke` in BotFather).
- Privacy mode was still **enabled** at the last check. Owner: BotFather `/setprivacy` →
  `@tbstimebot` → Disable, then add the bot to the group again.
- Owner's Telegram ID: `1957390574` (for the intended identity link only).
- Group ID `1276772106` is **not verified** (Telegram: chat not found; no group in
  `getUpdates`). Do not guess its prefix, do not accept arbitrary groups, do not leave the
  allowlist open. Ask for `TIME_BOT_CHAT_ID`; until a verified negative ID exists the bot's
  attendance stays off and is reported as pending. Time and the panel may go live without it.
- Native Windows release and installer on an authorised Windows machine, version bumped.
- The shared `hoplite-ro` SSH key is read-only, not deployment access. Deploy through a
  separately provisioned mechanism or hand exact artefacts and commands to the owner's
  administrator. Never ask for root passwords in chat.

## State of the code

Done and pushed (see the progress record for detail):

- Backend, migrations 0034/0035, the admin panel, the Time gateway, the desktop Hub entries
  and hosted pages, the e2e stack, the migration rehearsal, the deployment plan.
- Last run, commit `fa69c1b` (WIP): the Time page itself for the program's window
  (`time/crates/server/assets/index.html`, `app.css`, `app.js`): no name or account of its
  own, one 44 px header line of labelled tabs + period + state, one toolbar line, a viewer
  sees no edit controls and the page sends no edits. Checked **only in headless Chromium
  through a stand-in for the program**: 73 checks passed and 1 failed in the last complete
  run; the failing case (viewer, 1024 px, closed month) was tightened afterwards and not
  rerun to the end.

Not done, in the order to do it:

1. **Run the Chromium checks to the end** (commands below) and fix what fails.
2. **The program's slim top bar** in `desktop/crates/tbs-app/src/` (`app.rs`, `ctx.rs`,
   `screens/hosted/mod.rs`). Not started. The design, call sites and the tests to change are
   in the progress record, "The design for the program's top bar". Until it is in, the left
   rail and its hamburger are still on the Time screen.
3. **Rebuild and rerun everything the page and the shell touch**: `scripts/hub/desktop-tests.sh`
   (its `screens/hosted/assets.rs` tests read the page's files), the Windows type check, Time's
   own tests (no wrapper yet: `cargo test --locked` in `time/` with the pinned Rust image),
   `docker build -t timelist:local time`, `scripts/hub/backend-tests.sh`, `scripts/hub/e2e/run.sh`.
4. **A real window**: the rebuilt client under Xvfb (Linux WebKitGTK) against the stack, as a
   manager and as a viewer, both themes, at 1024 × 640 and 1360 × 860, Time and the panel.
   Commit cropped pictures under `docs/handoff/evidence/`; say that it is not WebView2.
5. **Version bump** (2.0.3 → next; the workspace version is in `desktop/Cargo.toml`, the
   installer script is `desktop/tools/build-installer.ps1`) and exact instructions for the
   native Windows build: Telegram, Time and admin assets embedded, `WebView2Loader.dll`, icon
   and version resource, protected access configuration, user settings preserved.
6. **Deployment hand-over**: work the owner's answers into the deployment document (both
   administrators keep the right; bot off until the group ID is verified, `TIME_BOT_TOKEN`
   empty in `.env` until then; the 2FA query; the BotFather steps; how to obtain the group ID
   from a group update). Run the read-only preflight again (`deploy/time-hub/prechecks.sql`).
   The administrator executes; this sandbox has no deployment access.
7. After the rollout: health checks of the document's section 6, the 2.0.3 client, global
   administration, Checks-only / Time-only / both / no-workspace accounts, revocation, the
   gateway. Creating test accounts on production needs the owner's word first.

Known and not changed: the Time page asks the gateway every 5 seconds (12 requests a minute
per open page) against the backend's limit of 200 requests a minute per address; three other
naive-against-aware lock comparisons in the backend (launch and chat password locks); the
Telegram section's web view has the Linux GTK gap that the hosted pages had.

## Production: status and rollback

- Status: release 2.0.3, schema `20261112_0033`, dictionary/details batch applied (17 / 656
  / 59; never rerun), `ANNOTATION_DETAILS_RULES_REFRESH_HOURS=24` (keep). Last read, read-only,
  2026-11-12 13:30 UTC: two active `admin` accounts (`admin`, `stlee`), one active `operator`,
  none of the three new columns. Not read in the last run.
- No migration, image, container, Caddy or `.env` change has been made by this work.
- Rollback, once deployed, is by layer and never restores the database: stop `timelist`, put
  the previous Caddyfile back, switch `BACKEND_IMAGE` back to `parcer-backend:20261112-36ffa0d`;
  the three columns may stay (2.0.3 runs on them; rehearsed) or `alembic downgrade 20261112_0033`
  with the new image. Sessions survive both directions (rehearsed). Details and the
  `global_admin` reconciliation query: deployment document, section 8.
- 2.0.3's own backups and rollback: `docs/handoff/DEPLOYMENT_2_0_3_20261112.md`.

## Missing: access, secrets, owner actions

- A deployment executor (root shell on `64.188.106.221`) or the owner's administrator.
- `TIME_BOT_TOKEN` provisioned securely to that executor (and reissued, see above).
- `TIME_BOT_CHAT_ID`: a verified negative group ID; privacy mode disabled first.
- Whether both administrators have 2FA: the shared read-only role cannot read it; the query
  for the administrator is at the top of `deploy/time-hub/prechecks.sql`.
- A Windows machine for the native build and installer.
- The read-only SSH key is not in git: a new sandbox needs it supplied again, outside the
  worktree, mode 600 (`docs/handoff/SHARED_SSH_ACCESS.md`). Key and database role expire on
  2026-11-25. If it arrives as a chat attachment under `.hoplite/attachments/`, move it out
  of the checkout at once.

## Setup in a fresh sandbox (Docker with compose; about 25 minutes of builds)

    git fetch origin work/time-hub-admin-finish && git checkout work/time-hub-admin-finish
    docker build -f desktop/tools/linux-dev.Dockerfile -t tbs-linux-dev:pinned desktop/tools
    docker build -t timelist:local time                        # about six minutes
    scripts/hub/backend-tests.sh --version                     # makes the volume hub-backend-venv
    docker build -f scripts/hub/e2e/page-shots.Dockerfile -t hub-page-shots:pinned scripts/hub/e2e

Suites and the stack:

    scripts/hub/desktop-tests.sh                               # was 348 app + 141 core before the last run
    scripts/hub/desktop-tests.sh check -p tbs-app --locked --target x86_64-pc-windows-gnu
    scripts/hub/backend-tests.sh                               # was 1349 passed, 1 skipped
    scripts/hub/e2e/run.sh                                     # stack, rehearsal, 173 checks, stack removed

The Time page in Chromium (layout and viewer checks):

    scripts/hub/e2e/run.sh up
    scripts/hub/e2e/run.sh compose run --rm -T tool -m alembic upgrade head
    scripts/hub/e2e/run.sh start
    scripts/hub/e2e/run.sh page
    docker run -d --name tl-standalone --network hub-e2e_default -e TIMELIST_ASSETS_DIR=/assets \
        -v "$PWD/time/crates/server/assets":/assets:ro timelist:local
    docker exec tl-standalone timelist demo
    docker run --rm --network hub-e2e_default -v "$PWD":/work -w /work hub-page-shots:pinned \
        python scripts/hub/e2e/page_shots.py --out output/hub-e2e/page --standalone http://tl-standalone:8080
    docker rm -f tl-standalone; scripts/hub/e2e/run.sh down

The real window: `desktop/README.md`, "A window without Windows", and the Finish session's
record (`socat` to Caddy, `SERVER=http://127.0.0.1:18080`, accounts `ui_manager` and
`ui_viewer` exist after `run.sh page`, password in `scripts/hub/e2e/e2e.env`).

Sandbox rules that cost time: `python3 - <<EOF` and a script created by heredoc and run in
the same command are refused: write the file with the file tool, run it in the next command.
Tool output over about 2,000 tokens is replaced by a summary: read long files in pieces.
`*.log` is ignored by git. Long commands go on in the background; read their output later.

## Saving work

Commit and push after every milestone with the explicit refspec
`git push origin work/time-hub-admin-finish:refs/heads/work/time-hub-admin-finish`; check
`git status --untracked-files=all` before each commit and compare `git ls-remote` with
`git rev-parse HEAD` after each push. Push labelled WIP before credits run low.
