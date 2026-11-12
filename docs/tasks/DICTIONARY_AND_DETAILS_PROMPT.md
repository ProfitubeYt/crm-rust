Work in https://github.com/borianahameldoluno24-debug/fix . Start from current main, which contains integrated version 2.0.2, code baseline 7002e14b35377e2ff87bfc3406f985362a5c3925 plus this task document. Create branch work/dictionary-and-details. Preserve the hub, authentication, column separators/order, filters, View, Excel synchronization and existing enrichment/provenance safeguards. Do not replay previous patches or use an older isolated branch.

First read repository instructions, the implementation plan and docs/handoff/ENRICHMENT_PROGRESS.md, START_WINDOW_PROGRESS.md and DEPLOYMENT_2_0_2_20261112.md. Reuse application_detection, operator_mapper, application_backfill, annotation_rule_service and enrich_details; do not build a parallel enrichment system. Production is already at schema 20261112_0033.

1. Remove the Description column

Remove Описание from the desktop table and the View column selector. Do NOT remove Подробности: that is a different field and must remain. Migrate old saved layouts containing the removed column without losing their other customizations or breaking deserialization. Do not delete production data, server fields or Excel protocol fields merely to hide a desktop column.

2. Research and populate the application dictionary

The customer expects the shared application dictionary to grow, but it has not. Establish where it is stored, how it is edited and consumed, whether any automatic population was actually implemented, and whether persistence/cache invalidation/matching is broken. Distinguish a defect from functionality that never existed.

Inventory ALL distinct operator/merchant strings across ALL available transactions. Record receipt frequency, current application and dictionary coverage. Group only proven variants of the same operator, preserving distinctions needed for safe matching. Prioritize frequent unknowns but account for every operator in the completion report.

Research missing and questionable mappings on the internet. Use the browser: focused searches followed by verification on real pages, preferably official company/application/bank websites. Do not scrape raw HTML using curl. Search snippets alone are not evidence. Search only sanitized public organization/service names; never send full receipts, card numbers, phone numbers or private individuals' names to a search engine or external provider.

Distinguish an application/online service, merchant, receipt-source bank and payment intermediary. A merchant/bank name alone does not prove which app the customer used. Preserve the existing documented meaning of Приложение and its confirmed normalizations. If a case only supports a merchant identity rather than an application, record that distinction; leave unknowns unresolved rather than approximate them into the live dictionary.

For each proposed mapping retain the raw/public operator label, normalized form, application, matching rationale, source URL, verification date and confidence/review status. Keep uncertain proposals separate. Verify existing mappings too, but never silently overwrite manual exceptions or confirmed conflicting values.

After preparing a backup and an explicit change preview, add verified mappings through the existing shared dictionary mechanism, without duplicates. Verify that persistence, cache invalidation, parsing and historical dry-runs actually consume the new entries. Do not claim the dictionary was updated unless records were written and checked. If the session only has read access, produce a reproducible, auditable import for the administrator and continue research; do not bypass access restrictions.

3. Fill missing Details from confirmed history

Study already populated Подробности and compare all remaining receipts using meaningful combinations of operator, application, direction, card and stable text identifiers. Copy a complete confirmed label only when supported. Do not invent a payment purpose, person, contract or the meaning of DIK 0626; the number remains unconfirmed and must be treated as opaque text.

Preserve manual/confirmed values, locked periods, permissions and provenance. Do not train on the system's own unverified AI/rule output. Prevent one wrong match from propagating. Repeated processing must be idempotent. Existing thresholds are safeguards: do not reduce them merely to fill more cells.

The previous read-only baseline on 1,932 receipts proposed 457 application fills and 59 details fills, with 46 details suggestions only. These are historical proposals, NOT applied records or guarantees of correctness. Recalculate after dictionary changes and validate precision on confirmed examples. Clearly separate confident fills, suggestions and unresolved cases.

Before historical writes, produce a dry-run with aggregate counts, reasons, conflicts, protected/locked records and the exact affected IDs in a protected administrative change manifest. Do not publish receipt data in git. With authorized write access and a verified backup, apply only confirmed changes, retain before/after values and provenance, and verify the result. Do not apply suggestions or bypass locked periods. If writing is unavailable, report that limitation and provide the administrator-ready import/apply procedure instead of reporting success.

