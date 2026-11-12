# Resume Time/Hub/global admin after credits exhausted

## Sources preserved locally

- Current production/code release 2.0.3: f5eede480a6401350c1a13fa4e8ae48ed1acac28.
- Last published Time milestone: 22b84176c17a1aeb28b6f1f4a0b3c63c31284ee7.
- Prepared merge contains both histories; no textual conflicts. This does not prove runtime compatibility.
- The previous sandbox is gone. Final icon/helper/progress edits were not committed or pushed; no further unpublished source is available.

## Already published by the Time agent

Five commits implement the plan, workspace authorization/gateway, Time hub-mode and Telegram identity checks, global_admin independent of workspace roles, and a global Russian CRM admin panel. Migrations 0034 and 0035 follow current 0033. Historical browser checks used local PostgreSQL/Redis, fake accounts and a modified application lifespan; never production. Historical backend suite reported 1330 passing tests. These reports do not verify the merged 2.0.3 snapshot.

## Missing

- Desktop Time workspace and global-administration entry, workspace visibility/access handling, revocation/cache/session integration.
- Final local end-to-end test of the combined CRM + Time + admin system.
- Fresh Windows release/installer validation.
- Real Telegram token/group setup (owner has not created/supplied the bot).
- Deployment artifacts, final permission migration review, approval and rollout. Nothing from the Time task is on production yet.

## Owner policy already settled

Global accounts/admin panel belong to the whole CRM. Global administration does not imply workspace membership and vice versa. Telegram IDs are entered by the administrator. Start Time with no existing data; nobody is granted Time automatically. Preserve current Checks access and administrator access.

Production remains schema 0033 and version 2.0.3. The 17/656/59 dictionary/details batch is already applied, and daily rule refresh is set to 24h. Preserve it; do not repeat the import. Protected operational artifacts are not in this repository.

Read docs/tasks/RESUME_TIME_HUB_ADMIN_PROMPT.md as the authoritative continuation task. The old original prompt points at the previous GitHub account and is context only. No new account has yet been chosen at handoff preparation time.

## Fresh handoff verification

- 270 backend Python files parsed successfully; merge/diff whitespace checks passed.
- Focused Windows SQLite run of workspace access, global-admin APIs, Time gateway, panel/static guards, workspace migrations, dictionary and change-manifest tests: 116 passed, 1 failed. The failure is the existing POSIX assertion `stat.S_IMODE(...) == 0o600` in test_change_manifest.py; Windows reports 0666 despite chmod. It occurs after apply/protection assertions and does not establish full rollback coverage on this platform. Do not suppress the assertion globally or claim this is an authorization regression; rerun on Linux and use Windows ACL checks if Windows support for that administrative tool is needed.
- No full backend suite, merged Rust build, real bot or production check was run for this handoff. Original local log remains outside git at output/time-resume-verification.log.
- Use canonical time/ (the published integration changes are there). time/timemotion/ is the older nested copy, not an equivalent updated deployment source. Never accidentally deploy it with the old standalone authentication model.
