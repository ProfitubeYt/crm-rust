# Dictionary, «Подробности» and «Описание» — progress

Branch `work/dictionary-and-details`, created from `main` at `9559da3` (integrated 2.0.2).
Task specification: `docs/tasks/DICTIONARY_AND_DETAILS_PROMPT.md`.

Status words: **verified** = a check named here ran in this session and passed;
**measured** = an aggregate read from production through the read-only tunnel;
**proposed** = prepared and reviewed here, not written anywhere;
**not verified** = no such check ran. Nothing in this file is a claim that production changed.

## 0. Hard facts about this session

- **No production write happened.** No deployment, migration, restart, dictionary import or
  historical fill. The only credential is `hoplite_ro` (SELECT-only); it was never bypassed.
- No paid AI call. No receipt, card number, phone number or personal name left the sandbox:
  web research used only public organisation / service names.
- Row-level data stays outside git, in a private sandbox directory (mode 700).

## 1. Access used

- Shared read-only key supplied by the owner as a task attachment, installed outside the worktree
  with mode 600; client fingerprint matched `SHA256:WEfc++gQ…DpYE`.
- Server ED25519 key scanned, compared with the verified fingerprint `SHA256:v6EsGV56…22SM`, pinned
  only after it matched; `IdentitiesOnly=yes`, `StrictHostKeyChecking=yes`.
- `pgpass` went straight into a mode-600 file. One tunnel `127.0.0.1:15432 → 127.0.0.1:9990`, session
  forced `default_transaction_read_only=on`, one connection at a time.
- Measured 2026-11-12: PostgreSQL 15.16, schema `20261112_0033`, 1 934 receipts (1 932 at the
  2.0.2 rollout), 268 distinct operator lines, no locked periods.

## 2. Milestones

| # | Milestone | State |
|---|---|---|
| 1 | «Описание» removed from the desktop table and the view panel; saved views migrate | code done, `tbs-core` and `tbs-app` tests **verified** (section 3); Windows not run |
| 2 | Dictionary: why it did not grow, full operator inventory, web research, verified import | **WIP** — code and research file pushed; 17 lines **proposed**, none imported (section 4) |
| 3 | «Подробности»: recalculated dry-run, protected manifest, administrator procedure | **WIP** — manifest and apply tool pushed; 59 fills **proposed**, none applied (section 4) |
| 4 | Ongoing processing: cache invalidation, review of unknown operators | **WIP** — code and tests pushed (section 4) |

## 3. Milestone 1 — «Описание» (WIP)

- `Column::Description` is gone from `tbs-core` (`columns::ALL` has 23 columns), so the table, the
  view panel's column list, the counters, sorting and cell editing cannot offer it. «Подробности»
  (`AnnotationDetails`) is untouched.
- Not removed: the server field (`Transaction.description`), the sync protocol, the descriptions
  API client, the «Описания» reference tab, the workbook / Excel protocol, and the table search
  (it still reads the field). Production has no descriptions at all (0 rows, measured).
- Saved views: a view or named preset that names `description` — in the order, the hidden columns,
  the widths, the column or cell paint, or the sorting — opens without it and keeps everything else.
  A name the table never had is still refused, as before. `LAYOUT` stays 4.
- **verified**: `cargo test -p tbs-core --locked` — 123 unit and 2 integration tests passed
  (Rust stable, Linux).
- **verified**: `cargo test -p tbs-app --features devtools --locked` inside the image built from
  `desktop/tools/linux-dev.Dockerfile` — 306 passed, 0 failed (run 09:24–09:45 UTC on the pushed
  desktop code, commit `3e46811`).
- **not verified**: Windows build, installer and a real Windows screen; Excel add-in on Windows.

## 4. Milestones 2–4 — WIP checkpoint 2 (2026-11-12, about 10:10 UTC)

Pushed as work in progress so that nothing is lost. **Everything below is proposed, nothing was
written to production.** A full write-up with the operator accounting replaces this section.

What is in the commit:

- `backend/data/operator_dictionary/research_20261112.json` — the research registry: 12 verified
  entries (17 dictionary lines), 11 entries for the owner's review, 9 groups that name no
  application, and the review of the 70 existing active lines. Public names and sources only.
- `backend/services/operator_dictionary_service.py`, `backend/scripts/operator_dictionary.py` — the
  read-only import plan (`add`, `covered`, `exists`, conflicts that are never overwritten) and the
  review of operators without an application; `GET /api/reference/review` shows the same review.
- `backend/api/routes/reference.py` — creating, changing, deleting or importing a dictionary line
  now drops the matcher's cache in that process.
- `backend/scripts/build_change_manifest.py`, `backend/scripts/apply_change_manifest.py` — the
  protected change manifest (reads only) and the administrator's verify / apply / rollback tool
  with a per-record journal.
- `backend/services/annotation_rule_service.py`, `backend/workers/celery_worker.py`,
  `.env.example` — `details_plan` (receipt-level dry run) and relearning of the note rules on a
  timer, **off by default** (`ANNOTATION_DETAILS_RULES_REFRESH_HOURS=0`).
- `docs/handoff/ADMIN_EXECUTION_REQUEST_20261112.md` — the request to the administrator.

State of the checks at this checkpoint:

- **verified**: backend suite `pytest -q` — 1242 passed, 1 skipped; `backend/tools` tests — 2 passed.
- **measured / proposed** (production, read-only, 1 938 receipts): 17 dictionary lines to add,
  656 «Приложение» fills, 59 «Подробности» fills, 46 note suggestions that are never applied.
- **not finished**: re-check of every research source against its page, the review of the apply
  tool against the live schema, the operator accounting, the final report.
