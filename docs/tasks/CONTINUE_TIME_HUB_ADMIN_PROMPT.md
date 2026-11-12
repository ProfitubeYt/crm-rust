# Continue: Time / Hub / global CRM administration

Repository: https://github.com/josukekleinjewite51-ui/fix . Current main already includes
all work through 10f7ea811da0a29452fe3f41ac854268b4ad83ad plus this handoff correction.
Create work/time-hub-admin-finish from CURRENT main. Do not use the previous account's
older main, replay existing commits or publish to the old repository. Application code
is unchanged by this handoff.

Read first, in this order:

1. `docs/handoff/TIME_HUB_ADMIN_PROGRESS.md`, section "Resume session — 2026-11-12": what is
   done, what was actually tested, what is open, the e2e design and the sandbox rules.
2. `docs/tasks/RESUME_TIME_HUB_ADMIN_PROMPT.md`: the owner's task and its limits. Still
   authoritative; this file only says where to pick it up.
3. `docs/handoff/TIME_HUB_ADMIN_PLAN.md` section 9 and `docs/handoff/DEPLOYMENT_2_0_3_20261112.md`
   before writing the deployment document.

Nothing is deployed. Do not deploy, migrate, change permissions or restart production
without the owner's approval of a concrete plan.

## Where it stopped

The previous session was cut short by credits while finishing priority 1 and starting
priority 2. State of the last commit: the desktop tests pass (348 app, 141 core; how they were run is in the progress record). Nothing else was run on it.

## Resume here, in this order

1. **Desktop suite.** `scripts/hub/desktop-tests.sh` (or the `docker run` line in the progress
   record). If the line above reports failures, fix those first: the changes of commit
   `924a2b7` (session epoch in `tbs-core/src/api`, hosted page ownership in
   `tbs-app/src/screens/hosted`, account refresh order and folder removal in `app.rs`,
   settings sections) had not been run before they were pushed.
2. **Windows code paths.** Never compiled in that session:
   `scripts/hub/desktop-tests.sh check -p tbs-app --locked --target x86_64-pc-windows-gnu`.
   The hosted pages have `cfg(windows)` origins (`http://tbstime.localhost`). Report the
   limits of a cross-check honestly; a native Windows build and installer were not made.
3. **Local end-to-end stack** (`scripts/hub/e2e/`): `compose.yml`, `e2e.env` and
   `fake_telegram.py` exist and were never run. Still to write, as designed in the progress
   record: `prepare.py` (root-access config, local Caddyfile, 2.0.3-shaped schema and
   accounts), `checks.py` (the access matrix, revocation, sessions, exports, bot attendance)
   and `run.sh`. Then run it and record the real output.
4. **Migration rehearsal** on the stack's PostgreSQL 15: schema of `f5eede4` stamped
   `20261112_0033`, `prechecks.sql`, `alembic upgrade head`, `postchecks.sql`, the 2.0.3 code
   against the new schema, `downgrade 20261112_0033`, `upgrade head`.
5. **A real window.** `desktop/tools/window-check.py` (Xvfb, WebKitGTK) against the local
   stack: Time and the admin panel opened from the Hub, revocation while open, sign-out and a
   second account. Say plainly that it is Linux WebKitGTK, not WebView2.
6. **Whole backend suite** once more (`scripts/hub/backend-tests.sh`, about five minutes; run
   it detached), since `.env.example` and `deploy/Caddyfile` changed after the last full run.
7. **Deployment document** `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md` with exact commands,
   health checks and rollback, from the artefacts in `deploy/time-hub/` and the facts in the
   progress record. Then the final report and the request for approval.

Commit and push after every one of these with an explicit refspec
(`git push origin work/time-hub-admin-finish:refs/heads/work/time-hub-admin-finish`), check
`git status --untracked-files=all` before each commit and compare `git ls-remote` with
`git rev-parse HEAD` after each push.

## Things that cost time last session

- Inline interpreter heredocs (`python3 - <<EOF`) are refused. Write a script file and run it.
- A shell command is killed at its timeout (two minutes by default, ten at most), background
  ones too. Run long work as `docker run -d …`, then `docker wait` and `docker logs`.
- Docker may bind-mount only the checkout (`/tmp/hoplite/workspace`, `/tmp/hoplite/workspaces`).
- Tool output over about 2,000 tokens is replaced by a summary: read code in pieces of at most
  a hundred lines and keep command output short.
- The Docker images and volumes of the last sandbox (`tbs-linux-dev:pinned`, `timelist:local`,
  `hub-backend-venv`, the cargo caches) do not carry over. The first desktop build takes long.
- The read-only SSH key is not in git. A new sandbox needs it supplied again, outside the
  worktree, mode 600; the key and the database role expire on 2026-11-25.
