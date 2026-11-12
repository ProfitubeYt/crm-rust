Continue the unfinished Time/Hub/global-administration task in THIS private repository. Start from its current main and create work/time-hub-admin-resume. This is a prepared merge of deployed release 2.0.3 (f5eede480a6401350c1a13fa4e8ae48ed1acac28) and the previous Time session's last published commit 22b84176c17a1aeb28b6f1f4a0b3c63c31284ee7. Both histories are already included. Do not cherry-pick them again, revert to the old Time baseline or redo completed work from scratch.

Read docs/handoff/TIME_RESUME_STATUS.md, TIME_HUB_ADMIN_PLAN.md and TIME_HUB_ADMIN_PROGRESS.md, plus repository instructions. The last two record the OLD session and contain historical test results and paths; this resume document supersedes their baseline/status. docs/tasks/TIME_HUB_ADMIN_PROMPT.md contains the original requirements, but its old account/repository is not the current publishing target.

## What is preserved

The previous session published five milestones: backend workspace grants/enforcement (0034), Time gateway and hub-mode bot identity checks, separate global CRM administration (0035), and the Russian web administration panel at /admin/. Its final published SHA is 22b8417. No desktop Time integration was published. Credits ran out while preserving a clock icon, notes and helper scripts; those final uncommitted files are permanently unavailable. Recreate only what is actually missing. Do not wait for an old sandbox, /tmp helper, private key, browser screenshot attachment or unpublished commit.

The merge also preserves all later 2.0.3 work: startup Hub, shared session, column separators, current ledger order, filters/View, Description-column removal, receipt application/details enrichment and its safeguards, private-manifest guard and test-fixture fix. Preserve these features. Another agent's dictionary research is not your task.

## Owner decisions already settled

- Administration is GLOBAL for the entire CRM, independent of Time or any workspace. One central users directory, one login/password/session system. It manages Checks, Time and future workspaces.
- global_admin is separate from Checks role and Time role. A global administrator with no workspace still accesses administration; workspace administrators/managers do not automatically receive global administration.
- The administrator enters Telegram IDs; do not invent a mandatory linking-code flow or require passwords in group chats.
- No existing Time data needs migration: start Time empty. There is no real Telegram bot yet; token/group information must be securely supplied by the owner.
- Nobody receives Time automatically during migration. Preserve existing Checks access and protect existing administrators from lockout.
- Application launch always opens the Hub; workspace transitions reuse the authenticated session.

Use canonical time/ for the preserved new integration; time/timemotion/ is an older nested copy and must not be used as an equivalent updated server/auth implementation. The handoff ran a focused backend subset: 116 pass, 1 Windows POSIX-mode assertion failure; see TIME_RESUME_STATUS.md and rerun on Linux rather than treating the whole merge as verified.

## Finish the desktop integration first

1. Inspect the actual auth/me, workspace registry and Time gateway contracts already implemented. Extend core response models with backward-compatible optional/defaulted fields where needed. Preserve behavior against the currently deployed old backend, which has NO workspace/global-admin fields yet; specify and test that compatibility rather than guessing grants.
2. Show «Учёт времени» only to accounts with Time access, «Чеки» only to authorized accounts, and «Администрирование CRM» only to global administrators. Provide a clear no-workspace state; an administrator with no workspaces must still manage accounts.
3. Open the existing Time interface through its implemented Hub/gateway contract, without a second password/account system. Reuse the existing secure web hosting/WebView mechanisms only after reviewing origin restrictions, token handling, navigation and logout. Do not simply reuse a Telegram token/cookie or expose a general-purpose privileged bridge.
4. Provide entry to the global online admin panel independently of Time membership. Do not put central account creation inside Time's own workspace administration.
5. Handle revocation/deactivation, expired sessions and user switching: close forbidden workspace content, remove its navigation, clear/partition caches and WebView credentials, and enforce authorization on the server. Keep the single-session Hub return/logout behavior intact.
6. Recreate a clock icon through the project's conventions if needed. Preserve the current column order, separators and removal of Description; do not import old UI files over them.

## Finish full integration and deployment preparation

Audit the preserved global admin/auth/access code against the merged receipt backend. Run focused tests first, then the complete relevant suites. Verify Checks-only, Time-only, both, neither, global-admin-only, revoked grants, disabled accounts, expired tokens, session revocation and no cross-account cache leakage. Test direct unauthorized APIs, exports/Excel, Time attendance and Telegram identity/group restrictions. Keep current 2FA/rate-limit protections and manual-data/locked-period safeguards.

