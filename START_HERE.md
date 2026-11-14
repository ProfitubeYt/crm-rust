# Start here: CRM development handoff

Read [OPEN_TASKS.md](OPEN_TASKS.md), then [the Time continuation prompt](docs/tasks/CONTINUE_TIME_HUB_ADMIN_PROMPT.md), [the consolidated status](docs/handoff/CRM_RUST_STATUS.md) and the latest sections of [the progress record](docs/handoff/TIME_HUB_ADMIN_PROGRESS.md).

## Agent starting prompt

Continue development in THIS repository from current main. Create a non-default work branch. Do not clone an older account's repository, replay existing commits or rewrite finished features.

Priority: finish the preserved Time/Hub/global-CRM-administration work. Existing backend/auth/admin/Time/hosted-page code is already implemented. The latest Time page is WIP: implement the program's slim top bar without a left rail for Time and administration, retaining the Checks rail; inspect the real interface and complete the required verification/release preparation.

Read the linked handoff documents before editing. Older progress sections contain historical claims; do not mistake them for current test results. Reproduce important checks and record actual platform, baseline, output and limitations. A Linux target type-check is not a Windows build; a mock browser is not WebView2; skipped live tests are not integration coverage.

Preserve release 2.0.3 Checks/Excel/enrichment behavior and current data. The 17 dictionary / 656 application / 59 details production batch was already applied: do not repeat it. Preserve the 24-hour confirmed-rule refresh. Production is last known at schema 0033; Time migrations 0034/0035 are not confirmed deployed. Verify live state through authorized access rather than treating old records as today's measurements.

Production deployment has conditional owner approval: first complete the Time UI adaptation, tests and current live preflight, take verified backups, preserve both approved CRM administrators and their 2FA/access, then execute the documented rollout through an authorized administrator. The common read-only SSH key cannot deploy. Never reveal keys/passwords/tokens or weaken restrictions. Missing bot secrets/group setup must be reported, not bypassed. Do not invent the group ID or accept every group.

Follow repository instructions and actual sandbox policy. Use synthetic data and isolated local services for write-based tests. Research current official documentation through the browser if dependencies/services need checking. Do not assume old Docker images, helper paths or credentials exist in a new sandbox.

After EVERY substantial milestone review the diff, run focused checks, update progress, commit ALL intended modified AND new files, PUSH and verify remote SHA. Inspect untracked files; git add -u alone previously lost work. Push labelled WIP BEFORE credits become critical. If push fails, export a downloadable Git bundle and binary patch while the sandbox still exists. Exclude secrets, customer receipts, private manifests, dumps and credential-bearing installers.

Final report: repository/branch/full pushed SHA; implemented and pending tasks; fresh verification results; exact release/deployment status; missing owner actions; backup/rollback locations without exposing protected data. Do not claim deployment or verification that did not occur.

Future AI/R2/activation tasks are separate workstreams, not proof of existing features. Start them only when assigned, using the common contracts and file ownership in docs/tasks/next-ai-workstreams/.
