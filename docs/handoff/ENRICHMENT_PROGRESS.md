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
  nowhere in production; its meaning was asked of the owner and it is treated as opaque text
  until answered.
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

The workbook export already follows this. The table export and the desktop table do not: they
have no `(Без дубля)` column, and «Тип» stands between «Операция» and «Оператор».

## 7. Plan

1. **Application detection** (backend): suffix- and truncation-tolerant dictionary matching;
   explicit brand evidence for the confirmed application vocabulary and the owner's examples;
   conflicts and merchant-shaped operators left empty; provenance and confidence stored;
   manual values never replaced; verified-similar tier fed only by human values.
2. **Historical dry-run** for applications and details: aggregate proposed changes, unresolved
   counts and anonymised reasons; writing only with an explicit flag; idempotent.
3. **Details**: operator + ПК + type key with recency agreement, thresholds from the table
   above; weaker evidence reported as suggestions, not written; rule-written values re-derived.
4. **Column order**: table export sequence and a separate desktop commit.
5. **Optional AI verification**: decided after the rule coverage is measured; adapter only,
   disabled by default, no calls.

## 8. Status

| Milestone | State |
|---|---|
| 1. Investigation and plan (this file) | measured, pushed |
| 2. Application detection + dry-run | not started |
| 3. Details matching | not started |
| 4. Column order (export, desktop) | not started |
| 5. Optional AI adapter | not started |
