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
| 2 | `docs/EXCEL_SYNC.md`: prepared-workbook flow, contract, deployment proposal; loopback HTTP run; gate test | verified, pushed |
| 3 | Operation parity (plan P1.7): additive transaction fields, export sort, contract note | pending |

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
| Whole backend | `pytest tests` | 1084 passed, 0 failed, 0 skipped |
| Migration 0032, PostgreSQL 16 | alembic CLI, see below | passed |
| New API tests on PostgreSQL 16 | same test module, `db_session` bound to the migrated database | 123 passed |
| Existing sync API tests on PostgreSQL 16 | `tests/test_excel_sync_api.py`, same harness | 107 passed |
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

The gates of the real application were checked per middleware (`SystemAccessMiddleware`,
`LaunchSessionMiddleware`): making, fetching and listing workbooks are refused without the application's access,
while `/v1/activate` and `/v1/snapshot` of a prepared workbook still work.

## Limits of this verification

- **No real Excel ran.** Nothing here executes VBA or opens the workbook in Excel; the file is read with openpyxl,
  ElementTree and zipfile, and the workbook's HTTP calls are replayed by the tests. The native PowerShell suites in
  `backend/tools/excel_sync/testing` need Windows and Excel and were not run.
- The Rust client was not built or run in this session (out of scope); its expectations were read from source.
- Production was neither read nor written.

## Contract for the UI session

No change to any existing method, path or field. Milestone 1 only adds or tightens:

- `POST /api/excel/workbooks` replies `201 {"workbook": {...}, "server_url": "<address written into the file>"}`.
  `server_url` is additional to what the Rust client reads and may be ignored.
- 422 bodies of `/api/excel/*` no longer contain `detail[].input`. `detail[].msg`, `loc`, `type` are unchanged.
- `server_url` naming this host with a port other than the one in `Host` is now `422 invalid_server_url`.

The full contract (fields, codes, address rules, compatibility, deployment proposal) is in `docs/EXCEL_SYNC.md`,
section «Готовая книга с сервера: 11.11.2026».

## Remaining

- Milestone 3 as in the table above.
- Deployment proposal (not executed, needs separate authorization): `alembic upgrade head` (adds one nullable
  column, reversible), optional `EXCEL_SYNC_PUBLIC_URL=https://<public host>` in the server's `.env` (the backend
  service already reads `.env`; no compose change), then restart of the backend. A commented entry for
  `.env.example` is proposed for integration, since root files are outside this branch's scope:

  ```
  # Address Excel workbooks reach this server by (https://host[:port], no path). When unset the address the
  # desktop client used, or the request's own, is written into a prepared workbook.
  # EXCEL_SYNC_PUBLIC_URL=https://example.com
  ```
