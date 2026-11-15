# Continue Time / Hub / global CRM administration

Repository: https://github.com/asintiko/crm-rust (private).
Runtime source deployed on 2026-11-15: `5c0361eb4b293ac354c2a2ffd89e1a140e780e0c`,
branch `codex/time-production-release`. Start from current main if it contains this
commit; otherwise fetch that branch. Do not replay migrations or dictionary fills.

Read `docs/handoff/DEPLOYMENT_TIME_20261115.md` and `OPEN_TASKS.md` first.
Production is now schema `20261112_0035`; backend/workers/TBS Auth and Time are up.
Both approved administrators retain CRM administration; existing account credentials,
Telegram links, Checks access and sessions were preserved. Time roles were not granted
implicitly. The Time bot remains OFF until securely provisioned.

The owner wants Telegram TBS Auth, not an authenticator application. Preserve existing
authentication behavior. Scope OTP and Telegram QR login are not the same as mandatory
Telegram confirmation for every password login; do not claim that latter feature exists.

Remaining release work:
1. With the owner, verify production login, existing Checks/Excel synchronization and
   assign intended Time roles through the global admin panel.
2. Verify the complete 2.1.0 installer upgrade from 2.0.3 on a designated Windows client,
   including saved sessions, before publishing to the update channel. Do not overwrite
   the owner's running installation just to test. Private installer path and hash are
   in the deployment record; no installer or credential belongs in Git.
3. Finish Windows-specific populated closed-month, dark-theme and permission-revocation
   checks. Existing real Windows screenshots used a local synthetic backend and Time.
4. Provision a reissued Time bot token in protected configuration, disable privacy mode,
   verify the actual negative group ID and account bindings. Never guess a group prefix
   or enable unrestricted groups. Keep the independent TBS Auth bot intact.
5. Plan targeted server disk housekeeping (about 1.3 GB free), preserving rollback images,
   data volumes and verified backups.

Do not restart completed UI work. Existing native Windows tests: 357 app + 139 core;
backend Linux: 1354 passed, 1 skipped. Read the evidence limits before claiming more.
Shared read-only SSH access cannot deploy; never copy a private key into the worktree.

Commit and push after substantial milestones, including new files; compare remote SHA
with local HEAD and update handoff evidence before credits run low. Clearly distinguish
real production checks, real native windows, test fixtures and unverified work.
