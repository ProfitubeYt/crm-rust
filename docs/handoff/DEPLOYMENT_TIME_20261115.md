# Time, Hub and global CRM administration — deployment 2026-11-15

Source: `5c0361eb4b293ac354c2a2ffd89e1a140e780e0c`, branch
`codex/time-production-release`, repository `asintiko/crm-rust` (private).
Later commits that add this record and evidence do not change runtime code.

## Production result

- Live preflight confirmed release 2.0.3, schema 0033 and precisely two active Checks
  administrators. Migrations 0034 and 0035 are now applied; schema is `20261112_0035`.
- Backend and all three workers plus TBS Auth use
  `parcer-backend:20261115-time-5c0361e`. Workers and Time reached healthy state;
  backend health reports a connected database. No HTTP 500 responses appeared in
  the initial post-deployment backend log window.
- Time uses the off-server-built image
  `timelist:5c0361eb4b293ac354c2a2ffd89e1a140e780e0c`, a named SQLite volume,
  no published port, and private authenticated backend-to-Time requests.
- `/admin/` returns 200 over HTTPS. `/api/time/meta` without a user session returns
  401. Existing Caddy already forwards the admin path through its fallback route:
  **its file was preserved byte-for-byte**, as were unrelated services and volumes.
- Exactly the two owner-approved accounts retain CRM administration. All existing
  accounts retain Checks; no Time role was granted automatically. Grant Time access
  through CRM administration when wanted.
- Account count and a server-private comparison of all accounts' password/2FA
  material, roles, activation and Telegram links are unchanged. Sessions were not
  cleared. TBS Auth configuration was preserved; no authenticator was enrolled.
- The owner's clarification is Telegram TBS Auth, not TOTP enrollment. Existing
  scope OTP and Telegram QR login are distinct from account-password TOTP; this
  deployment does not add a mandatory Telegram second factor to every password login.
- The Time bot is deliberately OFF pending a reissued token, privacy disabled and
  a verified negative work-group ID. The separate TBS Auth bot remains running.
- A Time container restart reached healthy state again with the same nonempty
  SQLite file and named volume. Telegram `getMe` from the TBS Auth container
  confirmed that its configured bot token is accepted; no message was sent.
- The dictionary/details historical fill was not repeated. Daily Details refresh
  is still 24 hours. Original PostgreSQL, Redis, Caddy and unrelated bot containers
  were not recreated.

## Backup and rollback

Server-private backup: `/opt/parcer-backups/20261115-time-5c0361e`, directory mode 700.
It contains a custom PostgreSQL dump, globals, environment, Compose/Caddy/security
configuration, image/container metadata and account comparisons. The dump was
fully decoded with `pg_restore -f /dev/null`, its table/revision list checked,
and file checksums verified. This is **not** a new isolated restore rehearsal;
that rehearsal was performed by the previous session on a disposable database.

Old image `parcer-backend:20261112-36ffa0d` remains available. Reverting environment
and backend images is the first rollback layer; additive schema 0035 is compatible
with 2.0.3. Do not restore the database as an ordinary rollback, which would lose
new receipts. Private execution logs are in
`/opt/parcer-releases/20261115-time-5c0361e`.

## New fixes and verification

- Normalize PostgreSQL-aware launch/chat lock timestamps before UTC comparison;
  tests cover active and expired aware locks.
- Normalize auth-bot Unix timestamps independently of host timezone. A local Windows
  test found chat tokens were already expired on a UTC+5 host; Linux UTC behavior
  stays the same.
- The hosted page suppresses a pointer click without a pointer press in that page,
  while allowing fresh pointer and keyboard activation. Node event-sequence test
  passes. The original Linux xdotool reproduction has not been rerun.
- Closed Time months no longer suggest keyboard editing or quick corrections.
- Backend: **1354 passed, 1 skipped**, Python 3.11 Linux, in
  https://github.com/asintiko/crm-rust/actions/runs/43709882988 . The same run built
  the pinned Time image. Artifact SHA-256:
  `07154f6190de0afdc287adb487f028ebe9b8a896306accdcea51267604b6b35d`.
- Production-image isolated regression checks: 104 passed initially; two route/source
  inspection tests failed because their sibling source folders were not mounted.
  Both passed after mounting the source tree correctly. Tests used SQLite and no
  network; the production database was not a test target.
- Native Windows: **357 app + 139 core tests**, release build and Telegram assets
  build passed. The two live-suite test functions return early without their target
  server; their reported success is not a live integration check.
- **Real Windows / WebView2, synthetic local backend accounts:** signed in; returned
  from Checks to Hub; opened Time; returned without signing in again; opened global
  CRM administration; signed out; switched to a Time-only viewer. Viewer Hub showed
  only Time (no Checks/admin); Time displayed "Только просмотр". Screenshots are
  under `evidence/time-hub-windows-20261115/`.
- The local GUI fixture used the real new backend and Time server, in-memory SQLite
  accounts and fake Redis. This does not establish real Redis/production login or
  Telegram OTP delivery. The previous 173-check PostgreSQL e2e rehearsal was not
  repeated in this session. Dark theme, revocation during an open Windows page,
  and a populated closed month on WebView2 were not exercised in this run.

## Windows installer

Version 2.1.0, built natively on Windows, includes Telegram/Time/admin assets,
WebView2Loader, icon/version resources and the production `client-access.json`.
The production access file was verified against the server without printing it.
Keep this installer private; do not commit or attach it to a public release.

Local artifact: `C:/Dev/projects/parcer/output/time-2.1-complete-installer/TBSparcer-Native-Setup-2.1.0.exe`.
SHA-256: `960a6a8995021e75c74fbde3c4e2f7a1b1b6e1f41a63dbd180f4b483186bbb56`.

This complete installer was compiled but **not installed over the user's running
2.0.3 client**. The previous CI installer installation passed, but lacked the access
file. The update channel is unchanged. Before fleet rollout, verify an actual
2.0.3 upgrade, session preservation and production sign-in with the owner.

## Remaining

1. Owner checks real account sign-in, existing Checks/Excel synchronization, and
   grants Time roles as intended. No production user's credentials were used in GUI tests.
2. Verify installer upgrade on a designated Windows client before changing the update channel.
3. Securely provision the Time bot and verify its work group and identity bindings.
4. Continue the separate activation-code/R2/Checks-AI/copilot workstreams in `OPEN_TASKS.md`.
5. Server disk remains tight (about 1.3 GB free after rollout). No volumes or old
   backups were deleted to make room. Plan targeted housekeeping before larger releases.
