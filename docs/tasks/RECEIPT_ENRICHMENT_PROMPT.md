Continue development in https://github.com/johnellgalathejikawe48-boop/fix . Start from its current main, which contains integrated version 2.0.1, code baseline bfff4f62b22521e86364e2f09c142b3cdcb055a4 and this task document. Create branch work/receipt-enrichment. Preserve both previous Excel/backend and Rust UI workstreams. Do not replay old patches or start from an older separate branch.

Goal: improve evidence-based receipt application detection and details enrichment, and align the column order with the original Excel workbook. Investigate before changing rules; reuse existing parsers, dictionaries, operator mappings, application_backfill and annotation mechanisms.

Another independent agent owns the startup hub, authentication navigation, burger-menu wording and column separator rendering. Do not change those areas. Work primarily in backend/ and tests. Put necessary desktop column-order changes in a small separate commit for integration. Do not modify rendering, authentication or hub code. If integration touches the other agent's files, document the exact proposed change instead of overlapping edits.

1. Application detection

Trace raw receipt -> parser -> operator normalization -> dictionaries/mappings -> database -> API -> desktop and Excel. Distinguish data lost in processing from information absent in the source.

Expected normalization examples: "Окта Яндекс Плюс оплата" -> "Яндекс"; Payme -> Payme; Uzum Market -> Uzum. Validate against actual formats. A merchant or purchased service does not necessarily identify the payment application. Define the meaning of Приложение explicitly; do not confuse it with Сервис (receipt source) or assert which payment app was used without evidence.

Use explicit receipt evidence first, confirmed dictionary/rule mappings second, verified similar-operation mappings third, and optional AI verification only when justified. Do not invent values to eliminate empty cells. Leave insufficient-evidence cases empty or offer reviewable suggestions. Preserve provenance and confidence.

2. Details enrichment

Investigate existing Подробности values and stable matching evidence: operator, application, operation text, identifiers and labels such as DIK 0626. Establish their meaning before using them. Account for meaningful differences between superficially similar receipts.

Reuse confirmed mappings cautiously. Never overwrite manual/confirmed values or violate locked periods and permissions. Prevent one false match from propagating and never learn from the system's own unverified suggestions. Make repeated processing idempotent.

3. Optional AI

The provider and budget are NOT selected. Support configurable OpenAI-compatible base_url, API key and model; DeepSeek is a candidate, not a dependency. Measure what rules solve without AI first. Do not activate paid API calls or transmit real receipts to external providers without separate authorization.

If useful, implement an optional adapter with minimal redacted context, strict structured-output validation, an insufficient-evidence outcome, caching, request/token/cost/retry limits, timeouts and graceful failure. Reuse the existing configurable AI plumbing. Never send card numbers or unnecessary personal data. Preserve existing working AI configuration.

4. Column order

Required sequence through Подробности:
1) Date
2) A second date, intentionally a separate column
3) Сервис: receipt source, CardXabar / HUMO Card
4) Операция: payment, account change, etc.
5) Оператор
6) Приложение
7) Сумма
8) Остаток
9) ПК: final four digits of the card, kept as a string including leading zeros
10) P2P: 1 or empty
11) Подробности

Determine the exact names, formats and meanings of BOTH dates from the original workbook/existing export. Do not assume they are duplicates or invent a second date. If evidence is unavailable, ask one focused question and continue independent work.

This is a sequence through Подробности, not permission to delete other columns. Preserve remaining columns and customized layouts. Keep Операция separate from Тип. Check the corresponding desktop table and Excel export. Missing recipient-card information must remain unknown; having all columns does not mean every cell must be filled. Do not rewrite the new View/filter features.

5. Production investigation: READ ONLY

Host 64.188.106.221 / 377344.senko.network, SSH port 22, user hoplite-ro. API https://64.188.106.221.nip.io . Application directory /opt/receipt-parser is context only: this account has no ordinary shell or filesystem browsing.

Shared client-key fingerprint: SHA256:WEfc++gQxOhnrZCuD+xOfnaBGm0T6djPq8FmHZyDpYE . Verified server ED25519 fingerprint: SHA256:v6EsGV568CnEfD2uUH7tu3pZuXpgJbwjbR9HiXU22SM . Public key is installed, but its private half must be securely mounted outside git in YOUR sandbox. HOPLITE_SSH_KEY_PATH must point to that actual file. The owner's C:/Users/heper/.ssh/hoplite-shared-ro-ed25519 is not automatically available in Linux. Do not request private-key contents in chat, commit them or generate a replacement automatically. If missing, report the provisioning requirement and continue local work.

Verify client fingerprint, chmod 600, and pin the verified server key. Use IdentitiesOnly=yes and StrictHostKeyChecking=yes. Retrieve credentials privately with:
ssh -i "$HOPLITE_SSH_KEY_PATH" -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes hoplite-ro@64.188.106.221 pgpass

The output is a password-file entry: capture it into protected memory/file without displaying it in tool logs or chat. Open the permitted tunnel:
ssh -N -i "$HOPLITE_SSH_KEY_PATH" -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o ExitOnForwardFailure=yes -L 127.0.0.1:15432:127.0.0.1:9990 hoplite-ro@64.188.106.221

DB role hoplite_ro, use retrieved database/password at local port 15432; adapt a private password-file port accordingly. SELECT-only approved tables/columns; statement timeout 30s; limit 4 connections shared across sessions, use one. Key and role expire 2026-11-25 00:00 UTC. Shell/sudo/Docker/reverse tunnels and access to password hashes/salts, TOTP, sessions and Excel tokens are intentionally denied. Do not bypass or broaden restrictions.

Prefer aggregate analysis of field completeness, operator/application distributions and mappings. Inspect only small necessary samples within the secure session, never copy real receipts or personal/payment data into commits, screenshots or reports. No production writes, backfills, migrations or deployment without separate explicit authorization. All write-based tests must use isolated local data.

6. Verification and delivery

Write a short evidence-based plan and maintain docs/handoff/ENRICHMENT_PROGRESS.md. Do not overwrite another agent's progress or global plan. Test definite/ambiguous matches, similar-operator false positives, manual values, locked periods, idempotence, DB/API/table/Excel consistency and operation without AI or during AI errors.

Implement historical processing as a dry-run first: aggregate proposed-change counts, unresolved cases and anonymized reasons. Do not apply production changes automatically. Report missing reference workbook or unavailable live access honestly; neither prevents local development.

After each substantial milestone, review the diff, run focused checks, update progress, commit ALL intended modified AND new files, PUSH and verify the remote SHA. Use git status --untracked-files=all; git add -u previously omitted new files. Do not wait until the end. If credits are low, push a clearly labelled WIP with failing/unrun checks and remaining tasks. No secrets, customer workbooks, force push or destructive cleanup.

Keep the owner's main Fable 5.2 model for complex decisions. Use at most one cheaper available helper for narrow mechanical work when useful. Opus 5.5 / extra-high is a preference only if actually available and appropriate; never invent a model ID or assume it is cheaper. Avoid duplicate surveys, overlapping edits and multiple simultaneous builds.

Final report: causes of empty fields, rules implemented, exact tests/dry-run results, unresolved ambiguities, provider/budget decisions needed, branch and verified pushed SHA. Include precise integration notes for the separate hub/UI agent. Do not deploy automatically.
