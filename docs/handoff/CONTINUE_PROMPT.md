Continue the Rust desktop repair project in THIS repository. Begin at its current main, which preserves the previous agent's incomplete PR plus a recovered source file. Read docs/handoff/RECOVERY_STATUS.md, IMPLEMENTATION_PLAN.md and desktop/README.md before editing. This is a continuation, not a rewrite.

## Latest handoff: continue from THIS repository main

Read docs/handoff/SESSION4_STATUS.md first. The latest handoff already merges BOTH third-session branches: verified desktop checkpoint 06c7a1bb65f84a77684a8c4e1452bcf2ffa28a22 and WIP 43dcc1c5ac3969636d3c543b4a46d3a275e49371. Their common ancestor is 2acf4e9. Do NOT reapply the old patches or merge/cherry-pick these commits again.

The third session restored facets.rs, filter fixtures and transactions/mod.rs / menus.rs wiring, and verified a clean desktop checkpoint: 108 core library tests and 235 app tests. Those are historical results, not verification of this new merged snapshot. Live suites returned early without a backend; UI screenshots and the Windows release cross-build were not confirmed.

The merge also preserves the incomplete Excel workbook builder, migration 20261111_0032, its unit tests, and presets.json from the WIP branch. These files are now PRESENT, not lost. Do not recreate them from scratch. Audit their implementation against the existing API/client/VBA contracts, then complete it. backend/tests/test_excel_workbook_api.py is still absent and needs to be implemented. Build/test first to identify actual remaining gaps; old RECOVERY_STATUS.md describes a superseded snapshot and must not be treated as current status.

The final session JSON is available from the owner if a specific question needs it, but do not depend on previous sandbox /tmp files, credentials, containers or background jobs. The old credits expired after starting a low-priority Windows release cross-build; its result is unknown. First preserve and verify the combined state, then finish the feature work below.

## Intended outcomes and priorities

P1: retain the already implemented table fixes: annotation labels, MM.YYYY periods, workbook-like layout, custom layout migration, quick/verify synchronization and search. Preserve separate Тип and Операция columns. Operation now has a recovered implementation with 16 tests. Resolve remaining inputs/parity gaps carefully: API-inferred vs stored transaction type, missing API excel_bank, and server export sort support. Do not fill genuinely absent receipt fields with unrelated values.

P2: finish automatic Excel synchronization. Read the existing Rust Excel API/panel and backend routes to determine the exact contracts. The new client expects POST /api/excel/workbooks with name/password/server_url, returning 201 {workbook: {workbook_id,name,created_at,updated_at,locked_until}}, then GET /api/excel/workbooks/{id}/file. The existing static template is blank because _sync B1/B5/B7 are not configured; inspect VBA to configure all required values correctly. Support password activation before a device grant exists, ownership, captured scope/permissions version, password lockouts, existing grants, listing, re-download and revocation. Preserve vbaProject.bin and untouched ZIP parts byte-for-byte. Never embed plaintext passwords or bearer tokens. Reject unsafe URLs and avoid orphan workbook records. Add the migration, backend tests and documentation. Verify create -> file contents -> wrong-password rejection -> activation -> allowed-data snapshot -> revoke on a LOCAL isolated backend. Real Excel macro execution requires Windows/Excel; report any limitation honestly.

P3: finish filters, using the extensive code already present: removable active chips, clear-all, collapsible summarized groups, date presets, searchable multiselects, live scoped result count, inline validation and draft retention across Esc/reopen. Preserve stored Filters and saved-view compatibility. Add meaningful tests and verify screenshots; do not assume the old intermediate passing suites validate the final snapshot.

P4: finish View: searchable/show-hide/reorder columns, pinning, workbook order/reset widths, density/page size/newest first, saved views lifecycle. Reset layout independently of filters; migrate old state without losing customization. UI text is Russian.

## Production SSH access for YOUR session

