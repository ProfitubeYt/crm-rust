Continue the Rust desktop repair project in THIS repository. Begin at its current main, which preserves the previous agent's incomplete PR plus a recovered source file. Read docs/handoff/RECOVERY_STATUS.md, IMPLEMENTATION_PLAN.md and desktop/README.md before editing. This is a continuation, not a rewrite.

## Preserve and repair the interrupted work first

The previous session exhausted credits. Its PR committed only tracked modifications. This repository preserves that commit b9d302076998c39aa456f9bafed963d64319ca84 and its ancestors, plus operation.rs recovered exactly from the JSON tool history (Write + six Edits; VBA symbol table regeneration). Do not apply old patches or repeat old cherry-picks: those commits are already in history.

The preserved snapshot is NOT build-ready. First inventory and restore:
- backend/services/excel_workbook_builder.py;
- a migration after 20260923_0031 for ExcelSyncActivation workbook name/creation metadata, matching the preserved model;
- backend/tests/test_excel_workbook_api.py and test_excel_workbook_builder.py;
- transactions/facets.rs and the filter fixtures;
- missing transactions/mod.rs integration (state, messages, handlers, active-chip rendering and draft lifetime) required by the new filters.rs.

Their exact source was absent from the export, so implement them against the actual preserved call sites and tests, not imaginary contracts. Do not replace the existing filters UI with the old one just to make compilation pass. The full old JSON transcript can be supplied by the user if a specific evidence gap requires it; do not assume access to old /tmp paths or the former sandbox.

## Intended outcomes and priorities

P1: retain the already implemented table fixes: annotation labels, MM.YYYY periods, workbook-like layout, custom layout migration, quick/verify synchronization and search. Preserve separate Тип and Операция columns. Operation now has a recovered implementation with 16 tests. Resolve remaining inputs/parity gaps carefully: API-inferred vs stored transaction type, missing API excel_bank, and server export sort support. Do not fill genuinely absent receipt fields with unrelated values.

P2: finish automatic Excel synchronization. Read the existing Rust Excel API/panel and backend routes to determine the exact contracts. The new client expects POST /api/excel/workbooks with name/password/server_url, returning 201 {workbook: {workbook_id,name,created_at,updated_at,locked_until}}, then GET /api/excel/workbooks/{id}/file. The existing static template is blank because _sync B1/B5/B7 are not configured; inspect VBA to configure all required values correctly. Support password activation before a device grant exists, ownership, captured scope/permissions version, password lockouts, existing grants, listing, re-download and revocation. Preserve vbaProject.bin and untouched ZIP parts byte-for-byte. Never embed plaintext passwords or bearer tokens. Reject unsafe URLs and avoid orphan workbook records. Add the migration, backend tests and documentation. Verify create -> file contents -> wrong-password rejection -> activation -> allowed-data snapshot -> revoke on a LOCAL isolated backend. Real Excel macro execution requires Windows/Excel; report any limitation honestly.

P3: finish filters, using the extensive code already present: removable active chips, clear-all, collapsible summarized groups, date presets, searchable multiselects, live scoped result count, inline validation and draft retention across Esc/reopen. Preserve stored Filters and saved-view compatibility. Add meaningful tests and verify screenshots; do not assume the old intermediate passing suites validate the final snapshot.

P4: finish View: searchable/show-hide/reorder columns, pinning, workbook order/reset widths, density/page size/newest first, saved views lifecycle. Reset layout independently of filters; migrate old state without losing customization. UI text is Russian.

## Production SSH access for YOUR session

Production: 64.188.106.221 / 377344.senko.network, SSH 22. Application: /opt/receipt-parser. API: https://64.188.106.221.nip.io . Read-only DB tunnel account hoplite-ro and DB role hoplite_ro already exist; previous agents' private keys do NOT transfer to you.

Generate your own ED25519 session key outside the tracked tree. Give me the PUBLIC key, its SHA256 fingerprint and a correctly formatted root-side append command for my administrator. Do not request root passwords/tokens in chat. Preserve other authorized keys and restrictive ownership/modes. One authorized_keys line must use:
restrict,port-forwarding,permitopen="127.0.0.1:9990",command="/usr/local/bin/hoplite-ro-gate",expiry-time="20261125" followed by your public key.

Never add permitlisten="none" to authorized_keys: it was confirmed to reject authentication on this OpenSSH 8.9 server. The existing /etc/ssh/sshd_config.d/90-hoplite-ro.conf already restricts forwarding to local and disables passwords, PTY, agent/X11 forwarding. Keep those protections. Validate sshd configuration before any reload; appending a key needs no reload.

Verified server ED25519 host fingerprint: SHA256:v6EsGV568CnEfD2uUH7tu3pZuXpgJbwjbR9HiXU22SM . Pin and compare it. After installation, test auth and tunnel. `ssh hoplite-ro@64.188.106.221 pgpass` retrieves credentials privately; never print/commit them. Only forwarding to 127.0.0.1:9990 and the pgpass forced command are permitted. DB role is SELECT-only, default read-only, 30-second timeout, 4-connection limit; expiry 2026-11-25 00:00 UTC. It cannot read password hashes/salts, TOTP, sessions or Excel tokens; do not broaden grants (access_scopes has secret columns). Ask the administrator to renew access if it has expired. Continue independent local work while access is pending.

Do not change production data, run production migrations or deploy without explicit authorization. Run write-based tests only locally.

## Execution and completion

Use this repository and the account authorized in your current session. If access is missing, request the actual GitHub app/authorization connection; do not change repository visibility. Create a continuation branch and preserve unrelated work. Never force push or discard the interrupted changes.

Before expensive work, commit and push an honest recovery milestone once the first missing pieces are restored, then push each coherent verified milestone. Use git status --untracked-files=all and explicitly include all intended new files (git add -u alone missed them last time). Never include secrets, customer workbooks, build artifacts or environment files. Do not rely on more parallel agents if credits are limited; prioritize a working, saved result.

MANDATORY CHECKPOINTS: after each substantial set of changes (restoring compilation, finishing the Excel backend, integrating filters, completing View, or another major fix), review the diff, run the relevant available checks, update IMPLEMENTATION_PLAN.md, commit ALL intended modified AND newly created files, and PUSH immediately. Verify that the remote branch points to the new commit before starting the next large task. Do not accumulate all work until the end of the session. If a session or credit limit is approaching before a milestone is complete, preserve the current source in a clearly labelled WIP commit and push it, documenting failing or unrun checks and unfinished work. Never describe a WIP checkpoint as tested or release-ready. Provide the branch and latest pushed SHA in progress updates after these checkpoints.

Update IMPLEMENTATION_PLAN.md as you verify tasks. Run Rust core/app tests, appropriate backend Excel/VBA tests and a Windows build when available. Report exact fresh results, skipped integration coverage and visual/Excel limitations. Finish with committed and pushed work, verified remote SHA, completed/remaining tasks and a reviewable deployment proposal. Do not deploy automatically.
