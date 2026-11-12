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
| 1 | «Описание» removed from the desktop table and the view panel; saved views migrate | **WIP** — `tbs-core` verified, `tbs-app` tests not yet run (see section 3) |
| 2 | Dictionary: why it did not grow, full operator inventory, web research, verified import | in progress |
| 3 | «Подробности»: recalculated dry-run, protected manifest, administrator procedure | in progress |
| 4 | Ongoing processing: cache invalidation, review of unknown operators | not started |

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
- **not verified yet**: `cargo test -p tbs-app --features devtools` (needs the GTK/WebKit headers
  of `desktop/tools/linux-dev.Dockerfile`; the image was still building when this was written).
  Windows build, installer and a real Windows screen: not run in this session.
