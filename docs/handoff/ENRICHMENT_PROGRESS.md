# Receipt enrichment progress — «Приложение», «Подробности», column order

Branch `work/receipt-enrichment`, created from `main` at `031d2e4` (integrated 2.0.1).
Owner of this file: the enrichment session. `BACKEND_PROGRESS.md`, `UI_PROGRESS.md`,
`IMPLEMENTATION_PLAN.md` and the hub / authentication / menu / separator code belong to other
sessions and are not modified here.

Status words: **verified** = a check named in this file ran in this session and passed;
**measured** = an aggregate read from production through the read-only tunnel;
**not verified** = no such check ran.

No production write, backfill, migration or deployment happened. No paid AI call was made and
no receipt left the sandbox.

## 1. Access used

- Shared read-only key supplied as a task attachment, installed outside the worktree with mode
  600; client fingerprint `SHA256:WEfc++gQ…DpYE` matched. The server ED25519 key was scanned,
  compared with the verified fingerprint `SHA256:v6EsGV56…22SM` and pinned only after it matched;
  every connection uses `IdentitiesOnly=yes` and `StrictHostKeyChecking=yes`.
- `pgpass` went straight into a mode-600 file, never to a log. One tunnel
  `127.0.0.1:15432 → 127.0.0.1:9990`, role `hoplite_ro`, session forced read-only, one short
  connection at a time. PostgreSQL 15.16, `statement_timeout` 30 s.
- Only aggregates and short dictionary-level samples were read. Nothing row-level is stored in
  this repository: no receipt text, card numbers, names, phone numbers or details values.

## 2. What production looks like (measured 2026-11-11)

| Figure | Value |
|---|---|
| Receipts | 1 932 (2026-01 … 2026-11, two from 2022) |
| «Приложение» empty | 1 654 (85.6 %) — every one of them has an operator |
| «Подробности» empty | 732 (37.9 %) |
| Dictionary `operator_reference` | 165 rows, **70 active**, 95 inactive |
| `operator_mappings` (older table) | 0 rows |
| Locked periods | none active |
| «Подробности» provenance | import 1 066, agent 106, rule 28, manual 0 |
| `annotation_rules` | details 14, person 57, article 44 (computed 2026-09-01) |

Empty «Приложение» by parser: CardXabar 1 218 of 1 398, HUMO 334 of 393, transfer 62 of 81,
vision 18 of 34, AI text 16 of 19, SMS 6 of 7. The loss is not a parsing failure: the operator is
present in all of them.

## 3. Why «Приложение» is empty

Trace: receipt text → `RegexParser` / AI parser (`operator_raw`) → `ParserOrchestrator._finalize`
→ `OperatorMapper.map_operator_details` over active `operator_reference` rows → worker stores
`transactions.application_mapped` → API / desktop / Excel read that column unchanged. The value
is decided once, at parse time, and nothing downstream drops it. Causes, in order of weight:

1. **The dictionary is the owner's 70-row lookup table and nothing else.** Rows 1–70 are the
   seed of `операторыпродавцыиприложения-2.txt` (P2P gateways). The other 95 rows are AI
   suggestions the worker stores with `is_active = false`; none was ever reviewed, and several are
   wrong (a telecom operator filed under a payment app, a card system under an app). They must not
   be trusted as they are.
2. **The matcher needs the dictionary string inside the receipt string, token for token.** Real
   receipts differ from the dictionary in ways that carry no meaning:
   - the trailing country code: the dictionary has `…, 99`, CardXabar now prints `…, UZ`;
   - HUMO cuts the operator at 20 characters, so the dictionary's longer spelling never fits;
   - a city after `>` (`…P2P>TASHKENT`).
3. **Operators that name an application outright are not in the dictionary at all** — payment
   and service brands paid through their own gateway (`PAYME OPLATA`, `XAZNA PAYMENTS`,
   `BEEPUL OPLATA`, `AO PAYNET MOBILE`, the Yandex Plus and Uzum Market descriptors).
4. **AI mapping is off** (`AI_APPLICATION_MAPPING_ENABLED=false`) and was on only for a while:
   the same operator string carries an application on some receipts and none on others
   (for example one gateway: 11 filled, 70 empty). That is data lost in processing.
5. **The backfill was not re-run**, and with the strict matcher it would find little.
6. **Information that is not in the source.** Shops, pharmacies, individual entrepreneurs, fuel
   stations, ATMs and the 63 receipts whose operator is `UNKNOWN` name no application. These stay
   empty by design.

