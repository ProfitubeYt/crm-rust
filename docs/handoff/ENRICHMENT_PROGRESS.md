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
- «ДиК» is an accountable-person code (28 receipts). A number such as `0626` after it occurs
  nowhere in production. The owner confirmed on 2026-11-11 that «DIK 0626» was given as an
  example of an existing label and that its number must not be interpreted or generated: a label
  is reused whole, and only when the matching evidence is sufficient.
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

The workbook export already follows this, and it is not changed by this work: the owner
clarified on 2026-11-11 that item 4 is about the desktop table only, with the workbook as the
reference for the order. The desktop table has no `(Без дубля)` column, and «Тип» stands between
«Операция» and «Оператор».

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

## 9. Status

| Milestone | State |
|---|---|
| 1. Investigation and plan | measured, pushed `9287e0f` |
| 2. Application detection, provenance, dry-run | verified, pushed |
| 3. Details matching | not started |
| 4. Desktop column order (separate commit) | not started |
| 5. Optional AI adapter | not built: not justified (section 8) |
