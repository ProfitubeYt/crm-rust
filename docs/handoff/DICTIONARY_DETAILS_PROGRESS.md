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

| | Researched | Proposed | Imported / applied |
|---|---|---|---|
| Dictionary lines | 12 verified entries (17 lines), 11 entries for the owner, 9 groups that name no application | 17 | **0** |
| «Приложение» values | — | 656 fills | **0** |
| «Подробности» values | — | 59 fills, and 46 suggestions that are never applied | **0** |

## 1. Access used

- Shared read-only key supplied by the owner as a task attachment, installed outside the worktree
  with mode 600; client fingerprint matched `SHA256:WEfc++gQ…DpYE`.
- Server ED25519 key scanned, compared with the verified fingerprint `SHA256:v6EsGV56…22SM`, pinned
  only after it matched; `IdentitiesOnly=yes`, `StrictHostKeyChecking=yes`.
- `pgpass` went straight into a mode-600 file. One tunnel `127.0.0.1:15432 → 127.0.0.1:9990`, session
  forced `default_transaction_read_only=on`, one connection at a time. The tunnel is closed.
- Measured 2026-11-12: PostgreSQL 15.16, time zone UTC, schema `20261112_0033`, no active locked
  period. Receipts: 1 934 at 08:49 UTC (the private snapshot), 1 938 at 09:49, 1 939 at 10:09
  (highest id 10532), on 270 distinct operator lines.

## 2. Milestones

| # | Milestone | State |
|---|---|---|
| 1 | «Описание» removed from the desktop table and the view panel; saved views migrate | done, tests **verified** on Linux; Windows **not verified** |
| 2 | Dictionary: why it did not grow, full operator inventory, web research, verified import | research and import plan done; 17 lines **proposed**, **none imported** (read-only access) |
| 3 | «Подробности»: recalculated dry-run, protected manifest, administrator procedure | done as a preview; 59 fills **proposed**, **none applied** |
| 4 | Ongoing processing: cache invalidation, review of unknown operators | code and tests done; **not deployed** (no deployment is authorised) |

## 3. Milestone 1 — «Описание»

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
  (Rust stable, Linux; run again on the pushed code at the end of the session).
- **verified**: `cargo test -p tbs-app --features devtools --locked` inside the image built from
  `desktop/tools/linux-dev.Dockerfile` — 306 passed, 0 failed.
- **not verified**: Windows build, installer and a real Windows screen.

## 4. Milestone 2 — the application dictionary

### 4.1 Why it did not grow

It was never built to grow by itself, and nobody added lines. In detail:

- **Where it lives.** `operator_reference` (operator text, application, p2p flag, active flag):
  70 active and 95 inactive lines (measured). `operator_mappings` is a second table that only the
  synchronisation set and an old seed script still name; the matcher does not read it and it is
  empty.
- **How it is edited.** By a person, in the reference screen (`/api/reference`: create, change,
  delete, Excel export and import). Production also has a unique index on (operator, application).
- **How it is used.** `OperatorMapper` reads the active lines through a per-process cache of
  120 seconds; `application_detection` adds the brand rules. A new receipt gets its application
  when it is parsed. An old receipt gets one only when an administrator runs
  `scripts.backfill_applications --apply`.
- **Automatic population never existed.** The only writer besides a person is the optional AI
  parser, and it stores its proposal as an **inactive** line. The 95 inactive lines date from
  2026-05-23 and 2026-07-12; no screen or job ever activates them, and some are wrong.
- **Persistence and matching are not broken.** The 70 lines are stored and matched; in the dry run
  the dictionary, with the 17 new lines previewed, answers 270 empty receipts by the existing
  mechanism (exact 150, variant 72, substring 48).
- **One real defect, small.** The reference routes did not drop the matcher's cache, so an edited
  line was ignored for up to two minutes. Fixed in the branch for the process that serves the edit;
  the workers pick a line up within the same two minutes, as before.
- **Why the owner saw nothing change.** Besides the missing lines, the historical pass of 2.0.2 was
  never applied: all 278 stored applications carry no source, and 457 old receipts that today's
  dictionary and brand rules already answer are still empty.

### 4.2 Every operator line, accounted for

Measured at 10:09 UTC: 1 939 receipts, every one with an operator line, 270 distinct lines
(256 names once country code and city are set aside), 1 661 receipts without an application.

| What answers the line | Lines | Receipts | Without application |
|---|---|---|---|
| An active dictionary line, today | 16 | 241 | 71 |
| A brand rule, today | 46 | 473 | 386 |
| One of the 17 verified lines, once imported | 32 | 202 | 199 |
| Nothing yet: evidence without official proof, for the owner | 19 | 43 | 41 |
| Nothing: not an application (bank office, generic card-to-card, fare, government) | 36 | 419 | 407 |
| Nothing: a merchant or a person (address, name, legal form) | 112 | 464 | 464 |
| Nothing: no operator in the receipt, cash or ATM | 6 | 92 | 91 |
| Nothing: unreviewed, arrived after the inventory | 2 | 2 | 2 |
| Only a stored value (no line or rule answers it) | 1 | 3 | 0 |
| **Total** | **270** | **1 939** | **1 661** |