`application_mapped` has no provenance column, so a value typed by a person cannot be told from
one the AI guessed. Some stored values are AI guesses that contradict the owner's examples.

## 4. What «Приложение» means

Evidence: the header of the owner's reference list reads «оператор/продавец — приложение, которое
относится к оператор/продавец»; the workbook fills the column by lookup from the operator; the
task examples map a service descriptor to its brand (Яндекс Плюс → Яндекс, Uzum Market → Uzum).

**«Приложение» is the application or online service that the receipt's own operator line
identifies** — the payment app whose gateway processed the operation, or the online service that
was paid. It is a statement about the operator descriptor. It is not «Сервис» (the notification
source, CardXabar / HUMO Card), and it does not assert which app was open on the cardholder's
phone. An ordinary merchant, terminal, ATM or bank office names no application and stays empty.

## 5. «Подробности»: what the evidence supports

Measured on the 1 066 workbook-confirmed values (import), replayed in date order so that each
receipt is predicted only from earlier confirmed ones:

| Key and thresholds | Predictions | Right | Precision | Empty rows it would fill |
|---|---|---|---|---|
| operator; support 3, purity 0.9 (today's rule) | 81 | 68 | 0.840 | 0 |
| operator + ПК + type; support 3, purity 0.9, last 3 agree | 97 | 87 | 0.897 | 27 |
| operator + ПК + type; support 5, purity 0.9, last 3 agree | 68 | 65 | 0.956 | 23 |
| the same, then operator + type, then operator | 70 | 66 | 0.943 | 23 |

- 392 distinct values among 1 064 comparable rows; inside one operator + ПК + type the next
  confirmed value differs from the previous one more often than not. Most notes (phone numbers,
  purposes, names) are simply not in the receipt.
- Four-digit identifiers at the end of a note are the last four digits of one of the owner's own
  cards (112 of 121 such notes): the note records the counterpart card of an internal transfer.
  The receipt itself does not contain that card, so it works as part of a confirmed pattern, not
  as something to generate.
- «ДиК» is an accountable-person code (28 receipts). A number such as `0626` after it was not
  found in any receipt field that was queried. What the number in the task's example «DIK 0626»
  stands for is **not established**: asked on 2026-11-11, the owner chose "something else" than
  the readings offered, and the follow-up question about its meaning has no answer yet. Until it
  has one, the number is not interpreted, split or generated: a confirmed note is reused whole,
  and only when the matching evidence is sufficient.
- The 106 agent-written and 28 rule-written values are the system's own output and are never
  used as training evidence.

## 6. Column order: the two dates

The customer's workbook (the pruned copy in `backend/services/excel_client_template.xlsx`, both
sheets) heads the assembled block: `№`, `1`, **`Дата_Время`**, **`(Без дубля)`**, `Сервис`,
`Операция`, `Оператор/Продавец`, `Приложение`, `Сумма`, `Остаток`, `ПК`, `p2p`, `Подробности`.

- `Дата_Время` — the operation's own date and time from the receipt.
- `(Без дубля)` — the same timestamp made unique: the n-th receipt with an identical timestamp
  on a sheet gets `+ n × 0.0001 day` (8.64 s). It is how the workbook tells same-minute receipts
  apart; it is not a second clock and never a different date.
- Format: both are Excel date-time cells with one number format, `dd/mm/yy hh:mm` (one template
  row of the CardXabar sheet has `dd/mm/yy h:mm`). At that format the two columns read alike on
  the sheet; the shift is in the value, not in what is shown.

The workbook export already follows this, and it is not changed by this work: the owner
clarified on 2026-11-11 that item 4 is about the desktop table only, with the workbook as the
reference for the order. Before this work the desktop table had no `(Без дубля)` column, and
«Тип» stood between «Операция» and «Оператор» (section 10).

## 7. Plan

1. **Application detection** (backend): suffix- and truncation-tolerant dictionary matching;
   explicit brand evidence for the confirmed application vocabulary and the owner's examples;
   conflicts and merchant-shaped operators left empty; provenance and confidence stored;
   manual values never replaced; verified-similar tier fed only by human values.
2. **Historical dry-run** for applications and details: aggregate proposed changes, unresolved
   counts and anonymised reasons; writing only with an explicit flag; idempotent.
