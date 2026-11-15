# crm-rust: consolidated handoff — 14 November 2026

> Current status (2026-11-15): production is schema 0035; Time/admin deployed. Native Windows WebView2 and the complete installer were checked to the limits in [the deployment record](DEPLOYMENT_TIME_20261115.md). Earlier sections below are historical. See `OPEN_TASKS.md` for remaining work.

Repository: https://github.com/asintiko/crm-rust . This is the complete preserved project and reachable source history through Time WIP 04101ae3b30a1fc26ba290d862cdc8ea2219a6ef, plus current handoff documents. It is NOT a claim that Time is released or production was checked today. Runtime secrets, private SSH keys, database dumps, private journals, local dependencies and credential-bearing installers are deliberately outside Git.

## Latest confirmed production work

Last confirmed rollout: release 2.0.3, backend schema 20261112_0033. Checks, Excel preparation/sync backend, startup Hub, filters/View, column separators/order and Description-column removal are preserved. Dictionary/details batch was applied exactly once: 17 dictionary lines, 656 Application fills and 59 Details fills, with private before/after journal and backups; daily confirmed-rule refresh is 24 hours. Never repeat the batch. See DEPLOYMENT_2_0_3_20261112.md. No fresh server inspection was made for this repository transfer.

## Preserved Time/global-administration WIP

Central workspace grants and separate global_admin, migrations 0034/0035, Time gateway/hub mode, bot identity checks, Russian global administration panel, desktop hosted-page integration, lockout fix, local e2e/migration rehearsal and deployment plan are committed. The latest Time page change has compact labelled tabs and no edit controls for viewers.

Historical evidence: 348 app/141 core tests, backend 1349 passed/1 skipped, local stack 173 checks, migration round trip and Linux hosted windows. The final Time page change had 74 headless Chromium checks, but suites were not rerun after it and its final pictures were not visually reviewed. Windows had only a cross-target type check; no complete native executable/installer for the Time feature. Do not describe prior desktop installers 2.0.3 as containing Time.

## Finish before Time production release

1. Recreate and inspect screenshots of the latest Time page (manager/viewer, both themes, narrow layout and standalone mode).
2. Implement the PROGRAM'S slim top bar with no left rail for Time and CRM administration. The visible hamburger is the shell's rail toggle; the embedded Time page never had a hamburger. Checks keeps its rail. Preserve shared sessions/access checks.
3. Rerun affected desktop/core, Time, backend and local e2e/migration checks; rebuild the Time container. Run the actual hosted windows and clearly state platform/stub limitations.
4. Build a new native Windows release with embedded assets, WebView2 loader, icon/version resources and installer. Verify Windows-specific behavior; do not equate a Linux type check or Wine run with it.
5. Update the deployment document to include owner approval, backups, administrator preservation/2FA, exact live preflight, rollout and rollback. Production deployment is approved AFTER UI adaptation and required preflight/backups; do not skip these conditions.
6. Use an authorized deployment executor. Shared hoplite-ro SSH access is SELECT-only and cannot deploy. Preserve latest Checks/dictionary data and existing Caddy routes/volumes. Migrations 0034/0035 are not yet confirmed deployed.
7. Verify both approved CRM administrators (admin and stlee) retain access, and check/enroll 2FA safely without disabling protections or locking them out.
8. Finish the Telegram bot: securely provision a valid token, disable privacy mode, obtain and verify the actual group ID, and validate permissions/identity binding. Positive 1276772106 was not a verified group ID. Never guess its prefix or allow every group. Time/admin may be prepared independently; do not claim the bot works while blocked. No token is in Git.

Authoritative continuation: docs/tasks/CONTINUE_TIME_HUB_ADMIN_PROMPT.md. Original session details remain in TIME_HUB_ADMIN_PROGRESS.md and TIME_HUB_ADMIN_DEPLOYMENT.md. Old lower sections contain historical status; use the latest section first.

## Future work, not implemented yet

Three owner-approved task specifications are under docs/tasks/next-ai-workstreams/:
- Account activation: admin-created login, single-use activation code, user chooses password; one session/authorized workspaces. Private R2 originals plus two successful weekly backup generations. Live PostgreSQL remains on the server.
- Main Checks AI chat: common scoped tools, structured tables/charts/sanitized SVG, reconciliation with authorized originals, explicit confirmation for data changes/bulk operations. Provider/model/credentials supplied later.
- Table AI mini-chat: shared AI service, semantic table/filter/selection/export commands and a separate in-app animated cursor; not arbitrary OS automation.

These prompts must be reconciled with the finished global-admin/Time contracts before parallel execution. All agents must share the same baseline, own separate files/contracts and push complete milestones before credits run low. Actual Cloudflare free-tier limits/pricing must be checked at implementation time; indefinite receipt retention is not a promise of free unlimited storage.
