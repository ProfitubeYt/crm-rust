# Continue: Time / Hub / global CRM administration

Repository: https://github.com/josukekleinjewite51-ui/fix . Branch `work/time-hub-admin-finish`
holds everything described here; start from it (or from `main` once it is merged). Do not
replay earlier commits or start from an older baseline.

Read first, in this order:

1. `docs/handoff/TIME_HUB_ADMIN_PROGRESS.md`, section "Finish session — 2026-11-12": what was
   run, what came out, what was found and fixed, what is still open.
2. `docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md`: the plan put to the owner. **Not approved and
   not executed.**
3. `docs/tasks/RESUME_TIME_HUB_ADMIN_PROMPT.md`: the owner's task and its limits. Still
   authoritative.

Nothing is deployed. Do not deploy, migrate, change permissions or restart production
without the owner's approval of the concrete plan.

## Where it stands

All seven steps of the previous continuation are done: the desktop suite, the Windows
cross-check, the local end-to-end stack, the migration rehearsal, a real window (Linux
WebKitGTK), the whole backend suite and the deployment document. The record has the numbers.

## What is left

1. **The owner's answers** to section 2 of the deployment document: the window, who carries
   it out, the two CRM administrators, the second factor, the Time bot's token and group.
2. **The deployment itself**, by somebody with a root shell on the server, step by step from
   the document. The shared read-only key cannot do it.
3. **The Windows client**: a native build and installer with the Hub entries, and the hosted
   pages seen in WebView2 (first size of a page, its folder after sign-out, the header at the
   default width). Never done; needs a Windows machine.
4. **The Time page for a viewer** offers the editing controls and only says «только просмотр»
   when one is used; `body.readonly` is set in `app.js` and no rule in `app.css` uses it.
5. Three more comparisons of a naive `utcnow()` with a time-zone-aware column (launch
   password lock, chat password lock); the same kind as the sign-in lockout fixed here. Not
   part of this work; listed in the record.

## To run things again

- `scripts/hub/desktop-tests.sh`, `scripts/hub/backend-tests.sh`, `scripts/hub/e2e/run.sh`
  (one command: stack, rehearsal, 173 checks, stack removed; about five minutes once the
  images exist). `docker build -t timelist:local time` first, about six minutes.
- The window: `desktop/README.md`, "A window without Windows", and the progress record.
- Inline interpreter heredocs and `sh -n` on a new script are refused; write a file and run
  it. `*.log` is ignored by git.
- The read-only SSH key is not in git; a new sandbox needs it supplied again, outside the
  worktree, mode 600. The key and the database role expire on 2026-11-25.