3. **Details**: operator + ПК + type key with recency agreement, thresholds from the table
   above; weaker evidence reported as suggestions, not written; rule-written values re-derived.
4. **Column order**: the desktop table, in a separate commit. No export or workbook change.
5. **Optional AI verification**: decided after the rule coverage is measured; adapter only,
   disabled by default, no calls.

## 8. Milestone 2 — application detection (2026-11-11)

Code: `backend/services/application_detection.py` (new), `backend/parsers/operator_mapper.py`,
`backend/services/application_backfill.py`, `backend/scripts/backfill_applications.py`,
revision `20261112_0033` (`application_src`, `application_confidence`).

Evidence tiers, first answer wins:

| Tier | Source stamp | Trust | What it is |
|---|---|---|---|
| dictionary exact | `dict` | 1.0 | the owner's line, whole |
| dictionary variant | `dict` | 0.95 | same name, other country code or city |
| dictionary truncated | `dict` | 0.9 | same name, one side cut at 20 characters |
| brand in operator | `rule` | 0.9 | the line names an application (`BRAND_RULES`) |
| dictionary substring | `dict` | 0.85 | the historical rule, unchanged |
| confirmed similar | `similar` | 0.8 | ≥ 2 hand-set receipts of the operator, unanimous |
| optional AI | `ai` | model's | off by default; see below |

- Every brand rule's application is in the owner's dictionary or is one of the owner's examples;
  each pattern is that application's name or a stem shared by several of the owner's lines.
  A name that is also an everyday word (Hamkor, Asaka) counts only beside a payment-channel word.
- Two tiers that disagree, or two applications in one line, give `ambiguous`: nothing is written,
  both candidates are reported. Cash machines and receipts without an operator line never get one.
- Provenance: `manual` (API edit, bulk edit, Excel sync, value typed at creation), `dict`, `rule`,
  `similar`, `ai`; NULL = unknown origin. Changing `application_mapped` anywhere clears the stamp
  (model listener), so unstamped writers leave "unknown", which is protected. A person's value,
  a person's emptied cell and unknown-origin values are never replaced — also not by a forced
  re-parse, which used to overwrite them.
- Only `manual` values feed the confirmed-similar tier; where people consistently set another
  application than the rules, the detector stands down for that operator.
- The optional AI fallback (`AI_APPLICATION_MAPPING_ENABLED`, unchanged switch and provider
  settings) is now asked only when there is no evidence at all, is shown the operator line and
  the confirmed vocabulary and nothing else (no receipt text), never sees a line with an address
  or a person's name, and may only answer with a confirmed application; any other name becomes an
  inactive dictionary line for review, as before. A provider error leaves the receipt intact.

Dry run on production through the read-only tunnel (`scripts.backfill_applications`, nothing
written; measured):

| | Receipts |
|---|---|
| looked at | 1 932 |
| already with an application | 278 |
| without an application | 1 654 |
| **would be filled** | **457** (brand in operator 386, dictionary variant 71) |
| left empty | 1 197: no evidence 1 104, no operator line 76, cash or terminal 15, ambiguous 2 |

By application: Xazna 91, UzumBank 77, Paynet 65, Payme 57, Milliy 2.0 50, Uzum 41, Beepul 26,
Mavrid 21, Яндекс 13, Hamkor 7, Humans 4, Davr Mobile 4, Chakanapay 1. Empty would fall from
85.6 % to 62.0 %. The matched operator names were reviewed in the session: every one is a gateway
or brand descriptor, none a shop. For review, not changed: 4 stored `UzumBank` where the evidence
says `Uzum`; 2 ambiguous `Paynet | Xazna`.

Of the 1 104 without evidence about half are shops, pharmacies, entrepreneurs and fuel stations.
The other half are about twenty operator families that name something the owner never filed
(first word and receipts): UZCARD 183, MILLIY 85, VIA 58, GOOGLE 48, EZVIZ 27, TRANSPORT 27,
KAPITALBANK 20, NBU 18, PLUM 15, UNIVERSAL 13, HUMO 11, WB 11, HUMOCARDBOT 6, MULTICARD 5, TBC 5,
UB 5, ALLIANCE 4, CLICK 7, ALIF 2, BEELAB 2. One dictionary line per family, added by the owner
in the existing reference screen, resolves the family for old and new receipts alike.

