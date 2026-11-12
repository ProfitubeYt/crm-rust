Work in https://github.com/lossamarchallmapoqe18-alt/fix . Start from current main: integrated version 2.0.2, code baseline 7002e14b35377e2ff87bfc3406f985362a5c3925 plus this task document. Create branch work/time-hub-admin. Preserve the existing Checks hub, session behavior, column separators/order, filters, View, Excel synchronization and receipt-enrichment safeguards. Do not replay old patches or use a previous isolated branch.

Goal: integrate the existing Time project into the desktop Hub, provide one account/password system and a simple online administration panel, and prepare the Time Telegram bot for server operation in our group.

Another independent session is researching the application dictionary, enriching receipt details and removing the Description column. Avoid those receipt-processing areas; isolate unavoidable shared-model/auth/UI changes and document integration points.

1. Investigate and reuse Time

Locate time/ and any nested project such as time/timemotion/. Study its actual architecture, database, attendance/time tracking, authentication, Telegram features and deployment. It is believed to be mostly ready; verify that by running relevant checks. Reuse working code instead of rewriting it. Identify exact missing pieces and migration requirements.

2. Time workspace in the Hub

Add a Russian-labelled workspace such as «Учёт времени» alongside «Чеки». Every application launch must still open the Hub. Use one authenticated session across authorized workspaces; switching or returning to the Hub must not trigger another login.

Support Checks-only, Time-only, both, and no workspace access. Unauthorized workspaces must not appear in the Hub/navigation; no-access users need a clear explanation. Hiding UI is insufficient: enforce workspace authorization on every relevant server endpoint, export, background action and bot operation. Local settings or direct HTTP calls must not bypass access.

Keep existing Checks data scopes/roles. Workspace access is an additional layer, not a replacement for current permissions. Do not accidentally grant Time to all existing Checks users or remove their existing Checks access.

3. Unified login and account lifecycle

Inspect existing users/auth/session implementations and extend them where appropriate. Avoid conflicting separate passwords for Time and Checks. If Time already has accounts, explicitly plan safe linking/migration rather than silently joining people by matching names.

Support login/password account creation, password setting/reset, activation/deactivation, workspace grants/revocation, relevant workspace-specific roles and scopes, logout, session expiry and session revocation. Preserve password hashing, 2FA, rate limits and other existing protections. Administrators can reset a password, never retrieve the stored password.

Access changes must affect already running clients without reinstallation. Revoked workspaces disappear and further requests fail. Disabled users and revoked sessions must be denied in both workspaces and the bot. Prevent cached data leakage across user switching/logout or after access revocation; inspect the existing cache design and clear/partition sensitive state appropriately.

4. Simple online administration panel

Build a usable Russian web admin panel to list/search users, create accounts, set/reset passwords, enable/disable users, grant Checks/Time/both, manage relevant role/data restrictions and revoke sessions. Clearly show effective access. Keep it straightforward, with validation, useful feedback and confirmation for disruptive actions.

Protect pages AND APIs with server-side administrator checks. Ordinary users cannot manage accounts or grant themselves rights. Audit creation, resets, access changes, deactivation and session revocation without recording plaintext passwords/tokens. Preserve existing admins; prepare a safe bootstrap/migration path that cannot silently lock the owner out. Do not replace working existing administration facilities unnecessarily.

5. Telegram bot for Time

Inspect and reuse the existing Time bot and its real commands/attendance rules. Prepare it to run reliably on the server and be added by the owner to the intended group. Do not invent business rules if the project does not establish them.

Link Telegram identities securely to centralized accounts with Time permissions. Never ask users to send application passwords to a group. Restrict allowed groups/chats, prevent impersonated attendance, enforce revoked/disabled access and handle duplicate updates/restarts safely. Preserve timezone and existing attendance semantics unless the owner confirms changes.

If token, group ID, linking flow, commands or timezone cannot be established, ask focused questions and continue independent work. Request tokens via protected secrets, not chat; never print or commit them. Creating a bot in the owner's Telegram account or adding it to a group requires the actual account access and explicit instructions; otherwise provide exact owner-side steps.

6. Plan and deployment boundaries

Before shared-auth changes, write an implementation plan with account-data ownership, architecture, trust boundaries, permission migration and acceptance criteria. Use additive/backward-compatible migrations where feasible. Inventory existing permissions and document how Checks users retain their access; clarify any unclear policy rather than guessing.

