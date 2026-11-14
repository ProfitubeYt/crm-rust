# Remaining development work — 14 November 2026

This is a source/handoff review, not a fresh production audit. Preserved Time code: 04101ae; the work after it is on `vorflux/time-release`. Complete source history and task context were consolidated at a486e47. Production last confirmed as release 2.0.3/schema 0033; no Time rollout has been performed by this handoff.

## Priority 1: finish Time and global administration

State on 14 November 2026, branch `vorflux/time-release`, pull request https://github.com/asintiko/crm-rust/pull/1 . The results below are from a Linux sandbox, and one run of GitHub Actions on Windows Server 2022; nothing is deployed. Figures and limits: docs/handoff/TIME_HUB_ADMIN_PROGRESS.md, "Release session".

- [x] The desktop shell's slim top bar, no left column or hamburger for Time and CRM administration; Checks navigation unchanged; the Time page not rewritten.
- [x] Manager/viewer, both themes, narrow viewport, closed-month and standalone pictures of the Time page regenerated (74 Chromium checks) and looked at.
- [x] Core/app/Time/backend tests and the local end-to-end/migration stack rerun after the UI change: 351 app/141 core, Time 46, backend 1349 pass/1 skipped, 173 e2e checks.
- [x] The client's real window on Linux (Xvfb, WebKitGTK) against the stack: Time as manager and viewer, CRM administration, both themes, 1360 and 1024 wide, sign-out, user switch, revocation while open. Pictures: docs/handoff/evidence/time-hub-window-20261114/.
- [ ] Time image: rebuilt and healthy in Hub mode in the stack. Persistent SQLite storage across a restart is still to verify.
- [x] Windows client 2.1.0 built on Windows by the workflow `.github/workflows/windows-client.yml` (https://github.com/asintiko/crm-rust/actions/runs/43598951623): 351 app/139 core tests, release executable, setup without client-access.json (SHA-256 234848f6…0bbb2b, kept as a run artifact until 21 November 2026), silent install, first start to the sign-in window. Evidence: docs/handoff/evidence/time-hub-windows-run-20261114.txt.
- [ ] Windows client 2.1.0 on a real Windows desktop with a server: sign in, open Time and CRM administration in WebView2, the bar, hints, sign-out, user switch, the update from 2.0.3. Not done anywhere. Do not publish a setup to the update channel before it.
- [ ] Check on Windows: a click on «Открыть» in the start window also lands in the page that opens (seen on WebKitGTK with xdotool).
- [x] Remaining low-severity review findings: each is recorded with its reason for deferral in the progress record. One is closed by the new layout (header width).
- [ ] Correct the naive-against-aware lock comparisons of the launch password and the chat password (backend/api/dependencies.py, api/routes/security.py, api/routes/telegram_client.py) with tests on PostgreSQL.
- [x] Deployment plan updated with the owner's decisions, both administrators, readable backups, the bot and the 2FA steps; its new commands tried on the local stack only.
- [ ] Execute the conditionally approved deployment through an authorised administrator: fresh live preflight, verified backups, migrations 0034/0035, both approved global admins kept, 2FA/enrollment/recovery without lockout. Blocked: no executor with a root shell, no key in the sandbox. Do not expose credentials in issues.
- [ ] Configure the real Telegram bot securely: reissued token from protected configuration, privacy disabled, verified negative group ID, membership/rights and explicit account linkage. No arbitrary groups; report a blocked bot separately from a running Time service.
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
