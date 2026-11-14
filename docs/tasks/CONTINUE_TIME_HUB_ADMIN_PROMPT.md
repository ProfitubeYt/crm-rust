# Continue: Time / Hub / global CRM administration

Repository: https://github.com/asintiko/crm-rust . Current main includes
the full preserved history through 04101ae3b30a1fc26ba290d862cdc8ea2219a6ef plus
repository handoff documents. Create work/time-release from current main. Do not
restart from an older account/repository or replay existing commits. Read
docs/handoff/CRM_RUST_STATUS.md for the consolidated status and verification limits.

**Nothing is deployed. Production is release 2.0.3 at schema `20261112_0033`, untouched.**

Read first, in this order:

1. `docs/handoff/TIME_HUB_ADMIN_PROGRESS.md`, the top section "UI and release session": the
   owner's latest decisions, what was done and what was not verified. Then "Finish session"
   for the tests, the rehearsal and the window run before it.
2. `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md`: the plan the owner approved, with conditions.
3. `docs/tasks/RESUME_TIME_HUB_ADMIN_PROMPT.md`: the original task and its limits.

## The owner's decisions that stand

- Production deployment is approved **after** the Time UI adaptation, preflight checks and
  verified backups. No part of it has been executed.
- Both `admin` and `stlee` may retain global CRM administration.
- Remove the inner Time hamburger and adapt the embedded layout: the owner chose **one slim
  top bar of the program, no left column**, for Time and CRM administration; Checks keeps its
  rail.
- The bot token is supplied separately as a protected secret (`TIME_BOT_TOKEN`; the owner's
  file `C:/Users/heper/.config/parcer-secrets/time-bot.env`). **Never commit or print it.**
  The owner's later handoff message exposed the token in chat; the agent did not use or
  repeat it. Treat it as exposed and ask the owner to reissue it.
- Bot `@tbstimebot` was valid, but privacy mode was still enabled at the last check. Owner:
  BotFather `/setprivacy` → `@tbstimebot` → Disable, then add the bot to the group again and
  take the real negative ID from a group update.
- Owner's Telegram ID: `1957390574`.
- Group ID `1276772106` is **not verified** (Telegram: chat not found). Do not guess its
  prefix, do not accept arbitrary groups. Attendance polling stays off until a verified ID
  exists. Assumption to confirm with the owner: Time and the panel may go live before that.
- Shared SSH access (`hoplite-ro`) is read-only, not deployment access. Deploy through a
  separately provisioned mechanism or the owner's administrator. No passwords in chat.
- Find out through an authorised mechanism whether both administrators have 2FA, without
  revealing secrets; keep enrollment and recovery working; nobody may be locked out.

No fact above has been re-verified or changed by the agent since.

## State of the code

Done and pushed (detail in the progress record):

- Backend, migrations 0034/0035, the admin panel, the Time gateway, the desktop Hub entries
  and hosted pages, the e2e stack, the migration rehearsal, the lockout fix, the deployment
  and rollback plan.
- Commit `fa69c1b` (WIP): the Time page for the program's window
  (`time/crates/server/assets/`): no name or account of its own there, one 44 px header line
  of labelled tabs, a viewer gets no edit controls and the page sends no edits. Final check
  run: **74 checks, 0 failed**, headless Chromium 131 on Linux through a stand-in for the
  program, on the committed files. Not a desktop window, not Windows. The pictures of that
  run were not looked at.

Not done, in the order to do it:

1. **Look at the pictures** of the Chromium run (regenerate them with the commands below):
   manager and viewer at 1024 px, the closed month, both themes, standalone.
2. **The program's slim top bar** in `desktop/crates/tbs-app/src/` (`app.rs`, `ctx.rs`,
   `screens/hosted/mod.rs`). Not started. This is what removes the hamburger: the Time page
   has none of its own; the one on the Time screen is the program's rail toggle. The design
   and the tests to change are in the progress record.