4. Make ongoing processing reliable

Ensure future receipt processing automatically uses verified dictionary entries and confirmed details rules. Test dictionary edit -> cache invalidation -> next receipt -> safe reprocessing. Preserve manual edits and prevent repeated changes/duplicates.

Expose unknown operators for review with frequency, evidence-backed candidate and review status, preferably using existing reference/admin mechanisms. Do not autonomously publish uncertain web/AI matches. Do not enable paid AI or send real receipts to an external provider without separate authorization. No provider/budget decision is needed merely to run deterministic matching and browser research.

5. Production access and data safety

Host 64.188.106.221 / 377344.senko.network; SSH 22; user hoplite-ro; API https://64.188.106.221.nip.io . The application directory /opt/receipt-parser is context only: the restricted account cannot browse it or open a shell.

Shared client fingerprint: SHA256:WEfc++gQxOhnrZCuD+xOfnaBGm0T6djPq8FmHZyDpYE . Verified server ED25519 fingerprint: SHA256:v6EsGV568CnEfD2uUH7tu3pZuXpgJbwjbR9HiXU22SM . The key is installed server-side, but the private half must be securely provisioned outside git into YOUR sandbox. Use the actual path in HOPLITE_SSH_KEY_PATH, mode 600. Never request private-key contents in chat or commit them. The owner's Windows path is not automatically available in Linux. If provisioning is missing, report it and continue independent local work.

Verify the client fingerprint and pin the verified server key. Use IdentitiesOnly=yes and StrictHostKeyChecking=yes. The sole allowed exec command is pgpass; capture its returned DB password-file entry into protected memory/file without printing it. Open only a local tunnel, for example 127.0.0.1:15432 -> 127.0.0.1:9990. Adapt private password-file host/port to the local tunnel. DB user hoplite_ro has SELECT-only grants, 30-second statement timeout and 4 connections shared across sessions; use one. Both role and key expire 2026-11-25 00:00 UTC. Password hashes/salts, TOTP, sessions and Excel tokens are intentionally inaccessible. No sudo, Docker, reverse forwarding, ordinary shell or permission changes are allowed.

The customer wants verified dictionary updates and historical fills, but this existing credential CANNOT write. Request a narrowly scoped administrator-assisted execution mechanism for the reviewed import/apply plan; never weaken the common read-only account or request root passwords in chat. No code deployment, migrations or production restarts are authorized by this task; prepare those as a separate proposal.

Before any authorized production-data write, create and validate a fresh backup through the administrator, plus protected per-record before-values and a batch identifier for targeted rollback. Undo only that batch's changes, preserving later manual edits/new receipts. Never restore the whole database to undo enrichment without separately coordinated recovery authorization.

6. Verification, checkpoints and final report

Maintain docs/handoff/DICTIONARY_DETAILS_PROGRESS.md with evidence and honest completion state. Test removal/migration of Description, dictionary conflicts/duplicates, cache refresh, parsing, false-positive details matches, manual/locked protection, idempotence and consistency across DB/API/desktop/Excel. Use synthetic fixtures, not customer receipts. Report unavailable real Windows/Excel or production-write checks explicitly.

After EACH substantial milestone: review the diff, run focused checks, update progress, commit ALL intended modified AND new files, PUSH and verify the remote SHA. git add -u alone previously lost new files; inspect git status --untracked-files=all. Before credits run out, push clearly labelled WIP with unrun/failing checks and remaining work. No secrets, customer workbooks, private manifests or build artifacts in git; no force push or destructive cleanup.

Use the main Fable 5.2 model for difficult decisions; bounded cheaper helpers for mechanical work only when actually available and worthwhile. Do not invent model IDs or assume Opus 5.5 / extra-high is available or cheaper. Avoid overlapping edits, duplicate surveys and simultaneous full builds.

Final report: why the dictionary did not grow, operators researched/remaining, verified mappings actually added, application/details values actually filled, ambiguous/protected cases, ongoing automation, exact tests, protected backup/change-journal locations, branch and verified pushed SHA. Distinguish researched, proposed, imported and applied; do not claim pending administrator actions happened.
