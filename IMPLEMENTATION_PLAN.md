# Implementation plan — Rust client data, Excel sync, filters, table view

Legend: `- [ ]` ☐ not started · `- [x]` ☑ implemented **and** verified. A task is ticked only after its test or check listed under "Verification" has actually passed.

Branch: `hoplite/barke-6b3e18a3` (third session; continues `hoplite/kyzikos-dda212ec` and the interrupted snapshot `b9d3020`) · Started: 2026-11-11

## 0. Continuation after the interrupted second session

The second session ran out of credits. Its commit `b9d3020` held only tracked modifications: new files were lost and the snapshot did not build (`docs/handoff/RECOVERY_STATUS.md`). This session restores them against the preserved call sites and tests, then finishes P2–P4. Nothing from the old sessions is re-applied or rewritten.

| Lost or missing | State |
|---|---|
| `tbs-core/src/operation.rs` | Recovered from the transcript before this session (`d56409d`). |
| `transactions/facets.rs`, `transactions/fixtures/*`, the wiring `filters.rs` expects in `transactions/mod.rs` and `menus.rs` | Restored in `1c8ddb5`; verification in §5. |
| `backend/services/excel_workbook_builder.py`, migration `20261111_0032`, `backend/tests/test_excel_workbook_{api,builder}.py` | Restored under P2.1; state in §3 and §5. |

Rules of this session:

- Rust runs only in a container built from `desktop/tools/linux-dev.Dockerfile` (base `rust:1.99.1-bookworm` pinned by digest; approved by the owner on 2026-11-11). Nothing is installed on the host.
- A milestone is verified on a clean checkout of its commit and then pushed. Work that is not verified is pushed only under a commit that says WIP; `hoplite/barke-6b3e18a3-wip` holds a safety snapshot taken mid-edit and is not a milestone.

## 1. What was studied

| Area | Where | Role |
|---|---|---|
| Backend | `backend/` (FastAPI, SQLAlchemy, PostgreSQL, Alembic head `20260923_0031`) | API, receipt parsing, Excel sync protocol |
| Electron client (old) | `frontend/` (React, TanStack Table) | Reads `GET /api/transactions/` page by page, server-side filters |
| Rust client (new) | `desktop/crates/tbs-core` (API, cache, query engine), `desktop/crates/tbs-app` (iced UI) | Downloads all rows once, keeps a local SQLite snapshot, filters and sorts locally |
| Excel sync | `backend/api/routes/excel_sync.py`, `backend/tools/excel_sync/*.bas`, `backend/assets/excel_sync_template.xlsm` | Macro workbook: password activation → device grant → snapshot/sync every 60 s |

Evidence used besides the code:

- **Production, read-only** (tunnel-only SSH account + `hoplite_ro` PostgreSQL role, SELECT only, no secrets columns). Aggregates only, no personal data was copied.
- **The customer's reference workbook** «Чеки (с паролем).xlsm» (kept outside git).

## 2. Findings and root causes

### 2.1 Priority 1 — table shows wrong / empty data

The field mapping itself is correct: `tbs-core/src/model.rs` uses the same JSON keys as `TransactionResponse`, and `columns.rs`/`query.rs`/`grid` address cells by column, not by position. Electron reads the very same endpoint. The differences are elsewhere:

**RC-1 · The ledger columns are rendered as storage codes, not as the text the workbook shows** (`tbs-core/src/columns.rs`).

| Column | Excel / Electron | Rust client before the fix |
|---|---|---|
| «Статья» | `р1=ФЗП,ФМП,МП` (`code=label`) | `р1` |
| «Период» | month (`07.2026`) | `2026-07-01` |
| «Лицо» | `code — ФИО` when the dictionary has a name | code only |
| «Тип» | server's `transaction_type_display` (incl. «Изменение счета») | local label of the code |