**AI verification is not justified by these numbers**: what is left is either not in the receipt
or is a twenty-line decision for the owner, which no model can make for them. No adapter was
added, no call was made, and no provider or budget decision is needed for this work.

Verification (verified): backend suite `1196 passed, 1 skipped` (`--ignore=tools`), tools `2
passed`; baseline before this work `1092 passed, 1 skipped`. Revision 0033 on a disposable
PostgreSQL 16.15 from the schema of `031d2e4` stamped `20261111_0032`: `upgrade head` twice,
columns `varchar(12)` / `numeric(4,3)` nullable, both check constraints reject bad values, the
two seeded receipts unchanged; `downgrade 20261111_0032` removes both columns and keeps the
receipts; `upgrade head` again; `alembic heads` → `20261112_0033 (head)`.

## 9. Milestone 3 — «Подробности» from confirmed similar operations (2026-11-11)

Code: `backend/services/annotation_rule_service.py`, `annotation_service.py`
(`rows_for_details_training`), `backend/scripts/enrich_details.py` (new). No schema change: a note
rule is stored in `annotation_rules` under the key `OPERATOR KEY | ПК 1234 | DEBIT`.

- **Key**: operator + own card (ПК) + direction. Codes (person, article) keep the operator key.
- **Bar for writing**: ≥ 5 confirmed receipts, ≥ 80 % of them with the note, and the 3 most recent
  confirmed receipts agree. The candidate is always the newest confirmed note. Spacing,
  punctuation and case do not make two notes; the latest spelling is kept.
- **Suggestion only** (reported, never written): ≥ 3 confirmed and the last 3 agree, but too few
  or too mixed.
- **Training**: `manual` and `import` only — never `rule` or `agent` notes.
- **Whole labels**: a note is copied as it stands or not at all. Identifiers inside it are opaque.
- **Protection**: `manual` and `import` notes are never replaced; locked periods are skipped
  (`apply_rules` used to ignore them; `fill_defaults` already skipped them).
- **Self-correction**: `apply_rules(revalidate=True)` / `--revalidate` re-derives the notes a rule
  wrote: one whose pattern changed is corrected, one whose pattern is gone is taken back, with a
  journal event (`transaction_annotation_events`) so it can be undone. Without the flag such notes
  are only counted.
- **Earlier operator-only note rules** (14 in production) are no longer used unless a person
  pinned them; the next recompute switches them off and keeps them as history.

Why these numbers (measured on the 1 066 confirmed notes replayed in date order, so each receipt
is predicted only from earlier ones):

| Variant | Predictions | Right | Precision | Empty receipts reached |
|---|---|---|---|---|
| operator only, support 3, purity 0.9 (before) | 81 | 68 | 0.840 | 0 |
| operator + ПК + type, support 5, last 3 agree, purity 0.9 | 68 | 65 | 0.956 | 23 |
| the same, purity 0.8 (**chosen**) | 79 | 75 | 0.949 | 59 |
| the same, purity 0.7 | 96 | 87 | 0.906 | 66 |
| the same, no purity bar | 177 | 155 | 0.876 | 101 |

The thresholds are `ANNOTATION_DETAILS_MIN_SUPPORT`, `ANNOTATION_DETAILS_MIN_PURITY` and
`ANNOTATION_DETAILS_RECENT_AGREE`. Expect about one automatic note in twenty to differ from what
the accountant would have written; it carries the `rule` provenance and any manual edit replaces it.

Dry run on production through the read-only tunnel (`scripts.enrich_details`, nothing written;
measured):

| | Receipts |
|---|---|
| looked at | 1 932 |
| confirmed by a person or the workbook | 1 066 |
| confirmed patterns (operator + ПК + type) | 208, of them rules: 11 |
| without «Подробности» | 732 |
| **would be filled** | **59** |
| suggestion only, not written | 46 (mixed history 42, few confirmed 4) |
| left empty | 627: no confirmed similar 399, recent notes differ 154, too few confirmed 74 |
| notes an earlier rule wrote | 28: still supported 22, `--revalidate` would take back 6 |
| notes the agent wrote | 106, untouched; where a confirmed pattern exists it differs in 7 of 7 |

Most notes cannot be derived at all: a phone number, a purpose or a name is not in the receipt,
and 392 different notes occur among 1 064. That is a limit of the source, not of the matching.