The first three rows are the 656 proposed fills. The inventory was taken at 1 934 receipts and 268
lines; the two unreviewed lines came with later receipts, read as merchants and wait in the review
list. The line-by-line inventory names merchants and people, so it stays out of git;
`python -m scripts.operator_dictionary review --names` rebuilds it for somebody who may see receipts.

### 4.3 Research

`backend/data/operator_dictionary/research_20261112.json` holds, per entry, the operator lines, the
application, the reason, the source pages with the quoted sentence and the date, and a confidence.
Searches used public organisation and service names only; a snippet was never evidence. Every
source page of the verified section was fetched again at the end of the session and the recorded
quote was found on it (17 pages: Google Play and App Store listings, click.uz, aab.uz, nbu.uz,
humocard.uz, via.uz, uzcard.uz, ipotekabank.uz, alif.uz, gov.uz, ezviz.com, uzum.com,
support.google.com, gazeta.uz).

A line is verified only when it carries the brand of an application or online service, an official
source confirms that service and its function, nothing else plausibly prints the same line, and it
contradicts nothing the owner typed. A bank's or merchant's name alone was never enough.

**Proposed for import, 17 lines** (receipts carrying exactly that line):

| Application | Lines |
|---|---|
| Plum | `PLUM OPLATA, UZ` (15) |
| Click | `CLICK P2P, UZ` (2), `CLICK P2P U2H ER>TAS` (1), `UZCARD TO HUMO CLICK` (1), `AO CLICK, UZ` (4) |
| Alliance Pay | `ALLIANCE PAY P2P NA` (4) |
| Milliy 2.0 | `NBU KONVERS MILLIY 2026 UZCARD, UZ` (11), `NBU KONVERS MILLIY 2` (6) |
| HUMO Card | `HUMOCARDBOT PREMIUM` (7; one stored «Humans» differs and is left alone) |
| VIA | `VIA PAYMENT, UZ` (58) |
| Paysend | `Ipoteka bank PAYSEND` (1) |
| Alif | `ALIF UZ U2U 075, UZ` (2) |
| Wildberries | `WB INT EXPORT OPLATA, UZ` (11), `OOO WB INT EXPORT, UZ` (1) |
| EZVIZ | `EZVIZ` (27) |
| UzumBank | `UB VISAUZUM TO HUMO>` (3; an inactive AI twin exists and is not touched) |
| Google Play | `GOOGLE` (48 receipts on 15 lines) |

The owner's answers of 2026-11-12, recorded in the research file: «Google Play» for the Google
lines; mappings without official proof are **reported only**; no application was named for the
largest lines that show only a bank or a card-to-card operation, so they stay empty.

**For the owner, not imported** (19 lines, 41 receipts): other NBU lines (Milliy 2.0?), `UB P2P
Humo2Humo` (UzumBank?), the TBC lines, the Multicard gateway (the app is now Rahmat), `BEELAB
OPLATA` (Beepul?), `UNIVERSAL BANK VIVOD` (Unired?), `Hamkor Bank`, two lines cut at 20 characters
(Mavrid, Paynet), `XAZNA PAYNET` (two applications in one line) and one ATTO merchant line.

**Not an application** (36 lines, 407 receipts): `UZCARD TO VISA` and its kin (at least eight
providers offer the same transfer), `MILLIY BANK PK KIRIM` (an incoming credit), the offices of
Asia Alliance Bank, Kapitalbank and others, transport fares, fines and tax cashback, a parking
operator and a few merchants.

**The owner's 70 existing lines** were checked too, and a second independent check of the same 20
applications agreed: every application exists. Nothing is changed; the research file lists what the
owner may want to look at:

- Joyda is SQB Mobile since 13.11.2024, so the dictionary holds one app under two names; SmartBank
  is Openbank since 2025; the bank now calls Davr Mobile «DavrMobile 2.0».
- UPay is a separate app from Humans, and a court declared its owner bankrupt on 2 February 2026.
- Four lines carry no word of their application (`UZCARD P2P`, `UZCARD PLYUS P2P`, `NEW DBO
  UZKART-HUMO`, `PSP P2P AKSIYA`) and the Xalq Banki head-office line cannot be told apart between
  Paynet and xazna: they rest on the owner's knowledge.
- «Яндекс» comes from the owner's own example through a brand rule of 2.0.2: the line itself reads
  `YANDEXPLUS`. Nothing public ties Octobank to Yandex beyond that line.

Stored values that differ from the evidence are reported, never overwritten: 4 × UzumBank where the
line says Uzum Market, 1 × Humans on the HUMO Card bot, 12 × Milliy 2.0 on the incoming-credit line.

### 4.4 Imported

**Nothing.** The session can only read. `scripts.operator_dictionary plan` against production says
`add: 17`, no conflict.

## 5. Milestone 3 — «Подробности»

