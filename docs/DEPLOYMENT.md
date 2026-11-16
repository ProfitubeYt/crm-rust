# Production deployment and rollback

Recorded states carry their date. Nothing here was re-read from the server after
2026-11-15: verify the live state through authorized access before acting on it.
No secret value belongs in this file or anywhere else in git.

## What runs in production (last recorded 2026-11-15)

- One server, published as `64.188.106.221.nip.io` (see `deploy/Caddyfile`).
  Project directory `/opt/receipt-parser`, started with Docker Compose from the
  root `docker-compose.yml`. Container names begin with `uzbek_parser_`.
- Services of `docker-compose.yml`: `postgres` (15), `redis` (7), `backend`
  (FastAPI), `userbot`, `telegram_bot`, `auth_bot` (TBS Auth), `celery_worker`,
  `celery_ocr_worker`, `celery_maintenance_worker`, `caddy`. They share one
  backend image selected by `BACKEND_IMAGE` in the server's `.env`.
- Backend, the three Celery workers and `auth_bot` ran
  `parcer-backend:20261115-time-5c0361e`. The previous image
  `parcer-backend:20261112-36ffa0d` (release 2.0.3) was kept on the server.
- Schema: Alembic `20261112_0035`. Migrations 0034/0035 added three columns and one
  check to `users` (`checks_access`, `time_role`, `global_admin`); nothing else.
- Time: container `uzbek_parser_timelist`, image
  `timelist:5c0361eb4b293ac354c2a2ffd89e1a140e780e0c`, defined by
  `deploy/time-hub/docker-compose.time.yml` on top of the main file. Its whole
  data is one SQLite file in the named volume `timelist_data` (`/data`). No port
  is published: only the backend gateway `/api/time/*` reaches it, with a private
  token, over the compose network. The SQLite file survived a container restart.
- The Time Telegram bot is **off** (`TIME_BOT_TOKEN`, `TIME_BOT_CHAT_ID` empty).
  The separate TBS Auth bot runs.
- An unrelated bot container (`funpay-bot`) lives on the same server and is never
  touched by a CRM rollout.
- Server `.env` gained six names on 2026-11-15: `TIMELIST_IMAGE`,
  `TIMELIST_INTERNAL_URL`, `TIMELIST_API_TOKEN`, `TIME_HUB_KEY`, `TIME_BOT_TOKEN`,
  `TIME_BOT_CHAT_ID`. The two keys were generated on the server and are not known
  outside it. `ANNOTATION_DETAILS_RULES_REFRESH_HOURS=24` must stay.
- Disk was tight: about 1.3 GB free after the rollout. The server has about 2.9 GB
  of memory and must not compile Rust.

### Routes (Caddy)

`deploy/Caddyfile` routes `/mini-app*` and `/api/*` to the backend with the
`X-System-Access` header added from the server's environment, `/health` and
`/admin`, `/admin/*` to the backend, and never `/internal/*`.

**The server's Caddyfile is not the repository's.** On 2026-11-15 the live file was
kept byte-for-byte: `/admin/` already answered through its existing fallback route,
so the `@admin` route was not installed. The 2026-11-12 plan also lists a
`frontend` container (the retired web client) among the things left untouched on
the server. The repository's `docker-compose.yml` and `deploy/Caddyfile` have
since dropped the frontend; those edits are **not deployed**. The next rollout
must diff both files against the server and decide on the difference deliberately.

Public answers recorded on 2026-11-15: `/health` healthy with the database
connected, `/admin/` 200 over HTTPS, `/api/time/meta` 401 without a session.

### Data already applied — never repeat

Release 2.0.3 (2026-11-12) applied one reviewed dictionary/details batch exactly
once: 17 operator-reference lines, 656 Application fills, 59 Details fills
(batch `dict-details-20261112-36ffa0d`). It is not part of any later rollout. The
daily refresh of confirmed rules runs every 24 hours.

## Who can deploy

- **Read-only shared access** (`hoplite-ro` OS account, `hoplite_ro` PostgreSQL
  role): key-only SSH with a forced command and one permitted local tunnel to the
  database; SELECT-only on non-secret columns, 30-second statement timeout, four
  connections. No shell, no Docker, no deployment. The key and the role expire on
  **2026-11-25 00:00 UTC** unless the administrator renews them. The private key is
  held by the owner and reaches a sandbox only through a protected file mount
  (`HOPLITE_SSH_KEY_PATH` holds the path, never the key). Pin the server's host
  key before connecting. `deploy/time-hub/prechecks.sql` shows the `psql` call.