Verification (verified): `tests/test_details_matching.py` 28 passed; backend suite
`1226 passed, 1 skipped` (`--ignore=tools`).

## 10. Milestone 4 — desktop column order (2026-11-11)

Two commits that touch `desktop/` only, `a3e1835` and `afdf88b` (the second narrows what a click
on the new column does; take both): `tbs-core` `columns.rs`, `format.rs`, `query.rs`; `tbs-app`
`screens/transactions/table.rs`, `edit.rs`, and literal counts in the tests of `view.rs`. No
backend, API, export, workbook, grid-rendering, hub, authentication or menu code.

A fresh table, left to right:

| | Desktop title | Workbook heading | What the cell holds |
|---|---|---|---|
| | `№`, `№ опер.` | `№`, `1` | position in the table; number through time |
| 1 | `Дата и время` | `Дата_Время` | the receipt's own time, to the minute |
| 2 | `(Без дубля)` | `(Без дубля)` | the same time made unique, to the second — **new** |
| 3 | `Источник` | `Сервис` | where the receipt came from: CardXabar, HUMO Card, … |
| 4 | `Операция` | `Операция` | the bank's own line: payment, account change, … |
| 5 | `Оператор/Продавец` | same | |
| 6 | `Приложение` | same | section 4 |
| 7 | `Сумма` | same | |
| 8 | `Остаток` | same | |
| 9 | `ПК` | same | last four digits as text, zeros in front kept |
| 10 | `П2П` | `p2p` | `1` or an empty cell |
| 11 | `Подробности` | same | |

Then, as before, `Лицо`, `Период`, `Статья`, and after them `Тип`, `Валюта`, `Получатель`,
`Карта получателя`. `Дата`, `Время`, `День`, `Описание` stay one tick away in the view panel.
No column was removed: a fresh table shows 20 of 24 (19 of 23 before).

- **`(Без дубля)`** follows the rule of the workbook export: the receipt's time plus 8.64 s for
  every earlier receipt with the same timestamp. "Earlier" is the order of arrival (receipt id),
  counted over all sources as on the workbook's combined first sheet, so a receipt's value does
  not change with sorting, filtering or paging; it is recomputed when a receipt's time is edited
  or a receipt is deleted. The export counts down its own sheet: oldest-first it gives the same
  values, newest-first the steps fall on the receipts of one timestamp in reverse, and a
  per-bank sheet counts within the bank. Shown to the second, because at the workbook's minute
  format the two dates would read alike on screen. Read-only, sortable, copied as shown; an
  export sorted by it is sorted by time. The server needed no change.
