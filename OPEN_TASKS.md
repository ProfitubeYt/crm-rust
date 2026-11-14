# Remaining development work — 14 November 2026

This is a source/handoff review, not a fresh production audit. Latest preserved Time code: 04101ae. Complete source history and task context were consolidated at a486e47. Production last confirmed as release 2.0.3/schema 0033; no Time rollout has been performed by this handoff.

## Priority 1: finish Time and global administration

- [ ] Implement the desktop SHELL's slim top bar, without the left column/hamburger for Time and CRM administration. Keep Checks navigation unchanged. Do not remove a nonexistent inner Time hamburger or rewrite the compact Time page unnecessarily.
- [ ] Regenerate and visually inspect manager/viewer, both themes, narrow viewport, closed-month and standalone screenshots of the latest Time page.
- [ ] Rerun affected core/app/Time/backend tests and the local end-to-end/migration stack after final UI changes. Prior evidence: 348 app/141 core, backend 1349 pass/1 skipped, 173 e2e checks; final page alone 74 Chromium checks. These are historical, not fresh verification of a future release.
- [ ] Rebuild Time image, verify its healthcheck in Hub mode and persistent SQLite storage.
- [ ] Build a complete native Windows executable/installer with embedded pages/Telegram assets, WebView2 loader and icon/version resources; test real Windows navigation, sessions and WebView2. Prior GNU target check is type-check only.
- [ ] Resolve remaining low-severity review findings recorded in the progress document, or explicitly document why deferred.
- [ ] Update and execute the conditionally approved deployment plan after fresh preflight/backups, preserving existing accounts and current Checks/Excel/data behavior. Rehearse migrations 0034/0035 and rollback, keep both approved global admins, verify 2FA/enrollment/recovery without lockout. Do not expose credentials in issues.
- [ ] Configure the real Telegram bot securely: valid token from protected configuration, privacy disabled, verified negative group ID, membership/rights and explicit account linkage. No arbitrary groups; report blocked bot separately from a running Time service.
- [ ] After rollout verify access matrix, revocation while pages are open, sign-out/user switch, sessions, Time gateway, attendance and admin panel. Preserve unrelated Caddy services/volumes and 24-hour Details rule refresh.

Reference: docs/tasks/CONTINUE_TIME_HUB_ADMIN_PROMPT.md and docs/handoff/TIME_HUB_ADMIN_DEPLOYMENT.md.

## Priority 2: account activation and R2 storage

- [ ] Admin-created login + expiring single-use activation code -> user sets/confirms password. Existing accounts keep their password/session behavior; one login unlocks authorized Hub workspaces.
- [ ] Private Cloudflare R2 storage for exact original text and attachments, authorized retrieval, content hashes and durable retry/outbox. Safely migrate consumers; do not blank raw_message or delete local originals before verified upload/cutover.
- [ ] Consistent weekly backup generation, isolated restore checks and retention of the latest TWO successful generations. Failed uploads never delete the last good backups. Originals are not automatically expired. Live PostgreSQL stays on the server.
- [ ] Verify current Cloudflare pricing/limits before claiming indefinite retention fits a free tier.

Task: docs/tasks/next-ai-workstreams/PROMPT_1_ACCOUNTS_R2.md.

## Priority 3: shared Checks AI service and main chat

- [ ] Checks menu entry, provider-neutral OpenAI-compatible backend, safe streaming/cancellation, scoped domain tools, audit and explicit approval of exact data changes/bulk actions. Model/provider/key supplied later; no live calls without provisioning.
- [ ] Safe typed table/chart/SVG artifacts, library research and real renderer. No executable model HTML/JS or untrusted SVG resources.
- [ ] Authorized reconciliation against original receipt storage with evidence and uncertainty; no automatic correction or arbitrary SQL/shell/bucket access.

Task: docs/tasks/next-ai-workstreams/PROMPT_2_CHECKS_AI_CHAT.md.

## Priority 4: table AI copilot

- [ ] Mini-chat using the SAME AI backend/proposal system, stable semantic table/filter/selection/view/export actions and contextual help.
- [ ] Separate in-app animated pointer tied to actual state transitions, never OS cursor takeover or fake success. Stop/manual interruption, stale row/scope checks, DPI/virtualization/pinning and both themes.
- [ ] Persistent edits/bulk operations retain shared server authorization/confirmation; test with mock provider first.

Task: docs/tasks/next-ai-workstreams/PROMPT_3_TABLE_AI_COPILOT.md.

## Cross-cutting release obligations

- [ ] Recheck known timezone lock comparisons (launch/chat password checks) identified in the Time report; do not assert production exploitation without evidence.
- [ ] Document remaining export Operation semantics (inferred/source vs stored/assigned-bank fallbacks) before changing exported content.
- [ ] Keep public source free of genuine credentials and customer receipt samples; private deployment artifacts/backups are not Git content.
- [ ] Publish coherent commits/checkpoints, tests and limitations. No new release claim solely from historical passing tests.