- **Deployment** needs an administrator with a root shell on the server, an agreed
  window and the owner's approval of the concrete plan. Sandboxes and CI have no
  such access.
- Revoking the shared key: the administrator backs up
  `/home/hoplite-ro/.ssh/authorized_keys` and removes only the matching line;
  dropping the `hoplite_ro` role cuts every client that uses it. Do not put
  `permitlisten="none"` into `authorized_keys`: it broke authentication on this
  server's OpenSSH 8.9.

## Building the artefacts off the server

- **Time image:** `.github/workflows/time-image.yml` (started by hand). It runs the
  backend test suite, builds `timelist:<commit>` from `time/Dockerfile` and keeps
  `timelist.tar.gz`, `COMMIT` and `SHA256SUMS` as the artifact
  `time-deployment-image` for seven days. Locally: `docker build -t timelist:<tag> time`.
- **Backend source:** `git archive --format=tar.gz --prefix=src/ -o time-hub-<tag>.tar.gz HEAD backend deploy docker-compose.yml`.
  On the server the new backend image is an overlay of the running one:
  `docker build -f src/backend/Dockerfile.overlay --build-arg BASE_IMAGE=<previous image> -t parcer-backend:<tag> src/backend`
  (dependencies unchanged, code replaced).
- Copy the archive, the image, `COMMIT` and `SHA256SUMS` to
  `/opt/parcer-releases/<tag>/` and check the sums there. None of them holds a secret.
- **Windows client:** `desktop/README.md`. A setup that contains
  `client-access.json` is private: never commit it, attach it to a public release
  or leave it in a CI artifact.

## Rollout procedure (as used on 2026-11-15)

Run as root from `/opt/receipt-parser`. Stop at the first unexpected answer.

1. **Verify the artefacts:** `sha256sum -c SHA256SUMS`, unpack the source archive.
2. **Preflight, changes nothing:** `docker compose ps`; which containers run the
   backend image; `BACKEND_IMAGE` in `.env`; diff of the live `docker-compose.yml`
   and `deploy/Caddyfile` against the release; `alembic current` in the backend
   container; `deploy/time-hub/prechecks.sql` (read-only; expects the schema
   revision and lists the accounts that hold administration); the administrators'
   rows with a fingerprint of their sign-in data (kept on the server only); account
   count; `df -h` and `free -m`. Stop if an administrator is locked or the list of
   administrators is not the one the owner approved.
3. **Backup** into `/opt/parcer-backups/<tag>` (mode 700): custom-format
   `pg_dump`, `pg_dumpall --globals-only`, `.env`, Caddyfile, compose file,
   container and image metadata, the administrators' rows, the account count.
   **Prove the dump is readable** before changing anything (`pg_restore --list`,
   `pg_restore -f /dev/null`, the expected revision and row count inside it), then
   write checksums and restrict the files to mode 600.
4. **Image and `.env`:** build the overlay image, load the Time image, set
   `BACKEND_IMAGE`, add any new names. Compare the names in `.env` before and
   after. Running containers do not change yet.
5. **Migration**, with the old backend still running:
   `docker compose run --rm --no-deps backend bash -lc 'cd /app && alembic -c alembic.ini upgrade head'`,
   then `deploy/time-hub/postchecks.sql` (each answer states what it must be).
6. **Backend and workers:**
   `docker compose up -d --no-deps backend celery_worker celery_ocr_worker celery_maintenance_worker auth_bot`;
   `curl http://127.0.0.1:8000/health`; workers healthy. Expect one to two minutes
   during which clients show the server as unavailable.
7. **Time:** `docker compose -f docker-compose.yml -f deploy/time-hub/docker-compose.time.yml up -d --no-deps timelist`;
   health `healthy`; `docker port` shows nothing; the backend reaches Time with its token.
8. **Caddy**, only if its file changes: write the file **in place** (the container
   follows the file, not the name; a file put in place by rename is not seen),
   `caddy validate`, `caddy reload`.
9. **Postchecks:** `/health` over HTTPS; `/admin/` answers; `/api/time/meta` is 401
   without a session; `/internal/time/telegram/1` is not reachable from outside and
   is 403 on loopback without the key; `alembic current`; no ` -> 500 ` in the
   backend log of the window; the administrators' rows and the account count equal
   the backup's.

Rules that cost someone time before:

- Always name the services in `up`. A bare `docker compose up -d` recreates every
  container that reads the changed `.env`.
- Every command about `timelist` takes both compose files. A command with the main
  file alone warns about an orphan: **never add `--remove-orphans`**.
