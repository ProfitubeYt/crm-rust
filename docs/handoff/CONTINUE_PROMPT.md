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

A reusable shared ED25519 key is now INSTALLED and TESTED on production. Use it instead of generating a new key for every session, once its private half has been securely mounted in your sandbox. Read docs/handoff/SHARED_SSH_ACCESS.md.

Shared key fingerprint: SHA256:WEfc++gQxOhnrZCuD+xOfnaBGm0T6djPq8FmHZyDpYE . The owner's local private-key file is C:/Users/heper/.ssh/hoplite-shared-ro-ed25519; that Windows path does NOT exist automatically in your Linux sandbox. The owner must securely supply the private key through an actual protected secret/file mounting mechanism outside the tracked tree, then set HOPLITE_SSH_KEY_PATH to its real sandbox path. Do not assume a mount, file or secret exists, ask for the private key in chat, or write it into git. If not provisioned yet, report the missing secure mount and continue independent local work. No new server-side public-key installation is necessary for this shared key.

When provisioned: chmod 600 "$HOPLITE_SSH_KEY_PATH"; verify its public fingerprint; pin server ED25519 fingerprint SHA256:v6EsGV568CnEfD2uUH7tu3pZuXpgJbwjbR9HiXU22SM using the separately verified host key. Connect as hoplite-ro@64.188.106.221 on port 22 with IdentitiesOnly=yes. Only the pgpass forced command and a local tunnel to 127.0.0.1:9990 are permitted. Fetch the DB credential privately, never print/commit it. DB role hoplite_ro is SELECT-only, default read-only, 30-second timeout and 4-connection limit. No hashes/salts, TOTP, sessions or Excel tokens are readable. Never broaden grants.

Both shared key and DB role expire on 2026-11-25 00:00 UTC. Do not assume permanent access; request renewal if needed. Previous per-session keys remain installed and must not be removed without instruction. The server's /etc/ssh/sshd_config.d/90-hoplite-ro.conf blocks remote forwarding, passwords, PTY, agent/X11 forwarding. Never use permitlisten="none" in authorized_keys: it caused authentication failure on this OpenSSH 8.9 server. Authentication, pgpass retrieval, permitted tunnel and shell denial were tested after installing the shared key.

Do not change production data, run production migrations or deploy without explicit authorization. Run write-based tests only locally.

## Execution and completion

Use this repository and the account authorized in your current session. If access is missing, request the actual GitHub app/authorization connection; do not change repository visibility. Create a continuation branch and preserve unrelated work. Never force push or discard the interrupted changes.

Before expensive work, commit and push an honest recovery milestone once the first missing pieces are restored, then push each coherent verified milestone. Use git status --untracked-files=all and explicitly include all intended new files (git add -u alone missed them last time). Never include secrets, customer workbooks, build artifacts or environment files. Use the model-routing instructions below to conserve credits; avoid multiple simultaneous builds and prioritize a working, saved result.

MANDATORY CHECKPOINTS: after each substantial set of changes (restoring compilation, finishing the Excel backend, integrating filters, completing View, or another major fix), review the diff, run the relevant available checks, update IMPLEMENTATION_PLAN.md, commit ALL intended modified AND newly created files, and PUSH immediately. Verify that the remote branch points to the new commit before starting the next large task. Do not accumulate all work until the end of the session. If a session or credit limit is approaching before a milestone is complete, preserve the current source in a clearly labelled WIP commit and push it, documenting failing or unrun checks and unfinished work. Never describe a WIP checkpoint as tested or release-ready. Provide the branch and latest pushed SHA in progress updates after these checkpoints.

Update IMPLEMENTATION_PLAN.md as you verify tasks. Run Rust core/app tests, appropriate backend Excel/VBA tests and a Windows build when available. Report exact fresh results, skipped integration coverage and visual/Excel limitations. Finish with committed and pushed work, verified remote SHA, completed/remaining tasks and a reviewable deployment proposal. Do not deploy automatically.

## Model routing and credit budget (owner preference)

Keep the main model (Fable 5.2 at its highest available reasoning setting, as selected by the owner) for architectural decisions, difficult data-flow/debugging work, integration, review and final verification. Delegate bounded simple work such as documentation, mechanical edits, focused code searches and small independent tests to a less expensive available model.

The owner mentioned Opus 5.5 / extra-high as a preferred helper option. These are preference labels, NOT proof that this model or effort setting exists in Hoplite or costs less. Inspect the actual available model selection and pricing/budget information exposed by the platform. Use that preference only if actually supported and suitable; otherwise use an available lower-cost helper and state the actual model briefly. Never invent a model ID or claim a routing change you cannot control. If model choice cannot be set for subagents, explain the limit and conserve credits with bounded sequential work.

Use at most one helper at a time unless there is a demonstrated need. Give it a narrow file scope and acceptance criteria, avoid overlapping edits and duplicate repository surveys/builds, and review its result in the main session. High/extra-high effort is a preference for difficult helper work where supported; do not spend maximum effort on mechanical tasks unnecessarily. Keep the mandatory commit/push checkpoints after substantial changes and before credits run out.