- A note is copied whole from confirmed receipts (set by a person or imported from the owner's
  workbook) of the same operator line, card and direction, under the unchanged thresholds of 2.0.2.
  Nothing is composed: a number such as `DIK 0626` is carried as opaque text or not at all.
- What automation wrote is never evidence: 106 agent-written notes and the rule-written ones are
  not learned from and not changed.
- **Precision on confirmed examples**: replayed in date order over the 1 064 confirmed notes of the
  snapshot, each predicted only from earlier ones, the current key made 79 predictions and 75
  matched the person's note (94.9 %). Eleven other keys were replayed the same way (operator name
  without city, amount, application, receiver card, no card): none was more precise, so no key was
  added and no threshold lowered.
- **Proposed** (measured, 739 empty notes): 59 confident fills; 46 suggestions that are only
  reported (42 mixed history, 4 too few confirmed); 634 left empty (405 no confirmed similar
  receipt, 155 the latest notes differ, 74 too few confirmed); 0 in a locked period.
- **Applied: nothing.**

## 6. Milestone 4 — ongoing processing

- Creating, changing, deleting or importing a dictionary line drops the matcher's cache in the
  process that served it, so the next receipt it parses uses the line; other processes follow
  within 120 seconds. A receipt parsed before the line is filled once by the historical pass, and a
  value a person set stays. Tested: `tests/test_operator_dictionary.py`.
- Unknown operators are exposed for review: `GET /api/reference/review` (reference tab access) and
  `python -m scripts.operator_dictionary review`. Each line has its frequency, the researched
  candidate with sources and a status. Merchant lines are counted and not listed. Nothing is
  published from there: a line becomes a value only when a person adds or activates it.
- The note rules can relearn on a timer (`ANNOTATION_DETAILS_RULES_REFRESH_HOURS`), **off by
  default**: the first relearning also lets the background pass write notes into old receipts, and
  that belongs to the reviewed manifest, not to a timer.
- None of this runs in production until the branch is deployed, which is a separate proposal.

## 7. The change preview and the administrator's procedure

- `docs/handoff/ADMIN_EXECUTION_REQUEST_20261112.md` is the request: what would be written, the
  backup, the commands, the stop rules, the rollback and the evidence.
- `scripts.build_change_manifest` (reads only) writes the protected manifest: every planned change
  with the value it replaces, the suggestions and the locked receipts apart, mode 600, refused
  inside the repository. Content digest of the reviewed plan:
  `720660a287af0bc93266c8105b137763b582b3bc1425d6b10065120d5f34df6b` (identical at 1 934, 1 938 and
  1 939 receipts, and on the rehearsal copy).
- `scripts/apply_change_manifest.py` (standalone) verifies, applies with a per-record journal and a
  batch id, and takes exactly one batch back, leaving later edits and newer receipts alone.
- Rehearsed end to end on a private PostgreSQL 15.19 copy (17 / 656 / 59 applied, a second run wrote
  nothing, rollback restored every affected value); `verify` against live production reports
  ready 17 / 656 / 59. The live schema of the written tables was compared with the copy.
- **Protected locations.** In this sandbox, outside git and mode 600:
  `/tmp/hoplite/private/manifest/manifest-production-20261112b.json` (the preview) and the
  rehearsal journal under `/tmp/hoplite/private/manifest/pg/`. They vanish with the sandbox; the
  administrator rebuilds the manifest on the server and compares the digest. **No production
  backup and no production journal exist**, because nothing was written; the request places them
  under `/opt/parcer-backups/<batch>/`.

## 8. Checks that ran (all **verified**, Linux)

- `pytest -q` in `backend/` (tests and tools): 1244 passed, 1 skipped. The new and changed tests:
  `tests/test_operator_dictionary.py` (plan, conflicts, duplicates, preview, cache after an edit,
  next receipt, reprocessing once, review list and route, research file without private data),
  `tests/test_change_manifest.py` (manifest, refusal inside the repository, apply, journal, edits
  and locked periods in between, idempotence, rollback of one batch, parts and their order),
  `tests/test_details_matching.py` (receipt-level plan, relearning off by default and on a timer).
- `cargo test -p tbs-core --locked`: 123 + 2 passed. `cargo test -p tbs-app --features devtools
  --locked`: 306 passed.
- All fixtures are synthetic. No customer receipt is in a test or in git.

## 9. Not verified, and limits

- Production writes of any kind; the administrator's commands on the server (container and service
  names, the `docker compose` invocation); the tool inside the live 2.0.2 image.
- Windows build and installer, a real Windows screen, the Excel add-in on Windows.
- The 59 notes are confident by the thresholds, not certain (94.9 % on replay).
- The research is a snapshot of 2026-11-12; a renamed application needs a new look.

## 10. What remains, and whose it is

- Administrator, with the owner's approval and a fresh verified backup: the request in section 7.
- Owner: the 19 lines without official proof, the points on the existing lines, the stored values
  that differ, and whether the note rules are turned on for new receipts.
- A release that carries milestones 1 and 4 to the server and the desktop: a separate proposal.