- PostgreSQL, Redis, Caddy's data and unrelated containers and volumes are not recreated.
- The deployment writes no password, second-factor secret or session, signs nobody
  out and grants Time to nobody. Roles are granted afterwards in `/admin/`.
- Private execution logs go to `/opt/parcer-releases/<tag>` on the server, not to git.

## Backups on the server

| Rollout | Backup directory | Contents |
|---|---|---|
| 2026-11-15, Time/Hub/administration | `/opt/parcer-backups/20261115-time-5c0361e` | Dump (fully decoded with `pg_restore -f /dev/null`), globals, environment, Compose/Caddy/security configuration, image and container metadata, account comparisons. Logs: `/opt/parcer-releases/20261115-time-5c0361e`. |
| 2026-11-12, release 2.0.3 and the data batch | `/opt/parcer-backups/20261112-36ffa0d` | Database, configuration and source backups, the pre-batch dump, the batch manifest and journal, rule snapshots before/after, `rollback.sh`. |

Both are root-only and stay on the server. The 2026-11-15 dump was read back, not
restored into an isolated database in that session; an isolated restore was
rehearsed earlier on a disposable database. There is no off-server backup
generation yet (see `OPEN_TASKS.md`, R2).

## Rollback

Each layer goes back on its own, in reverse order. **Restoring the database dump is
never a routine rollback:** it loses every receipt, annotation and attendance mark
recorded since.

1. **Time off:** `docker compose -f docker-compose.yml -f deploy/time-hub/docker-compose.time.yml stop timelist`.
   The volume stays. Accounts granted Time get 502 from the gateway.
2. **Caddy:** write the backed-up Caddyfile in place, validate, reload.
3. **Backend image:** set `BACKEND_IMAGE` back to the previous image and run the
   `up -d --no-deps …` of step 6. Schema 0035 is additive and release 2.0.3 runs on
   it; sessions survive in both directions (rehearsed locally). While 2.0.3 runs on
   the new schema its `alembic` fails on any command, and it does not maintain
   `global_admin`: before starting the new image again, list the accounts where
   `global_admin <> (lower(role) = 'admin')` and settle each with the owner.
4. **Schema**, only if the columns must go, after step 3 and with the new image:
   `alembic downgrade 20261112_0033`, then `prechecks.sql`. This forgets every Time
   grant and every change of CRM administration.

Grants can be taken back without code or schema: in `/admin/`, or
`UPDATE users SET time_role = NULL` closes Time for everyone at their next request.

Release 2.0.3 itself: `/opt/parcer-backups/20261112-36ffa0d/rollback.sh` restores
the image, source and configuration before 2.0.3. It does **not** undo the data
batch. Undoing the batch means stopping the periodic rule refresh first, then
`apply_change_manifest.py rollback --journal <journal>` with the private backup
directory mounted; it reverts only journaled fields that still hold the batch's
value and keeps later edits. The rule recompute of that day is restored separately
from its before/after snapshots.

## Recovery when nobody can administer

- An account locked by wrong passwords:
  `UPDATE users SET locked_until = NULL, failed_attempts = 0 WHERE username = '<name>'`.
- No CRM administrator can sign in:
  `docker compose exec backend python scripts/create_admin.py --promote <username>`
  promotes an existing active account and keeps its password. The last active CRM
  administrator cannot be removed through the API.

## Clients and the update channel

- Installed clients are release 2.0.3; they keep working against the current server.
- Client 2.1.0 (Hub entries for Time and CRM administration) was built natively on
  Windows on 2026-11-15 and kept private on the owner's machine. It was **not**
  installed over a running 2.0.3 client and **not** published.
- The client reads updates from releases of `asintiko/parcer20-updates` and takes
  only an asset named `TBSparcer-Native-Setup-*.exe`. Publishing there offers the
  setup to every installed client.

## Open owner actions (2026-11-15)

1. Sign in with real accounts, check Checks and Excel synchronization, grant Time
   roles in `/admin/`. No production account was used in any test.
2. Try the 2.0.3 → 2.1.0 upgrade on one designated Windows machine (session kept,
   production sign-in) before anything is published to the update channel.
3. Time bot: reissue its token in @BotFather (the old one was shown in a chat and
   counts as exposed), disable privacy mode, remove and re-add the bot to the work
   group, set the token on the server, read the group's ID from the bot's `/id`
   answer **in that group** (a negative number; never guess it or accept every
   group), set it, then check a mark from a granted account and a refusal for
   everyone else.
4. Targeted disk housekeeping before a larger release; no volume or backup is to be
   deleted blindly.
5. Renew or retire the read-only access that expires on 2026-11-25.