**RC-2 · The default layout puts four almost empty columns where the ledger columns belong** (`tbs-app/src/screens/transactions/table.rs`). Production fill rates (1 931 receipts): «Приложение» 278 (14 %), «Описание» **0** (the merchant-description registry has 0 rows), «Получатель» 108 (6 %), «Карта получателя» 119 (6 %) — against «Подробности» 1 178, «Лицо» 1 253, «Период» 1 069, «Статья» 1 226. The default order shows exactly those four sparse columns right after «Тип» and pushes the filled ledger columns off-screen, so the table reads as "not loaded". The workbook order is: Дата · Сервис · Операция · Оператор · Приложение · Сумма · Остаток · ПК · p2p · Подробности · Лицо · Период · Статья · Валюта · Получатель · Карта получателя (no «Описание» at all — the notes are «Подробности»).

**RC-3 · The local snapshot can stay stale or incomplete** (`tbs-core/src/sync.rs`, `store.rs`). The snapshot is refetched only when `checksum | row_count | max(updated_at)` of `/api/sync/manifest` changes. That fingerprint (a) hashes only the newest 100 rows, (b) is cached by the server for 30 s + 300 s and never invalidated by writes, (c) cannot see anything resolved at read time (descriptions, chat titles). The cache also stores a re-serialisation of the client's own struct under a schema version that never changes, so rows saved by a build that did not know a field keep it empty until the fingerprint happens to move. Electron had none of this: it asked the server for every page.

**RC-4 · Smaller defects found on the way**

- A locked scope answers `has_more: null`; `TransactionPage.has_more: bool` fails to decode, so the sync reports an error instead of an empty table.
- Offset paging over a changing table can repeat a row on the next page; the snapshot kept both copies.
- Free-text search ignored «Подробности», receiver and description.

### 2.2 Priority 2 — downloaded workbook stays empty

**RC-5 · `GET /api/excel/workbook` returns a static, unconfigured template.** In its hidden `_sync` sheet `B1` (workbook id), `B5` (server URL) and `B7` (owner) are empty. `ExcelSync.StartSync` then generates an id, `ConfigureSharedProfile` exits (`Len(url) + Len(owner) = 0`), no timer is scheduled and **no HTTP request is ever made**. The working book «Чеки (с паролем).xlsm» has `B1`=UUID, `B5`=`https://64.188.106.221.nip.io`, `B7`=`1`, `B8`=`1` — filled by hand by an administrator; no API does it.

**RC-6 · The panel sends the user to a button that does not exist.** «В книге нажмите «Подключить»…» — the workbook has no such control; the token can only be entered through the `Connect` macro (Alt+F8). Production shows the result: grant «Моя книга Excel» created 2026-11-11, never bound to a workbook, never used.

**RC-7 · The password cannot start a connection.** `PUT /api/excel/activations` answers 404 until a grant is already bound to that workbook id, so "set a password" is only reachable after a manual connect.

Same flow in Electron (`ExcelSyncPanel.tsx`), so this is not a Rust regression; the "fully automatic" behaviour only ever existed for the hand-prepared file.

### 2.3 Priority 3 / 4 — filters and «Вид»

- Filters: one long 520 px sheet of eight always-open sections; nothing shows what is active once it is closed; a filter can only be removed by reopening the sheet; dates are typed with no presets; no feedback on how many rows a choice leaves until «Применить».
- «Вид»: a 320 px popover with a flat list of 22 checkboxes; columns cannot be reordered or searched there; no way back to the default order or widths without also wiping filters; density is a column of chips; page size lives elsewhere.

## 3. Tasks

### P1 — Critical: correct data in the table

- [x] **P1.1 Ledger text for annotation columns** (`tbs-core`)
  - `Labels` (person and article dictionaries) owned by `Dataset`; cells for «Лицо»/«Статья» rebuilt when the dictionaries arrive; «Период» as `MM.YYYY`; «Тип» prefers `transaction_type_display`.
  - Selected-rows export and copy keep working (`edit_text` still yields the raw code for paste).
  - *Acceptance:* a row with `annotation_article_code = "р1"` and label «ФЗП,ФМП,МП» shows `р1=ФЗП,ФМП,МП`; period `2026-07-01` shows `07.2026`; unit tests cover both and the fallback to the bare code.
