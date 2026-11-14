# Continue: Time / Hub / global CRM administration

Repository: https://github.com/asintiko/crm-rust . Current main includes
the full preserved history through 04101ae3b30a1fc26ba290d862cdc8ea2219a6ef plus
repository handoff documents. The work of 2026-11-14 is on the branch
`vorflux/time-release`, pull request https://github.com/asintiko/crm-rust/pull/1 :
continue on that branch, or from `main` once it is merged. Do not
restart from an older account/repository or replay existing commits. Read
docs/handoff/CRM_RUST_STATUS.md for the consolidated status and verification limits.

**Nothing is deployed. Production is release 2.0.3 at schema `20261112_0033`, untouched.**

Read first, in this order:

1. `docs/handoff/TIME_HUB_ADMIN_PROGRESS.md`, the top section "Release session": what was
   built, run and seen on 2026-11-14 and what was not. Then "UI and release session" for the
   owner's decisions, and "Finish session" for the rehearsal and the method of the window run.
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
  program, on the committed files. Not a desktop window, not Windows.

Done on 2026-11-14 on `vorflux/time-release` (figures in the progress record, "Release
session"; all on Linux, none on Windows):

- The pictures of the Chromium run were looked at (74 checks, 0 failed, run again).
- The program's slim top bar is written: no rail and no hamburger on «Учёт времени» and
  «Администрирование CRM», Checks keeps its rail.
- Rerun: the desktop suite (351 app + 141 core), the Windows type check, Time's tests (46), a
  rebuilt Time image, the backend suite (1349 passed, 1 skipped), `scripts/hub/e2e/run.sh`
  (173 checks).
- The real window under Xvfb with WebKitGTK against the stack: pictures in
  `docs/handoff/evidence/time-hub-window-20261114/`.
- Client version 2.1.0. A Windows executable cross-linked in the sandbox and a setup compiled
  by Inno Setup under Wine, without `client-access.json`: **never started on Windows**, not
  committed, not published.
- The deployment document carries the owner's decisions, the bot and the 2FA steps.

Not done, in the order to do it:

1. **Windows.** Build and try the client on a real Windows machine with WebView2:
   `pwsh desktop/tools/build-installer.ps1` there, or the workflow
   `.github/workflows/windows-client.yml` (hand-started; GitHub offers it only once the file
   is on `main`; a Windows runner uses paid minutes, so ask the owner). Try: the bar on both
   pages, the first sizing, the hints over the web view, sign-out and user switch, the update
   from 2.0.3, and whether the click on «Открыть» also lands in the page that opens (seen on
   WebKitGTK with `xdotool`). Only then publish to `asintiko/parcer20-updates`, with the
   owner's word.
2. **Deployment.** An authorised administrator runs
   `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md` from its section 2 on: the live read-only
   pre-check, the backups read back, the migration, the checks of section 6. This sandbox
   cannot: it has no key and the shared access is read-only.
3. **The bot**, section 7 of that document: a reissued token, privacy mode off, the group's
   verified negative ID. Polling stays off until then.
4. After the rollout: the health checks of section 6 and the access matrix on the server.
5. The deferred review findings in the progress record's table, the naive-against-aware lock
   comparisons first.

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
- A Windows machine with WebView2 to build and try client 2.1.0, and
  `frontend/client-access.json` on it for a setup that can do a first installation.
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
