# Handoff — 2026-11-16, cleanup session (in progress)

Repository `ProfitubeYt/crm-rust`, branch `hoplite/zankle-messana-af2203b8`.
The state before the cleanup is the tag `pre-cleanup-2026-11-16` (commit `9d312e5`).

## Done

- Removed: `mini-app/`, `time/timemotion/`, `pdf_receipt_bot/`,
  `desktop/docs/screenshots/`, root one-off scripts and the receipt samples file,
  two rollout scripts of past releases, old task prompts, handoffs, plans and
  reports (their current facts are in `docs/ARCHITECTURE.md`,
  `docs/DEPLOYMENT.md` and `OPEN_TASKS.md`).
- Build, CI, installer, compose and Caddy files no longer mention Node, npm or
  the `frontend` service. The private first-install file is now
  `security/client-access.json` (git-ignored).
- Backend regression (1354 passed, 1 skipped) and the Time image built on GitHub
  Actions for the first cleanup commit.

## Not finished

- **The native Telegram section is written but has not been compiled or tested.**
  `desktop/crates/tbs-core/src/api/telegram.rs` and
  `desktop/crates/tbs-app/src/screens/telegram/` replace the embedded React page;
  `build.rs` no longer embeds `frontend/dist-telegram`. Until it compiles, passes
  its tests and is seen in a window, the desktop client on this branch is not a
  working build.
- **`frontend/` (React/Electron sources) is still in the repository** and is
  removed only after the native section is verified. Nothing builds it any more.
- No Windows build, installer or window check was made in this session yet.

## Continuation prompt

    Continue the cleanup of ProfitubeYt/crm-rust on branch
    hoplite/zankle-messana-af2203b8 (tag pre-cleanup-2026-11-16 is the state before
    it). Read README.md, docs/HANDOFF.md and docs/ARCHITECTURE.md first.
    1. Make the native Telegram section compile and pass its tests:
       scripts/hub/desktop-tests.sh check -p tbs-app --features devtools --tests --locked,
       then scripts/hub/desktop-tests.sh and
       scripts/hub/desktop-tests.sh test -p tbs-app --features devtools --locked.
       Compare it with frontend/src/pages/UserbotPage.tsx and
       frontend/src/components/telegram/ so that no function of the old page is
       lost; no placeholders.
    2. See it in a window: desktop/tools/stub-server.py with
       desktop/tools/window-check.py on Linux, then the Windows client workflow
       (.github/workflows/windows-client.yml) for the native build, the setup and
       a picture of the Telegram section on a Windows runner.
    3. Only then delete frontend/, update README.md and this file, commit and push.
    Do not deploy, publish a release or change production. Do not start the
    features in OPEN_TASKS.md.