- [x] **P1.2 Default layout mirrors the workbook** (`tbs-app/.../table.rs`)
  - New default order; «Описание» joins the columns hidden by default; «Источник» and «Валюта» visible.
  - Saved layouts that are still the untouched old default migrate once (`layout` version field); customised layouts are left alone.
  - *Acceptance:* fresh table shows … Приложение · Сумма · Остаток · ПК · П2П · Подробности · Лицо · Период · Статья …; test for the migration both ways.
- [x] **P1.3 Sound synchronisation** (`tbs-core/src/sync.rs`, `store.rs`, `tbs-app/.../transactions/mod.rs`)
  - Two modes: *quick* (manifest fingerprint, as before) and *verify* (download, compare a SHA-256 digest of the rows with the stored one). Verify runs at start-up, on «Обновить», when the table is opened again, and every 5 minutes; quick runs every minute.
  - Cache stamp derived from the model's field list, so a build with a new field never trusts an older cache.
  - Page walk drops repeated ids; `has_more: null` decodes.
  - *Acceptance:* tests with a stub server: an edit invisible to the manifest reaches the table on the next verify; an unchanged server costs no dataset rebuild; a cache written under another stamp is discarded.
- [x] **P1.4 Search covers the ledger fields** (`tbs-core/src/query.rs`) — «Подробности», receiver, receiver card, description and the article label are searchable.
- [x] **P1.5 Verify against production data** — compare what the client would render for the reference workbook's rows with the workbook itself (same ids). Result in §5.
- [x] **P1.6 «Операция» as a column of its own** (`tbs-core/src/operation.rs`, `columns.rs`, `query.rs`, `export.rs`) — the workbook's `OperationDisplay` rule (the receipt's own headline, else a fallback from type and bank) next to «Тип», which keeps the server's wording. Source recovered from the transcript with its 16 unit tests.
  - *Acceptance:* the recovered tests pass on the restored tree; «Тип» and «Операция» stay two columns in the layout, the search, the sort and the export. (Copying the cell has no test of its own.)
- [ ] **P1.7 Inputs of «Операция»** (`backend/api/routes/transactions.py`, `tbs-core/src/model.rs`, `operation.rs`, `query.rs`)
  - The API sends an inferred type where the workbook uses the stored one; it does not send `excel_bank`; the server export ignores `sort_by=operation` and silently sorts by date.
  - Server: `stored_transaction_type` and `excel_bank` in `TransactionResponse`, `sort_by=operation` honoured by the export. Client: the fallback reads the stored type when the server sends it and keeps working with a server that does not. An absent field stays absent; nothing is filled from another field.
  - *Acceptance:* backend and core tests for the three gaps; how many production rows the difference touches, read-only.

### P2 — High: Excel workbook that configures itself

- [ ] **P2.1 Backend: create a ready workbook** (`backend/api/routes/excel_sync.py`, new `services/excel_workbook_builder.py`, migration `0032`)
  - `POST /api/excel/workbooks {name, password}` → new workbook id, `excel_sync_activations` row (password hash, owner, permissions, scope, name), template patched in `_sync` (`B1` id, `B5` public server URL, `B7` owner), file returned. No token is ever written into the file.
  - `GET /api/excel/workbooks/{id}/file` → the same configured file again (for another computer).
  - `PUT /api/excel/activations` also accepts a workbook the user created but nobody activated yet.
  - Public URL: `EXCEL_SYNC_PUBLIC_URL` or the request's own scheme/host; refused unless it satisfies the workbook's `ValidServerUrl` rule.
  - *Acceptance (pytest):* the returned file is a valid xlsm whose only changed part is the `_sync` sheet and whose `vbaProject.bin` is byte-identical; replaying the workbook's HTTP calls (`/v1/activate` with the password → `/v1/snapshot`) returns the user's receipts; wrong password is refused; another user cannot download or re-password it.
- [ ] **P2.2 Rust client: one-step flow** (`tbs-core/src/api/excel.rs`, `tbs-app/.../transactions/excel.rs`)
  - «Новая книга»: name + password → server builds the file → save dialog → three plain instructions (open, enable content, type the password).
  - Per workbook: computers connected, change password, download again, disconnect a computer.
  - Manual token flow removed from the panel; old connections stay listed and revocable.
  - Server without the endpoint (not yet deployed) → clear message instead of a broken file.
  - *Acceptance:* unit tests for the panel logic; live test (`live_writes`) creates a workbook against a local backend and activates it.
