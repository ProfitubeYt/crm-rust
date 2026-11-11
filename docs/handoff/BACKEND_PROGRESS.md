# Backend progress — Excel workbooks and synchronization

Branch `work/excel-backend`, created from baseline `a83b04467db66c24bf904b093a4ba9edc7d232c3`
(the only commit on `origin/main` when work started; no newer history existed).
Owner of this file and of `docs/EXCEL_SYNC.md`: the backend session. `IMPLEMENTATION_PLAN.md`,
`desktop/`, `frontend/` and the shared handoff files are not modified on this branch.

Status words used below: **verified** = a check listed under "Verification" ran in this session and passed;
**not verified** = no such check exists yet.

## Milestones

| # | Content | State |
|---|---|---|
| 1 | Workbook API audited and hardened, `test_excel_workbook_api.py` written, migration 0032 round trips | verified, pushed as `5d4156e` |
| 2 | `docs/EXCEL_SYNC.md`: prepared-workbook flow, contract, deployment proposal; loopback HTTP run; gate test | verified, pushed as `4f6995e` |
| 3 | Operation parity (plan P1.7): `stored_transaction_type` and `excel_bank` in the transaction API, export honours `sort_by=operation`, contract note | verified, pushed |

## Environment (nothing of it is in git)

- Python 3.11 (the version of `backend/Dockerfile`) in a virtualenv at `/tmp/hoplite/excel-backend-env/venv`,
  `uv pip install -r backend/requirements.txt` — the pinned requirements, plus `oletools` so that the optional
  VBA-container test runs instead of skipping.
- Test environment as documented in `docs/EXCEL_SYNC.md`: `PYTHON_DOTENV_DISABLED=1 DATABASE_URL=sqlite://
  TELEGRAM_API_ID=1 TELEGRAM_API_HASH=test-only-api-hash JWT_SECRET=test-only-jwt-secret`, run from `backend/`.
- Disposable PostgreSQL 16.15: `initdb` under `/tmp`, listening on `127.0.0.1:54329` only, `pg_trgm` enabled.
- Shared SSH/production: `HOPLITE_SSH_KEY_PATH` is not set in this sandbox and no key file was supplied, so the
  read-only tunnel was **not used**. Nothing in this work needed it. No production read or write happened.

## Audit of the inherited WIP (routes, builder, model, migration)

Baseline state, measured before any edit: full backend suite 956 passed, 2 failed, 1 skipped. The two failures
were not caused by the WIP: `test_grant_is_hashed_one_time_user_owned_and_revocable` and
`test_operator_cannot_issue_permanent_grant_but_keeps_default_expiry` compare a term with `pytest.approx` on a
`timedelta`, which pytest 7.4 evaluates as strict equality, while `create_grant` read the clock twice. They passed
on Windows only because its clock is coarse.

The WIP was kept; it already had the right shape. What the audit confirmed against the Rust client
(`desktop/crates/tbs-core/src/api/excel.rs`, `tbs-app/src/screens/transactions/excel.rs`) and the VBA
(`backend/tools/excel_sync/ExcelSync.bas`):