3. **Rerun what the page and the shell touch**: `scripts/hub/desktop-tests.sh` (its
   `screens/hosted/assets.rs` tests read the page's files), the Windows type check, Time's
   own tests (`cargo test` in `time/`; no wrapper script), a rebuilt Time image,
   `scripts/hub/backend-tests.sh`, `scripts/hub/e2e/run.sh`.
4. **A real window**: the rebuilt client under Xvfb (Linux WebKitGTK) against the stack, as
   manager and viewer, both themes, Time and the panel; the method is in `desktop/README.md`,
   "A window without Windows", and in the Finish session's record. Pictures go under
   `docs/handoff/evidence/`. Say that it is not WebView2.
5. **Windows**: a native build and installer on an authorised Windows machine, with a version
   bump for a new client (`desktop/Cargo.toml` is at 2.0.3; `desktop/tools/build-installer.ps1`).
   Only a GNU-target type check has passed so far.
6. **Deployment hand-over**: work the owner's decisions into the deployment document (its
   sections 1–9 predate the approval), add the bot and 2FA steps, rerun the read-only
   preflight (`deploy/time-hub/prechecks.sql`; the 2FA query for an administrator is in its
   header). An administrator executes; this sandbox cannot.
7. After the rollout: the health checks of the document's section 6.

## Production: status and rollback

- Release 2.0.3, image `parcer-backend:20261112-36ffa0d`, schema `20261112_0033`. The
  dictionary/details batch is applied (17 / 656 / 59; never rerun) and
  `ANNOTATION_DETAILS_RULES_REFRESH_HOURS=24` stays. Last read, read-only, 2026-11-12 13:30
  UTC: two active `admin` accounts (`admin`, `stlee`), one active `operator`, none of the
  three new columns.
- No migration, image, container, Caddy or `.env` change has been made by this work, and no
  backup has been taken for it.
- Rollback once deployed: `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md`, section 8 (by layer;
  the database is not restored; downgrade to `20261112_0033` was rehearsed locally).
  Release 2.0.3's own backups and rollback: `docs/handoff/DEPLOYMENT_2_0_3_20261112.md`.

## Missing: access, secrets, owner actions

- A deployment executor for `64.188.106.221`, or the owner's administrator.
- `TIME_BOT_TOKEN`, reissued, provisioned to that executor only.
- Privacy mode disabled, then `TIME_BOT_CHAT_ID`: a verified negative group ID.
- Whether both administrators have 2FA (the read-only role cannot read it).
- A Windows machine for the native build and installer.
- The read-only SSH key is not in git: a new sandbox needs it again, outside the checkout,
  mode 600 (`docs/handoff/SHARED_SSH_ACCESS.md`; key and role expire 2026-11-25 00:00 UTC).
  `.hoplite/` is **not** ignored by git: never `git add -A`.

## Setup in a fresh sandbox (Docker with compose)

    git fetch origin main && git switch -c work/time-release origin/main
    scripts/hub/desktop-tests.sh             # was 348 app + 141 core tests, before fa69c1b
    scripts/hub/desktop-tests.sh check -p tbs-app --locked --target x86_64-pc-windows-gnu
    scripts/hub/backend-tests.sh             # was 1349 passed, 1 skipped, before fa69c1b
    scripts/hub/e2e/run.sh                   # stack, rehearsal, was 173 checks 0 failed; removes the stack

Each script's header says which image and volumes it uses (`tbs-linux-dev:pinned` from
`desktop/tools/linux-dev.Dockerfile`; volume `hub-backend-venv`).

The Time page in Chromium, as run for the 74 checks:

    docker build -f scripts/hub/e2e/page-shots.Dockerfile -t hub-page-shots:pinned scripts/hub/e2e
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

`timelist:local` is the Time server's image used by the stack; if `up` does not build it,
`docker build -t timelist:local time`. The last two `docker run` lines are written from the
running containers' settings, not copied from a script: adjust if they do not start.
Made-up accounts `ui_manager` and `ui_viewer` exist after `run.sh page`; their password is
`E2E_PASSWORD` in `scripts/hub/e2e/e2e.env` (made-up values, no secret).

## Saving work

Commit and push after every milestone with the explicit refspec
`git push origin work/time-hub-admin-finish:refs/heads/work/time-hub-admin-finish`; check
`git status --untracked-files=all` before each commit, add files by name, and compare
`git ls-remote` with `git rev-parse HEAD` after each push. Push labelled WIP before credits
run low.