- [ ] **P2.3 Documentation** — `docs/EXCEL_SYNC.md`: new flow, deployment step (`alembic upgrade head`, optional `EXCEL_SYNC_PUBLIC_URL`).

### P3 — Filters redesign (`tbs-app/.../transactions/filters.rs`, `mod.rs`, `ui.rs`, `theme.rs`)

- [ ] **P3.1 Active filters bar** above the table: one removable chip per active filter, «Сбросить всё».
- [ ] **P3.2 Sheet rebuilt**: collapsible groups with a one-line summary and a per-group clear; period presets (сегодня, вчера, 7/30 дней, этот/прошлый месяц, год); searchable multi-selects with the chosen values on top; live counter «Найдено N из M» while editing; inline validation next to the field.
- [ ] **P3.3 Behaviour**: «Применить», «Сбросить», Esc; the draft survives validation errors; the stored filter format is unchanged (saved views keep working).
- *Acceptance:* unit tests for chip summaries, per-filter removal, presets and the draft round trip; existing filter tests still pass; screenshots of the sheet and the bar.

### P4 — «Вид» redesign (`tbs-app/.../transactions/view.rs` new, `menus.rs`, `table.rs`)

- [ ] **P4.1 Side sheet «Вид таблицы»** with three tabs: Столбцы · Отображение · Сохранённые виды.
- [ ] **P4.2 Columns tab**: search, show/hide, move up/down, pin marker, «Показать все», «Как в книге Excel», «Сбросить ширину», counter «Показано N из 22».
- [ ] **P4.3 Display tab**: density, rows per page, pinned columns, newest first.
- [ ] **P4.4 Saved views tab**: save, apply, rename, default, delete; «Сбросить вид» resets layout only (filters have their own reset).
- *Acceptance:* unit tests for move/reset/ledger order; screenshots of the three tabs.

### Cross-cutting

- [ ] **X.1** `cargo test -p tbs-core` and `cargo test -p tbs-app --features devtools` green.
- [ ] **X.2** Backend `pytest` green for the touched areas (Excel sync API/service/VBA package, sync API).
- [ ] **X.3** No secrets, keys or customer files in the commit; plan updated; commit and push.

## 4. Order and dependencies

1. P1.1 → P1.2 → P1.3 → P1.4 (core first, then the screens that read it).
2. P2.1 (server) → P2.2 (client needs the endpoint) → P2.3.
3. P3 and P4 touch the same screen module; P3 first, P4 reuses its sheet and chip helpers.
4. P1.5 needs the production tunnel; everything else is local.

## 5. Verification log

### P1 — table data (2026-11-11)