- `POST /api/excel/workbooks`, `GET /api/excel/workbooks/{id}/file` and the reply shapes match the client's structs
  and its error mapping (`detail.error` codes survive the application's error envelope).
- `_sync` cells: `B1` workbook id (StartSync), `B5` server address and `B7` owner (ConfigureSharedProfile,
  CheckIdentity compares `B7` with the snapshot's `user_id`), `B8` = `"1"` (SharedProfile; any other value stops
  StartSync with "Shared connection is disabled"), `B6` and `B9` blank. `B2`–`B4`, `B10` stay the template's.
- The record is stored only after the address was validated and the file was built, so a refused request leaves
  nothing behind; `write_audit_log` commits the record and its audit entry together.

Defects found and fixed in milestone 1 (all in `backend/api/routes/excel_sync.py`):

1. **A validation error echoed the workbook password.** FastAPI puts the rejected input into a 422 body; for a
   missing or mistyped field that is the whole request, password included. The Excel router now uses a route class
   that drops `input` from its validation errors. Other routers are unchanged.
2. **A malformed `Host` header crashed the address check** (`urlsplit` raises on `[::1`): now a refusal
   (`422 invalid_server_url`, or `503 excel_public_url_not_configured` when no address was sent).
3. **Only the host of a client-supplied address was compared.** A port named in `Host` now has to agree as well; a
   `Host` without a port (a proxy that drops it) still compares the host alone.
4. `create_grant` reads the clock once, so a grant's term is exact and the two baseline failures are gone.

Observed, deliberately not changed (outside this scope or pre-existing by design):

- The application's `HTTPException` handler in `api/main.py` drops exception headers, so `Retry-After` of the
  activation lockout (429) does not reach clients of the real application. The lockout itself is enforced; the VBA
  shows a fixed "15 minutes" text and does not read the header.
- A grant binds to whatever workbook id its first request names. For a workbook **without** a password record a
  user who knows its id can therefore tie a grant of their own to it and then protect it. They still only ever see
  their own receipts (the VBA refuses a foreign owner in CheckIdentity), and a workbook made by the server always
  has a record, which decides ownership alone. Behaviour of revision 0031, unchanged.
- The migration chain cannot start from an empty database (revision 0001 alters tables it expects), which is why
  the PostgreSQL check below starts from the previous release's models.

## Verification (2026-11-11, this sandbox)

All commands from `backend/` with the environment above.

| Check | Command | Result |
|---|---|---|
| New API tests | `pytest tests/test_excel_workbook_api.py` | 123 passed |
| Migration 0032, SQLite | `pytest tests/test_excel_workbook_migration.py` | 2 passed |
| Excel suites together | `pytest tests/test_excel_workbook_api.py tests/test_excel_workbook_builder.py tests/test_excel_workbook_migration.py tests/test_excel_sync_api.py tests/test_excel_sync_service.py tests/test_excel_vba_package.py` | 481 passed (480 + 1 skipped without `oletools`) |
| Operation parity | `pytest tests/test_transaction_operation_parity.py` | 9 passed |
| Export and layout suites next to it | `pytest tests/test_export_client_layout.py tests/test_transactions_export.py` | 101 passed |
| Whole backend | `pytest tests` | 1093 passed, 0 failed, 0 skipped |
| Migration 0032, PostgreSQL 16 | alembic CLI, see below | passed |
| New API tests on PostgreSQL 16 | same test module, `db_session` bound to the migrated database | 123 passed |
| Existing sync API tests on PostgreSQL 16 | `tests/test_excel_sync_api.py`, same harness | 107 passed |
| Operation parity on PostgreSQL 16 | `tests/test_transaction_operation_parity.py`, same harness | 9 passed |
| The flow over a real socket | `tools/excel_sync/test_server.py` (uvicorn, `127.0.0.1:8767`) + an httpx client, see below | 34 of 34 checks |

What `test_excel_workbook_api.py` covers: the create reply and stored record (hash only, audit entry, no grant);
listing before any grant; 22 malformed requests; text that cannot be stored; address normalisation; 39 malformed or
foreign addresses on create and on download; port agreement; plain-http loopback; `EXCEL_SYNC_PUBLIC_URL` winning
and being unusable; malformed `Host`; 7 broken templates (missing, empty, not an archive, truncated, without
macros, the export layout, somebody's configured workbook) leaving no record; rollback when the audit entry or the
commit fails; the per-owner limit; captured scope and permissions; a stale scope; inactive, locked and expired
owners; the file (only the `_sync` part differs, `vbaProject.bin` identical, no password/hash/salt/token in any
part, same bytes on every download); file names; another user's download, password change and removal; unknown
ids; workbooks connected by hand and password records without a name; then the workbook's own calls read from the
file: wrong password refused, activation, snapshot limited to permitted receipts with `user_id` equal to `B7`, key
bound to computer and workbook, revocation, lockout, password change, changed permissions and scope, key
replacement, connection limit, removal of the password.

PostgreSQL round trip (disposable cluster; alembic command line, not test doubles):

1. Schema created with the models of `8420c8f` (the backend before this work), `alembic stamp 20260923_0031`, one
   password record and one device grant inserted.
2. `alembic upgrade head` twice → `name character varying(120)`, nullable; the record unchanged, `name` NULL.
3. `alembic downgrade 20260923_0031` → the column is gone, both password records and the grant remain.
4. `alembic upgrade head` → column back, names NULL.
5. Column already present but stamped 0031 → upgrade succeeds without touching it; column absent but stamped head
   → downgrade succeeds. `alembic heads` → `20261111_0032 (head)` only.

Loopback run (real TCP, no test client): the synthetic server of the native Excel suites
(`backend/tools/excel_sync/test_server.py`: the real router and service, fresh in-memory SQLite, uvicorn on
`127.0.0.1`) answered an httpx script that first acts as the desktop client and then as the workbook, taking the
address and identifiers of every later request from the downloaded file. All 34 checks passed: `201` with the five
fields; address written `http://127.0.0.1:8767`; `Content-Type`, `Content-Disposition` (RFC 5987, Cyrillic name)
and `no-store` on the file; `B1`/`B5`/`B7`/`B8` set and `B6`/`B9` blank; one changed part (`xl/worksheets/sheet7.xml`);
`vbaProject.bin` SHA-256 equal to the template's; no password in the file; snapshot without a key `401`; wrong
password `401 excel_activation_password_invalid`; right password `201` and a key of the form the VBA accepts;
snapshot `200`, complete, `user_id` equal to `B7`, 2 receipts; conditional snapshot `304`; the key on another
computer `403`; an edit through `/v1/sync` applied; the computer listed under its workbook; password change keeps
the name and the activated computer; re-download byte-identical; revocation → `401` for reads and writes; the old
password no longer activates; a foreign `server_url` → `422 invalid_server_url` and nothing stored.

The file downloaded in that run was also read with `oletools` (an independent VBA reader): 15 modules; the five
source modules (`ThisWorkbook`, `SyncJson`, `SyncWindows`, `ExcelSync`, `BankViews`) are identical to
`backend/tools/excel_sync`; `xl/vbaProject.bin` has the SHA-256 of the template's.

The gates of the real application were checked per middleware (`SystemAccessMiddleware`,
`LaunchSessionMiddleware`): making, fetching and listing workbooks are refused without the application's access,
while `/v1/activate` and `/v1/snapshot` of a prepared workbook still work.

## Limits of this verification

- **No real Excel ran.** Nothing here executes VBA or opens the workbook in Excel; the file is read with openpyxl,
  ElementTree and zipfile, and the workbook's HTTP calls are replayed by the tests. The native PowerShell suites in
  `backend/tools/excel_sync/testing` need Windows and Excel and were not run. Steps for a manual check in Excel
  against the loopback server are in `docs/EXCEL_SYNC.md` («Как проверить в настольном Excel»).
- The Rust client was not built or run in this session (out of scope); its expectations were read from source.
- Production was neither read nor written.

## Contract for the UI session

No existing method, path or field changed. Everything below is additive or tightening, and none of it has to be
present for the desktop client to work: an older server simply does not send the new fields.

### Excel workbooks (milestones 1–2)

- `POST /api/excel/workbooks` replies `201 {"workbook": {...}, "server_url": "<address written into the file>"}`.
  `server_url` is additional to what the Rust client reads and may be ignored.
- 422 bodies of `/api/excel/*` no longer contain `detail[].input`. `detail[].msg`, `loc`, `type` are unchanged.
- `server_url` naming this host with a port other than the one in `Host` is now `422 invalid_server_url`.

The full contract (fields, codes, address rules, compatibility, deployment proposal) is in `docs/EXCEL_SYNC.md`,
section «Готовая книга с сервера: 11.11.2026».

### «Операция» inputs and export order (milestone 3, plan P1.7)

Two fields were added to `TransactionResponse`, so to `GET /api/transactions/` (`items[]`),
`GET /api/transactions/{id}`, `POST /api/transactions/` and `transaction` of the process-receipt reply:

| Wire name | JSON type | Value |
|---|---|---|
| `stored_transaction_type` | `string \| null` | the row's `transaction_type` as stored: `DEBIT`, `CREDIT`, `CONVERSION` or `REVERSAL`; identical to `records[].fields.transaction_type` of the Excel snapshot |
| `excel_bank` | `string \| null` | the bank assigned to the row in the workbook: `CardXabar`, `HUMO Card` or `null`; identical to `records[].fields.excel_bank` of the Excel snapshot |

`transaction_type` (the type inferred from the stored one and the receipt text) and `transaction_type_display`
keep their meaning and values. Expected consumption, which this branch does not implement: the «Операция»
fallback takes `stored_transaction_type` when the key is present and not null, otherwise `transaction_type`; an
absent field stays absent. The Rust model already has `excel_bank: Option<String>` and `operation::display`
already prefers it, so the **current** client starts honouring assigned banks as soon as a server with this change
answers it. No Rust struct uses `deny_unknown_fields`, so `stored_transaction_type` is ignored until it is modelled.

`GET /api/transactions/export` now honours `sort_by=operation` (it used to fall back to date silently), in both
layouts:

- key: the text of the «Операция» cell the export itself writes in the client layout — the bank's headline from
  the first line of the receipt, else mark and label of the **inferred** type in the style of the **source's** bank;
- comparison: lower-cased, by Unicode code point (what `Key::Text(cell.to_lowercase())` does in `query.rs`);
- equal keys: by date, then id, ascending, in both directions — the order `fetch_all` receives rows in, which the
  client's stable sort keeps;
- `№ опер.` counts downward only for date-descending, as before; every other `sort_by` is unchanged and
  `GET /api/transactions/` does not sort by `operation`.

Where the orders can differ, so that nobody has to guess: for a row whose receipt has a bank headline the text is
the same in the workbook, the client and the export. For a row without one, the export uses the inferred type and
the source's bank; the workbook uses the stored type and `excel_bank`; the client uses `transaction_type` today
plus `excel_bank` once it arrives. Such rows therefore sit at another place in the exported file than on screen
when they carry an `excel_bank`, and — after the client moves to `stored_transaction_type` — when stored and
inferred types differ. The export's cell was left alone on purpose: its label and its sign column are both built
on the inferred type and it ignores assigned banks altogether. **Decision for integration:** whether the export's
«Операция» should follow the workbook's rule (stored type, assigned bank). That changes exported content for those
rows and was not done here.

## Remaining

- **Not done: the production count asked for by plan P1.7** ("how many production rows the difference touches").
  It needs the read-only tunnel, which this sandbox does not have. Method for whoever has it, one connection, no
  writes: read `id, transaction_type, raw_message, excel_bank` of `transactions`, and count the rows for which
  `services.excel_client_layout.operation_from_raw(raw_message)` is `None` (no bank headline) and either
  `api.routes.transactions.infer_transaction_type(transaction_type, raw_message) != transaction_type` or
  `excel_bank` is not null. Those are exactly the rows where workbook, client and export can disagree.
- Not done, outside this scope: consumption of `stored_transaction_type` in the Rust client; the decision about
  the export's «Операция» cell described above; a run of the native Excel suites and of the Rust live suite
  against this backend.
- Deployment proposal (not executed, needs separate authorization): `alembic upgrade head` (adds one nullable
  column, reversible), optional `EXCEL_SYNC_PUBLIC_URL=https://<public host>` in the server's `.env` (the backend
  service already reads `.env`; no compose change), then restart of the backend. A commented entry for
  `.env.example` is proposed for integration, since root files are outside this branch's scope:

  ```
  # Address Excel workbooks reach this server by (https://host[:port], no path). When unset the address the
  # desktop client used, or the request's own, is written into a prepared workbook.
  # EXCEL_SYNC_PUBLIC_URL=https://example.com
  ```