- **«Тип»** left the ledger's run and stands after «Статья». «Операция» (the bank's line) and
  «Тип» (the server's word, the one that is edited) remain two columns.
- **«П2П»** cells read `1` or nothing; they used to read «П2П». The editor is unchanged.
- **«ПК»** needed no change: it is text from the API model to the cell, the sort, the copy and
  both exports. Tests now pin a card ending `0626`.
- **Titles are as they were** («Дата и время», «Источник», «П2П»): the same words label the
  filters and the view panel. The workbook's wording is a one-line change per title in
  `Column::title` — the owner's call, not made here.

Saved views (`LAYOUT` 3 → 4), each case under test:

| Saved view | After the update |
|---|---|
| untouched default of any earlier build | the new sequence, once; widths, pinning, density, sorting, filters, paint and page size kept |
| arranged by hand (a column moved, shown or hidden) | its columns in their places; `(Без дубля)` waits unticked at the end; «Как в книге Excel» applies the new sequence on request |
| named saved views | the same two rules, view by view |
| saved by this build and opened again | nothing moves |

Verification (verified; Linux container, Rust 1.99.1): `cargo test -p tbs-core` 120 unit and 2
integration tests passed; `cargo test -p tbs-app --features devtools` 268 passed. The suite's
own picture test (`p4_shots`) was run and looked at: the header reads `№`, `№ опер.`,
`Дата и время`, `(Без дубля)`, `Источник`, `Операция`, `Оператор/Продавец`, `Приложение`, and
the view panel lists all 24 columns in the order above, 20 ticked.

Not verified: the Windows build and installer, and the table on a real screen with live data.

## 11. Integration notes for the hub / UI session

- **Nothing of yours is edited.** The two desktop commits do not touch `crates/tbs-app/src/grid/`,
  `app.rs`, `ui.rs`, `theme.rs`, the hub, authentication or menu screens, the transactions
  screen's `mod.rs`, nor `UI_PROGRESS.md`, `BACKEND_PROGRESS.md`, `IMPLEMENTATION_PLAN.md`.
- **Where the branches can meet**: `screens/transactions/table.rs`. The hunks here are
  `default_order`, `LAYOUT`, `LATER`, `third_layout`, `repair`, `adopt_ledger_layout`,
  `date_linked`, one arm of `TxGrid::cell` (the word «П2П» removed) and tests; `grid_columns`,
  `cell_style` and `row_tone` are untouched. A conflict there is textual: keep both sides.
- **If your branch adds a column or changes the default order**: a new column goes last in
  `Column` and `columns::ALL`, is listed in `LATER` in the order it came, and `LAYOUT` goes up by
  one with the previous default kept as a `*_layout()` function. Otherwise an untouched saved
  view is taken for one arranged by hand and stops following the ledger.
- **Literal counts in tests**: tests that say «Показано 19 из …», «Столбцы · 19» or
  «Столбцов: 19» need 20, and 21 where one more column is ticked.
- **Pinned block**: with three columns pinned, the pinned edge now falls between «Дата и время»
  and «(Без дубля)». Nothing to change in the separator code; it is the place to look at.
- **Pictures**: `desktop/docs/screenshots/p3-*.png` and `p4-*.png` were made before this order
  and are left alone. Make them again once both branches are in:
  `TBS_SHOTS=<folder> cargo test -p tbs-app --features devtools p3_shots`, then `p4_shots`.
- **Rolling the client back**: a view saved by this build names `date_time_dedup`. Client 2.0.1
  cannot read that name, so it starts from its default view and an empty list of saved views.
  Keep a copy of the client's state folder before a rollback.
- **Backend of this branch** (`3dbdbf4`, `92f3a4a`): revision `20261112_0033` adds
  `transactions.application_src` and `application_confidence` and must be applied before the new
  code serves; there is no other schema change. The transaction responses carry both fields; the
  desktop does not read them yet. No route was added or removed.
- **Historical processing is applied nowhere.** Both scripts print aggregates and write nothing
  without `--apply`. From `backend/`, after the migration: `python -m scripts.backfill_applications`
  (`--revalidate` also corrects or clears earlier automatic values, `--top N` lists the most
  frequent operators left empty and may print names) and `python -m scripts.enrich_details`
  (`--revalidate` with `--apply` also corrects or takes back notes an earlier rule wrote).
  `--apply` on production needs the owner's explicit go-ahead and a backup first.

## 12. Open questions and limits

- **«DIK 0626»**: what the number stands for is unanswered (section 5). Nothing waits on it: no
  rule reads inside a note.
- **About twenty operator families** name something the owner never filed (section 8). One
  dictionary line each, entered in the reference screen, settles old and new receipts alike.
- **For review, unchanged**: 4 stored `UzumBank` where the evidence says `Uzum`; 2 receipts
  ambiguous between `Paynet` and `Xazna`.
- **AI**: no provider or budget decision is needed for what was built. The existing switch
  `AI_APPLICATION_MAPPING_ENABLED` stays off; no adapter was added and no call was made.
- **«Подробности»**: about one automatic note in twenty is expected to differ from what the
  accountant would write (section 9); such a note carries the `rule` provenance and any manual
  edit replaces it.
- **Desktop titles**: «Источник» / `Сервис`, «Дата и время» / `Дата_Время`, «П2П» / `p2p`
  (section 10).
- **Not verified here**: the Windows build and installer; `--apply` of either script against
  real data (local tests and read-only dry-runs only).

## 13. Status

| Milestone | State |
|---|---|
| 1. Investigation and plan | measured, pushed `9287e0f` |
| 2. Application detection, provenance, dry-run | verified, pushed `3dbdbf4` |
| 3. Details matching, dry-run | verified, pushed `92f3a4a` |
| 4. Desktop column order (separate commits) | verified on Linux, pushed `a3e1835`, `afdf88b` |
| 5. Optional AI adapter | not built: not justified (section 8) |

Final check on the tree of `afdf88b` (verified): backend `1226 passed, 1 skipped`
(`--ignore=tools`), tools `2 passed`; `tbs-core` 120 + 2 passed; `tbs-app` 268 passed.
Branch `work/receipt-enrichment`; this file is the last commit on it.