Use isolated local PostgreSQL/Redis/Time containers and synthetic accounts/bot messages for end-to-end checks. Do not use a real group or production accounts to generate attendance. Distinguish a stub or simulated screenshot from a real browser/native-window test; a skipped/early-return live suite is NOT integration coverage. Build native Windows release with Telegram assets/resources/WebView2 loader/installer when possible; clearly report cross-build/Wine limitations. Avoid claiming the historical 2.0.2 baseline counts verify this merged source.

Prepare compose/service configuration, Time SQLite persistent storage, secrets, /admin and gateway Caddy routes, migration/check commands, health checks and rollback without changing unrelated routes/services. Global users remain in the existing backend; do not create a second credentials database. No automatic broad grants or hidden promotion of workspace managers.

Production currently runs 2.0.3 backend code at schema 20261112_0033. The approved dictionary/details batch has ALREADY been applied: 17 dictionary lines, 656 applications, 59 details. Daily confirmed-rule refresh is enabled with ANNOTATION_DETAILS_RULES_REFRESH_HOURS=24. Do NOT run that batch again, restore old dictionary snapshots or remove this setting accidentally. Time's 0034/0035 are NOT deployed. Rehearse their upgrade/downgrade and exact existing-user access policy before proposing production migration. Do not restore an entire DB as routine rollback: it would lose newer receipts/attendance.

The owner wants the global panel and Time bot online eventually. Complete authorized local implementation, then present the exact production plan for approval BEFORE deploying, migrating accounts/permissions, restarting services or changing production. Keep owner/admin access safe; record which existing accounts gain global_admin under 0035 and why. Prepare rollback for grants/session/cache/client compatibility as well as containers/schema.

## Access and tooling

Read docs/handoff/SHARED_SSH_ACCESS.md. The shared key is installed, but each sandbox still needs its private half securely provisioned outside git, actual path HOPLITE_SSH_KEY_PATH, mode 600. Never request key contents/root passwords in chat or commit secrets. Installed client fingerprint SHA256:WEfc++gQxOhnrZCuD+xOfnaBGm0T6djPq8FmHZyDpYE; verified server ED25519 fingerprint SHA256:v6EsGV568CnEfD2uUH7tu3pZuXpgJbwjbR9HiXU22SM. Pin it and use strict host checking.

Host 64.188.106.221:22, user hoplite-ro. Only pgpass and a local tunnel to 127.0.0.1:9990 are allowed. Fetch DB credentials privately without printing them. DB role SELECT-only on approved Checks tables/non-secret columns; may not cover Time. Timeout 30s, four shared connections (use one), expiry 2026-11-25 00:00 UTC. No shell, Docker, sudo, reverse forwarding or deployment rights. Missing provisioning/grants must not block independent local work; never bypass restrictions.

Use the existing pinned Docker-only Rust toolchain where needed and Cargo --locked; old images/caches/helpers do not carry over automatically. Follow the new sandbox's actual source-control/tooling rules. The old run was blocked by its inline interpreter/heredoc policy: use ordinary script files and explicit non-default branch/refspecs rather than trying to bypass policy. Keep credentials in the configured credential mechanism and exclude them from logs.

## Preserve progress before credits run out

Update docs/handoff/TIME_HUB_ADMIN_PROGRESS.md with the new baseline, fresh evidence and remaining work; preserve prior historical verification notes as historical. After EVERY substantial milestone, review diff, run focused checks, commit ALL intended modified AND newly created files, PUSH and verify the remote SHA. Inspect git status --untracked-files=all; git add -u alone missed files in earlier sessions. Do not delay saving work for a perfect final report or a long build. Push labelled WIP BEFORE credits become critically low, recording known failures and missing artifacts. If push fails, immediately export a downloadable Git bundle and binary patch while the sandbox still exists.

Keep the capable main model for architecture/auth/integration, at most one genuinely cheaper helper for bounded mechanical work when worthwhile. Do not invent available model IDs or duplicate surveys/builds. Prefer finishing and pushing one verified milestone to opening many workstreams.

Final report: repository, branch, full verified remote SHA, implemented/pending features, exact fresh tests and limits, needed secrets/owner actions, concrete deployment/rollback plan and production approval status. Never claim a deployment or lost-sandbox recovery that did not happen.