- `cargo test -p tbs-core`: 86 unit tests pass (ledger labels and their fallback, `MM.YYYY`, search over the ledger fields, quick/verify sync against a stub server, cache stamp, duplicate ids, `has_more: null`); the two live suites skip without a local server.
- `cargo test -p tbs-app --features devtools`: 187 tests pass (fresh layout, one-time migration of an untouched saved layout, a hand-arranged layout left alone).
- The suites run on Linux now (the product still targets Windows): `rfd` gets a dialog backend off Windows, the WebView2-only switches are gated, and one test no longer assumes `\` separates paths.
- **P1.5, production against the reference workbook.** All 1 637 receipts of the workbook still exist in production (1 931 receipts now). For each of them the cell the client renders was compared with the workbook's cell:

  | Column | Workbook rows filled | Identical |
  |---|---|---|
  | Дата_Время, Оператор/Продавец, Сумма, Валюта | 1 637 | 1 637 / 1 637 |
  | Остаток | 1 501 | 1 637 / 1 637 |
  | ПК | 1 620 | 1 637 / 1 637 |
  | Приложение | 255 | 1 637 / 1 637 |
  | Подробности | 1 178 | 1 637 / 1 637 |
  | Лицо | 1 253 | 1 637 / 1 637 |
  | Период (`MM.YYYY`) | 1 069 | 1 637 / 1 637 |
  | Статья (`code=label`) | 1 226 | 1 637 / 1 637 |
  | Получатель | 108 | 1 637 / 1 637 |
  | Карта получателя | 118 | 1 637 / 1 637 |
  | p2p | 304 set | same flag in 1 637 / 1 637 (the workbook writes `0`, the client leaves the cell empty) |

  Method: the workbook was read with openpyxl, production with one read-only `SELECT` per table through the tunnel, and the client's rules (`Column::display`) were applied to the production values; the rules themselves are covered by the unit tests above. So the server holds exactly what the ledger shows, and the columns that looked empty («Приложение», «Получатель», «Карта получателя») are empty in the ledger too — 84–93 % of the receipts simply have no such value. What was wrong in the client was the presentation (RC-1, RC-2) and the freshness of its snapshot (RC-3).
- **Known difference, left as is.** The workbook's «Операция» is the receipt's own first line («💸 Оплата», «ℹ️ Счет по карте изменен»), which the macro takes from the raw text. Both clients show the server's wording of the type instead («Списание», «Изменение счета»); the Rust client now takes that wording from the server like Electron does. The client's label occurs in the workbook's headline in 206 of 1 637 rows; elsewhere the headline is another text. The two columns are not the same thing and were not reconciled.

### Session 3 — the build restored (2026-11-11)

Commit `1c8ddb5`, run on a clean checkout of that commit in the container (rustc 1.99.1), not on a working tree that was still being edited:

- `cargo test -p tbs-core --locked`: 108 unit tests pass, 16 of them the recovered `operation` tests; `the_operation_is_the_receipts_own_line_and_is_not_edited`, `the_operation_is_searched_and_sorted_as_shown` and `the_operation_goes_out_as_the_text_of_its_cell` cover the column, the search and sort, and the export. `live_backend` and `live_writes` each report one passing test but return at once without `TBS_E2E_*`: they are no coverage.
- `cargo test -p tbs-app --features devtools --locked`: 235 pass, 0 fail, the layout migration tests among them. The live UI suite and the screenshot test also return at once without their environment.
- One preserved test was stale and was corrected without dropping an assertion: `the_counter_says_when_the_draft_cannot_be_counted` typed into a field of a folded group. The reason is in the commit message.
- Not covered by this milestone: the Windows target, a real window, the live suites, anything in the backend.

## 6. Deployment notes

_Filled in as tasks complete._

## 7. Production access used for this work

- OS account `hoplite-ro`: key-only, forced command, port forwarding to `127.0.0.1:9990` only.
- PostgreSQL role `hoplite_ro`: SELECT on the transaction, reference and annotation tables and on non-secret columns of `users` / `excel_sync_*`; expires after 14 days.
- Revoke: `userdel -r hoplite-ro; rm -f /usr/local/bin/hoplite-ro-gate` and `DROP OWNED BY hoplite_ro; DROP ROLE hoplite_ro;`.
- Nothing was written to production.
- Third session: keys of earlier sessions do not carry over. A new ED25519 session key (fingerprint `SHA256:N37nlcLOzWtgi3XXfwmyNslah+9kJSkMuz0Sie0Eqtw`, private part outside the repository) was handed to the owner with one `authorized_keys` line: `restrict,port-forwarding,permitopen="127.0.0.1:9990",command="/usr/local/bin/hoplite-ro-gate",expiry-time="20261125"`. The server's host key was pinned after comparing it with the verified fingerprint. Whether the key was installed, and what was read through it, is recorded in §5.

## 8. Status

_In progress._ Recovery of the build: done (§0). P2 backend, P3 filters: in work. P4, P1.7: not started.


## Production integration — 12 November 2026

Backend b6419c6 and Rust UI 83f5bdd were merged without conflicts at b5bbc0d. Backend deployed with migration 0032 after backups. Native 2.0.1 release and private installer built. Fresh Windows checks: 116 core and 264 app tests pass. Fresh isolated backend checks: 1092 pass, 1 optional oletools check skipped. See docs/handoff/DEPLOYMENT_20261112.md for backup paths, rollback, live smoke checks and limitations. Earlier unfinished-state statements above describe previous sessions; real Excel execution and interactive Windows validation remain unverified.