The owner wants the admin panel and Time bot online. Prepare the complete deployment: secrets, exact config/route/container changes, backups, health checks and rollback. Before the first production deployment or production account/permission migration, present the concrete reviewable plan for approval. Do not deploy, restart services, run migrations or broadly change existing access silently.

Preserve production Checks, Caddy routes, credentials, volumes and unrelated services. Existing production is on schema 20261112_0033. New migrations must follow the actual latest head, coordinating integration with concurrent work. Restoring a DB is not an automatic rollback method because it can lose newer receipts/attendance.

7. Available production investigation access

Read docs/handoff/SHARED_SSH_ACCESS.md. Host 64.188.106.221 (377344.senko.network), SSH 22, user hoplite-ro; Checks API https://64.188.106.221.nip.io; application /opt/receipt-parser. Application path is context only; this account cannot browse files or open a shell.

Installed shared client fingerprint: SHA256:WEfc++gQxOhnrZCuD+xOfnaBGm0T6djPq8FmHZyDpYE . Verified server ED25519 fingerprint: SHA256:v6EsGV568CnEfD2uUH7tu3pZuXpgJbwjbR9HiXU22SM . The private half must be securely provisioned into YOUR sandbox outside git, with its actual mode-600 file path in HOPLITE_SSH_KEY_PATH. It is not automatically available at the owner's Windows path. Never request its contents in chat, commit it or create a replacement automatically. If absent, explain missing provisioning and continue locally.

Verify fingerprints and pin the server key; use IdentitiesOnly=yes and StrictHostKeyChecking=yes. Only the forced command pgpass is allowed to retrieve the private PostgreSQL credential entry: capture it into protected memory/file without logging it. Only a local tunnel to 127.0.0.1:9990 is permitted (for example local 15432). Use hoplite_ro with the privately retrieved database/password, adapt the private password-file port to the tunnel.

This role is SELECT-only on approved Checks tables/non-secret columns, timeout 30s, shared connection limit 4 (use one), and may not cover Time tables. Key/role expire 2026-11-25 00:00 UTC. No hashes/salts, TOTP, sessions or Excel tokens; no sudo, Docker, shell, reverse forwarding or deployment rights. Do not weaken/bypass these restrictions. Deployment-capable access and Time-specific investigation grants must be supplied separately by the administrator if needed; never ask for root credentials in chat.

8. Verification

Test Checks-only, Time-only, both and neither. Verify direct unauthorized requests fail; revocation affects an existing session/client; account deactivation blocks linked bot actions; user switching does not reveal another user's cache; ordinary users cannot call admin APIs; password resets/session revocation work; existing Checks scopes, auth and Excel remain compatible; Time attendance and Telegram linking/group restrictions/duplicate handling/restarts work.

Use isolated local databases and test/stub bot updates for write-based tests, not production attendance or the real group. Run relevant backend/frontend/Rust tests, Windows build and admin-UI screenshot checks. Distinguish mocks/simulators from real runtime and production evidence. Do not report early-return live test functions as successful integration coverage.

Existing desktop/tools/linux-dev.Dockerfile provides the previously approved Docker-only Rust workflow (pinned Rust 1.99.1-bookworm); Cargo uses --locked. Do not assume previous sandbox images, /tmp helpers or caches still exist. Follow actual workspace tooling policy and request specific approval only when required.

9. Checkpoints and report

Maintain docs/handoff/TIME_HUB_ADMIN_PROGRESS.md and your own plan. After EACH substantial milestone: review diff, run focused checks, update progress, commit ALL intended modified AND new files, PUSH and verify remote SHA. Inspect git status --untracked-files=all; git add -u previously missed new files. Push labelled WIP before credits expire, with failures/unrun checks and remaining tasks. No secrets, customer data, credential files or build artifacts in git; no force push or destructive cleanup.

Keep main Fable 5.2 for architecture/auth/security/integration decisions; use at most one genuinely cheaper available helper for narrow mechanical work when worthwhile. Opus 5.5 / extra-high is a preference only if actually supported and appropriate; never invent model IDs or assume it is cheaper. Avoid overlapping edits and duplicate builds.

Final report: reused Time components, Hub/auth/admin/bot functionality, permission model and migration behavior, exact tests/limitations, required secrets/owner actions, concrete deployment/rollback plan or approved execution status, branch and verified pushed SHA. Do not stop at a proposal when authorized local implementation is still possible.
