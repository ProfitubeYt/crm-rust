# Remaining development work — 15 November 2026

Production Time/Hub/admin rollout is complete at schema 0035. Runtime source: `5c0361e`.
Current evidence and limitations: `docs/handoff/DEPLOYMENT_TIME_20261115.md`.

## Priority 1: finish client rollout and provision the Time bot

- [x] Slim native top bar for Time/admin; Checks rail preserved.
- [x] Backend full regression: 1354 passed, 1 skipped; native Windows: 357 app + 139 core.
- [x] Windows release and complete installer including production access configuration built.
- [x] Real Windows/WebView2: Time, global admin, return to Hub, sign-out/user switch,
  and Time-only viewer. Screenshots use synthetic local accounts; no production login tested.
- [x] Launch/chat aware timestamp lock checks fixed; host-independent auth-bot Unix times.
- [x] Initial hosted click guard and event-sequence regression test; closed-month editing hint fixed.
- [x] Production preflight, fully readable backup, migrations 0034/0035, preserved account
  credentials/Telegram links, backend/workers/Time health and HTTPS admin/gateway checks.
- [x] Time named-volume SQLite survives a container restart (see deployment record).
- [ ] Owner acceptance: actual production login and Checks/Excel sync, intended Time grants.
- [ ] Verify a designated Windows 2.0.3 installation upgrade/session preservation before
  publishing to the update channel. Current installer is private and local, not published.
- [ ] Windows-specific checks still open: populated closed month, dark theme and live
  permission revocation while the page stays open; historical Linux checks are recorded separately.
- [ ] Configure Time Telegram bot: new protected token, privacy disabled, verified negative
  group ID and identity bindings. Bot remains OFF; TBS Auth stays separate and running.
- [ ] Targeted server disk housekeeping: about 1.3 GB free. Do not delete volumes/backups blindly.

The owner chose Telegram TBS Auth, not authenticator enrollment. Do not require TOTP
setup as a deployment prerequisite or claim existing scope OTP covers every password login.

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

- [x] Launch/chat timezone comparisons corrected and covered by active/expired lock tests; see the 2026-11-15 deployment record.
- [ ] Document remaining export Operation semantics (inferred/source vs stored/assigned-bank fallbacks) before changing exported content.
- [ ] Keep public source free of genuine credentials and customer receipt samples; private deployment artifacts/backups are not Git content.
- [ ] Publish coherent commits/checkpoints, tests and limitations. No new release claim solely from historical passing tests.