Production: 64.188.106.221 / 377344.senko.network, SSH 22. Application: /opt/receipt-parser. API: https://64.188.106.221.nip.io . Read-only DB tunnel account hoplite-ro and DB role hoplite_ro already exist; previous agents' private keys do NOT transfer to you.

A reusable shared SSH key was discussed but has NOT been provisioned or installed by this handoff. Do not claim it exists or that access is ready. If the owner has securely supplied a shared private key to this sandbox, use its actual secret/file path, never ask for its contents in chat, and never commit it. If no shared key is present, explain the missing secure provisioning and continue local work; if production access is necessary, provide a new public-key installation command for the administrator using the restrictions below.

For that fallback, generate your own ED25519 session key outside the tracked tree. Give me the PUBLIC key, its SHA256 fingerprint and a correctly formatted root-side append command for my administrator. Do not request root passwords/tokens in chat. Preserve other authorized keys and restrictive ownership/modes. One authorized_keys line must use:
restrict,port-forwarding,permitopen="127.0.0.1:9990",command="/usr/local/bin/hoplite-ro-gate",expiry-time="20261125" followed by your public key.

Never add permitlisten="none" to authorized_keys: it was confirmed to reject authentication on this OpenSSH 8.9 server. The existing /etc/ssh/sshd_config.d/90-hoplite-ro.conf already restricts forwarding to local and disables passwords, PTY, agent/X11 forwarding. Keep those protections. Validate sshd configuration before any reload; appending a key needs no reload.

Verified server ED25519 host fingerprint: SHA256:v6EsGV568CnEfD2uUH7tu3pZuXpgJbwjbR9HiXU22SM . Pin and compare it. After installation, test auth and tunnel. `ssh hoplite-ro@64.188.106.221 pgpass` retrieves credentials privately; never print/commit them. Only forwarding to 127.0.0.1:9990 and the pgpass forced command are permitted. DB role is SELECT-only, default read-only, 30-second timeout, 4-connection limit; expiry 2026-11-25 00:00 UTC. It cannot read password hashes/salts, TOTP, sessions or Excel tokens; do not broaden grants (access_scopes has secret columns). Ask the administrator to renew access if it has expired. Continue independent local work while access is pending.

Do not change production data, run production migrations or deploy without explicit authorization. Run write-based tests only locally.

## Execution and completion

Use this repository and the account authorized in your current session. If access is missing, request the actual GitHub app/authorization connection; do not change repository visibility. Create a continuation branch and preserve unrelated work. Never force push or discard the interrupted changes.

Before expensive work, commit and push an honest recovery milestone once the first missing pieces are restored, then push each coherent verified milestone. Use git status --untracked-files=all and explicitly include all intended new files (git add -u alone missed them last time). Never include secrets, customer workbooks, build artifacts or environment files. Work sequentially by default to conserve credits. Do not start parallel agents or multiple simultaneous builds without my explicit request; prioritize a working, saved result.

MANDATORY CHECKPOINTS: after each substantial set of changes (restoring compilation, finishing the Excel backend, integrating filters, completing View, or another major fix), review the diff, run the relevant available checks, update IMPLEMENTATION_PLAN.md, commit ALL intended modified AND newly created files, and PUSH immediately. Verify that the remote branch points to the new commit before starting the next large task. Do not accumulate all work until the end of the session. If a session or credit limit is approaching before a milestone is complete, preserve the current source in a clearly labelled WIP commit and push it, documenting failing or unrun checks and unfinished work. Never describe a WIP checkpoint as tested or release-ready. Provide the branch and latest pushed SHA in progress updates after these checkpoints.

Update IMPLEMENTATION_PLAN.md as you verify tasks. Run Rust core/app tests, appropriate backend Excel/VBA tests and a Windows build when available. Report exact fresh results, skipped integration coverage and visual/Excel limitations. Finish with committed and pushed work, verified remote SHA, completed/remaining tasks and a reviewable deployment proposal. Do not deploy automatically.
