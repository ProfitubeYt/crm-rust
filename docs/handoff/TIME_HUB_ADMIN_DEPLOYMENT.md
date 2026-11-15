# Time workspace, Hub and CRM administration: deployment and rollback

**Approved by the owner on 2026-11-12 on conditions (section 10). Nothing in it has been run on the server.** Production is
release 2.0.3 at schema `20261112_0033` (`DEPLOYMENT_2_0_3_20261112.md`); it was last read,
read-only, on 2026-11-12 at 13:30 UTC. What was tested and how is in
`TIME_HUB_ADMIN_PROGRESS.md`: section "Release session" for the runs of 2026-11-14 on the
code to deploy, section "Finish session" for the earlier ones.

Brought up to the owner's decisions on 2026-11-14: section 2 carries the answers, step B the
read of the second factor, step C the proof that the backups can be read, section 6 the
check that the administrators' sign-in was not touched, section 7 the bot's steps. The
session that wrote this had no access to the server of any kind: the pre-checks of step B
were **not** rerun on production on 2026-11-14 and are the first thing the person who
carries the plan out does.

The person who carries this out needs a root shell on `64.188.106.221`. The shared read-only
key (`SHARED_SSH_ACCESS.md`) cannot do any of it.

## 1. What changes, and what does not

| | Before | After |
|---|---|---|
| Schema | `20261112_0033` | `20261112_0035`: three columns and one check on `users` |
| Backend image (backend, three Celery workers, `auth_bot`) | `parcer-backend:20261112-36ffa0d` | the same image with the new code laid over it |
| Containers | as now | one more, `uzbek_parser_timelist`, with its own volume and no published port |
| Caddy | as now | one more route: `/admin`, `/admin/*` to the backend |
| `.env` | as now | `BACKEND_IMAGE` changed; six lines added |
| Accounts | Checks roles `admin`, `operator` | the same, and the two `admin` accounts are CRM administrators |

Not touched: PostgreSQL and Redis and their volumes, the `frontend` container, Caddy's
certificates, `funpay-bot`, `security/root-access.server.json`, `docker-compose.yml`, every
table but `users`, the dictionary and details batch already applied (17 lines, 656
applications, 59 details; it is not run again), `ANNOTATION_DETAILS_RULES_REFRESH_HOURS=24`
and the sign-in settings. No existing value of any account row is rewritten.

What people see: the installed 2.0.3 client keeps working as it does now. Nobody is signed
out. Nobody gets Time until a CRM administrator grants it. `/admin/` starts to answer. The
Hub entries «Учёт времени» and «Администрирование CRM» appear only in a new build of the
client, which this plan does not roll out (section 7).

Also in the new image: a fix for the sign-in of a locked account (500 instead of «locked»
until an administrator unlocks it; progress record, "Found by the end-to-end run").

## 2. What the owner decided, and what is still to be supplied

Decided (section 10):

1. **The plan is approved on conditions**: the Time UI adaptation finished, the tests, the
   pre-checks on the day and fresh backups proved readable, all before step D.
2. **Both accounts with the Checks role `admin`, `admin` and `stlee`, become and stay CRM
   administrators.** Step B stops the run if `prechecks.sql` lists any other account. Neither
   is demoted in the panel afterwards without the owner.
3. **Their sign-in is not touched.** The deployment writes no password, no second-factor
   secret, no recovery code and no session: migrations 0034 and 0035 add three columns and
   fill them, nothing else. Step B records the second-factor state of both, section 6
   compares it afterwards.
4. **The Time bot stays off** (`TIME_BOT_TOKEN` and `TIME_BOT_CHAT_ID` empty) until its token
   is reissued, privacy mode is disabled and the group's ID is verified (section 7, step 2).
   Read as: Time and the panel may go live before the bot works. **To confirm with the
   owner.**

Still to be supplied, each by the owner or through the owner:

1. **A person with a root shell on the server** to carry the plan out, and **a window**. The
   backend and the workers restart once; expect one to two minutes during which clients show
   the server as unavailable. The shared key is read-only and no sandbox has more than that.
2. **Authentication decision, clarified by the owner on 2026-11-15:** use the existing
   Telegram TBS Auth flow; do not require an authenticator application or enroll TOTP.
   Read-only production checks found both administrators linked to Telegram, with OTP and
   bot control enabled. Preserve their existing sign-in configuration. Telegram scope OTP,
   Telegram QR login and account-password TOTP are distinct mechanisms in the current code;
   these checks do not prove Telegram is a second factor on every password login.
3. **The bot's new token**, given to the person who carries the plan out and to nobody else,
   never by chat: the token of `@tbstimebot` was shown in an earlier chat and counts as
   exposed.
4. **Who builds the Time image** off the server (a Rust release build, about six minutes on
   two cores; the server has about 2.9 GB of memory and must not do it).
5. Later, and not part of the window: the Windows client 2.1.0 (section 7, step 5).

`TIMELIST_API_TOKEN` and `TIME_HUB_KEY` are generated on the server in step D and are never
shown, sent or committed.

## 3. What the plan rests on

Checked on a local stack (PostgreSQL 15.19, Redis 7, Caddy 2, the real Time server), with the
code of `f5eede4` standing in for the server's:

- `alembic upgrade head` from a database built and stamped as 2.0.3: `prechecks.sql` before,
  `postchecks.sql` after, as expected.
- Release 2.0.3 runs on the new schema and on the schema downgraded again; it can create
  accounts on the new schema.
- A session opened under 2.0.3 goes on under the new code, and one opened under the new code
  goes on under 2.0.3: neither the deployment nor an image rollback signs anyone out.
- `deploy/time-hub/docker-compose.time.yml` as it is, on top of a compose file with the
  server's service names; `timelist` becomes healthy with and without a bot token.
- `deploy/Caddyfile` with only its site address replaced: `/admin` reaches the backend,
  `/internal/*` does not; `caddy validate` accepts the file as it is. Step H itself on a
  Caddy with its file mounted the way the server mounts it: written in place, validated,
  reloaded, the route is live; a file put in place by rename is not seen by the container.
- 173 end-to-end checks of access, revocation, sessions, exports and the bot.
- The 2.0.3 client (its Linux build) signed in to the new backend: receipts, settings and
  the user list work.
- The checks of section 6 and the statements of sections 8 and 9, with the stack's names.
- The archive of section 4 and the overlay build of step D, on a stand-in for the server's
  image (the pinned Python with the 2.0.3 code in `/app`): the two revisions, the panel and
  the new modules are in the result.

Taken from the repository and the 2.0.3 record, **not seen** (the read-only key gives no
shell): the project directory `/opt/receipt-parser`, the image name
`parcer-backend:20261112-36ffa0d`, that `.env` carries `BACKEND_IMAGE`, that the live
`docker-compose.yml` and `deploy/Caddyfile` are the repository's, which containers run on the
backend image. Step B confirms each before anything changes; a difference stops the run.

Not checked at all: TLS on the real name, a real Telegram bot and group, the Windows client.

## 4. Artefacts, made off the server

On a machine with this repository at the commit to deploy (`main` after the merge of
`vorflux/time-release`, pull request 1 of `asintiko/crm-rust`; the commit the runs of
"Release session" were made on, or a later one with the same `backend/`, `time/` and
`deploy/`), Docker and network access:

    TAG=$(date +%Y%m%d)-$(git rev-parse --short HEAD)
    git rev-parse HEAD > COMMIT
    git archive --format=tar.gz --prefix=src/ -o time-hub-$TAG.tar.gz HEAD backend deploy docker-compose.yml
    docker build -t timelist:$TAG time
    docker save timelist:$TAG | gzip > timelist-$TAG.tar.gz        # about 39 MB
    sha256sum COMMIT time-hub-$TAG.tar.gz timelist-$TAG.tar.gz > SHA256SUMS

Copy the four files to `/opt/parcer-releases/$TAG/` on the server. No secret is in them.

## 5. On the server

Every block is run from `/opt/receipt-parser` as root. Stop at the first answer that is not
the one written next to the command; section 8 says how to go back from each point.

    cd /opt/receipt-parser
    TAG=<the tag of section 4>
    REL=/opt/parcer-releases/$TAG
    BAK=/opt/parcer-backups/$TAG
    PREV_IMAGE=parcer-backend:20261112-36ffa0d
    HOST=64.188.106.221.nip.io
    PGUSER=$(grep -E '^POSTGRES_USER=' .env | cut -d= -f2)
    PGDB=$(grep -E '^POSTGRES_DB=' .env | cut -d= -f2)
    TIME="-f docker-compose.yml -f deploy/time-hub/docker-compose.time.yml"
    sql() { docker compose exec -T postgres psql -X -U "$PGUSER" -d "$PGDB" -f - < "$1"; }
    q() { docker compose exec -T postgres psql -X -U "$PGUSER" -d "$PGDB" -c "$1"; }

### A. Unpack and verify the artefacts (changes nothing)

    cd $REL && sha256sum -c SHA256SUMS && tar -xzf time-hub-$TAG.tar.gz && cd /opt/receipt-parser

### B. Confirm what this plan assumed (changes nothing)

    docker compose ps                                    # everything up; note what is not healthy now
    docker ps --format '{{.Names}}\t{{.Image}}' | grep parcer-backend
        # the containers to move, all on 20261112-36ffa0d: uzbek_parser_backend, the three celery
        # workers, uzbek_parser_auth_bot. Any other name in this list is moved in step F as well.
    grep -n '^BACKEND_IMAGE=' .env                       # BACKEND_IMAGE=parcer-backend:20261112-36ffa0d
    grep -nE '^(TIMELIST_|TIME_HUB_KEY|TIME_BOT_)' .env  # nothing
    grep -n '^ANNOTATION_DETAILS_RULES_REFRESH_HOURS=' .env   # ...=24
    diff docker-compose.yml $REL/src/docker-compose.yml  # no difference
    diff deploy/Caddyfile $REL/src/deploy/Caddyfile      # only five added lines, the @admin route
    docker compose exec -T backend bash -lc 'cd /app && alembic -c alembic.ini current'   # 20261112_0033
    sql $REL/src/deploy/time-hub/prechecks.sql           # 0033; "(none)"; the accounts by role; no stray role
    ADMINS="SELECT id, username, role, is_active, totp_enabled, force_2fa, totp_confirmed_at IS NOT NULL AS totp_confirmed, backup_codes IS NOT NULL AS has_backup_codes, md5(coalesce(password_hash,'') || coalesce(salt,'') || coalesce(totp_secret,'') || coalesce(backup_codes::text,'')) AS sign_in_mark, locked_until, failed_attempts FROM users WHERE lower(role) = 'admin' ORDER BY id"
    q "$ADMINS"                                          # the accounts prechecks.sql listed: admin and stlee, both active
    q "SELECT count(*) AS accounts, count(telegram_id) AS with_telegram_id FROM users"
    df -h / /var/lib/docker; free -m                     # room for a 40 MB image and a dump

The accounts `prechecks.sql` lists under "0035 marks exactly these accounts" are `admin` and
`stlee`, the two the owner agreed to (section 2). If the list is different, stop.

`sign_in_mark` is a fingerprint of what an account signs in with; it shows a change without
showing a secret. It still must not leave the server: keep the answer in `$BAK` (step C),
not in a chat, a ticket or git. A false `totp_enabled` is not a deployment blocker under
the owner's 2026-11-15 Telegram-only decision. Do not enable TOTP or change existing
authentication flags during deployment. If `locked_until` is in the future, stop and
take it to the owner (section 2).

### C. Backups

    mkdir -p $BAK && chmod 700 $BAK
    docker compose exec -T postgres pg_dump -U "$PGUSER" -d "$PGDB" -Fc > $BAK/before.dump
    docker compose exec -T postgres pg_dumpall -U "$PGUSER" --globals-only > $BAK/globals.sql
    cp -a .env $BAK/env.before
    cp -a deploy/Caddyfile $BAK/Caddyfile.before
    cp -a docker-compose.yml $BAK/docker-compose.yml.before
    docker inspect $(docker compose ps -q) > $BAK/containers.before.json
    docker image inspect $PREV_IMAGE --format '{{.Id}}' > $BAK/image.before
    q "$ADMINS" > $BAK/admins.before
    q "SELECT count(*) FROM users" > $BAK/users.count.before

Proof that the dump can be read, before anything is changed:

    ls -l $BAK/before.dump                               # tens of megabytes at least, not zero
    docker compose exec -T postgres pg_restore --list < $BAK/before.dump > $BAK/before.list
    wc -l < $BAK/before.list                             # some hundreds of lines
    grep -c 'TABLE DATA public users' $BAK/before.list   # 1
    grep -c 'TABLE DATA public alembic_version' $BAK/before.list    # 1
    docker compose exec -T postgres pg_restore -f - --data-only -t alembic_version < $BAK/before.dump | grep -c 20261112_0033   # 1
    docker compose exec -T postgres pg_restore -f - --data-only -t users < $BAK/before.dump | grep -c $'^[0-9]\+\t'   # the number in users.count.before
    grep -c 'ROLE' $BAK/globals.sql                      # 1 or more
    (cd $BAK && sha256sum before.dump globals.sql env.before Caddyfile.before docker-compose.yml.before > SHA256SUMS && chmod 600 *)

`pg_restore -f -` writes SQL to its output and restores nothing. Any answer other than the
one written: stop, nothing has been changed yet. The condition of the approval is a backup
that was read back, not one that was only written.

The backups stay on the server in `$BAK` (mode 700, root). They hold the whole database and
`.env`: they are not copied into a chat, a ticket, git or a sandbox. A second copy off the
server, if the owner wants one, goes the way the owner's other backups go.

The dump is for a disaster, not for rollback: restoring it would lose every receipt taken
after it.

### D. The new image and `.env`

    docker build -f $REL/src/backend/Dockerfile.overlay --build-arg BASE_IMAGE=$PREV_IMAGE \
        -t parcer-backend:$TAG $REL/src/backend
    docker run --rm --entrypoint ls parcer-backend:$TAG /app/alembic/versions | tail -n 2
        # 20261112_0034_workspace_access.py, 20261112_0035_global_admin.py
    docker load < $REL/timelist-$TAG.tar.gz              # Loaded image: timelist:<tag>

    sed -i "s|^BACKEND_IMAGE=.*|BACKEND_IMAGE=parcer-backend:$TAG|" .env
    {
      echo "TIMELIST_IMAGE=timelist:$TAG"
      echo "TIMELIST_INTERNAL_URL=http://timelist:8080"
      echo "TIMELIST_API_TOKEN=$(openssl rand -hex 32)"
      echo "TIME_HUB_KEY=$(openssl rand -hex 24)"
      echo "TIME_BOT_TOKEN="
      echo "TIME_BOT_CHAT_ID="
    } >> .env
    diff <(sed 's/=.*//' $BAK/env.before) <(sed 's/=.*//' .env)     # six names added, none removed
    grep -n '^ANNOTATION_DETAILS_RULES_REFRESH_HOURS=' .env          # still 24

Nothing running has changed yet: containers read `.env` only when they are created.

### E. The migration

The old backend keeps running during it; it is compatible with the new columns.

    docker compose run --rm --no-deps backend bash -lc 'cd /app && alembic -c alembic.ini upgrade head'
        # Running upgrade 20261112_0033 -> 20261112_0034, then -> 20261112_0035
    sql $REL/src/deploy/time-hub/postchecks.sql | tee $BAK/postchecks.out

`postchecks.sql` says what each answer must be: revision 0035, the three columns, the check,
nobody without Checks, nobody with Time, CRM administrators exactly the `admin` accounts, at
least one of them active. Anything else: section 8, R4, then stop.

### F. The backend and the workers

    docker compose up -d --no-deps backend celery_worker celery_ocr_worker celery_maintenance_worker auth_bot
    docker ps --format '{{.Names}}\t{{.Image}}' | grep parcer-backend    # the same names, all on the new tag
    curl -fsS http://127.0.0.1:8000/health                           # "status":"healthy","database":"connected"
    docker compose exec -T celery_maintenance_worker printenv ANNOTATION_DETAILS_RULES_REFRESH_HOURS   # 24
    docker compose ps                                                # the workers healthy again

### G. Time

    install -D -m 644 $REL/src/deploy/time-hub/docker-compose.time.yml deploy/time-hub/docker-compose.time.yml
    docker compose $TIME up -d --no-deps timelist
    sleep 45; docker inspect uzbek_parser_timelist --format '{{.State.Health.Status}}'   # healthy
    docker port uzbek_parser_timelist                                # nothing: no port is published
    docker logs uzbek_parser_timelist 2>&1 | tail -n 5               # shared accounts mode; the bot is off
    docker compose exec -T backend python -c "import os,urllib.request as u; print(u.urlopen(u.Request(os.environ['TIMELIST_INTERNAL_URL']+'/api/month/2026-11', headers={'Authorization':'Bearer '+os.environ['TIMELIST_API_TOKEN']})).status)"
        # 200: the backend reaches Time with the token Time expects

From now on every compose command that concerns `timelist` takes both files (`$TIME`). A
command with `docker-compose.yml` alone leaves `timelist` as it is and warns about an orphan;
**never add `--remove-orphans`**. And name the services in every `up`: a bare
`docker compose up -d` would recreate every container that reads `.env`, because the file
has changed.

### H. The Caddy route

    cat $REL/src/deploy/Caddyfile > deploy/Caddyfile     # written in place: the container follows the file, not the name
    docker compose exec -T caddy grep -c '@admin' /etc/caddy/Caddyfile            # 2
    docker compose exec -T caddy caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile   # Valid configuration
    docker compose exec -T caddy caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile

If the `grep` answers 0, the container still holds the old file: `docker compose up -d
--no-deps --force-recreate caddy` (HTTPS is away for a few seconds; the certificates are in
the `caddy_data` volume and are not asked for again).

## 6. Health checks

    curl -fsS https://$HOST/health                                             # healthy, connected
    curl -sS -o /dev/null -w '%{http_code} %{redirect_url}\n' https://$HOST/admin        # 308 https://<host>/admin/
    curl -sS -D - -o /dev/null https://$HOST/admin/ | grep -i '^content-security-policy'   # default-src 'none'; ... frame-ancestors 'none'
    curl -sS -o /dev/null -w '%{http_code}\n' https://$HOST/api/time/meta      # 401: the gateway is there and wants a session
    curl -sS https://$HOST/internal/time/telegram/1 | grep -c 'time_hub_key\|allowed'    # 0: not the backend's answer
    curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8000/internal/time/telegram/1   # 403: refused without the key
    docker compose exec -T backend bash -lc 'cd /app && alembic -c alembic.ini current'   # 20261112_0035 (head)
    docker compose logs --since 15m backend | grep -c ' -> 500 '               # 0
    q "$ADMINS" | diff $BAK/admins.before -                                    # no difference: sign-in of both untouched
    q "SELECT username, global_admin, checks_access, time_role FROM users WHERE lower(role) = 'admin' ORDER BY id"
        # admin and stlee: global_admin t, checks_access t, time_role empty
    q "SELECT count(*) FROM users" | diff $BAK/users.count.before -            # no difference

A difference in `locked_until` or `failed_attempts` alone means somebody mistyped a password
during the window, not that the deployment changed an account; any other difference is a
reason to stop and look before anything of section 7.

By hand, by the owner or with the owner:

- The installed 2.0.3 client, still signed in from before: receipts load, a new receipt
  arrives, Settings → Пользователи lists the accounts.
- `https://<host>/admin/`: **each** of the two administrators signs in, with the password and
  the second factor they had before; the list shows both as «Администратор CRM» and nobody
  with «Учёт времени». If one of them cannot, the other still can; section 9 is for the case
  that neither can.
- Excel synchronisation of one workbook, as after any release.

## 7. After the window

In this order, none of it urgent:

1. In `/admin/`, keep CRM administration for the accounts that should have it.
2. The bot `@tbstimebot`, in this order. Until the last step it accepts no mark from anyone,
   and Time and the panel work without it.

   a. The owner, in @BotFather: `/revoke` for `@tbstimebot` (the old token was shown in a chat
      and stops working here), then `/setprivacy` → `@tbstimebot` → Disable. The new token
      goes to the person who carries the plan out, into the server's `.env`, and nowhere
      else: not into a chat, a prompt, a ticket or git. The owner's own copy is replaced in
      the owner's protected file.
   b. The owner removes the bot from the work group and adds it again: a group keeps the
      privacy setting the bot had when it joined.
   c. On the server, the token alone, the group still empty:

          sed -i "s|^TIME_BOT_TOKEN=.*|TIME_BOT_TOKEN=<the new token>|" .env    # typed at the server, not pasted from a chat
          docker compose $TIME up -d --no-deps timelist
          docker logs uzbek_parser_timelist 2>&1 | tail -n 5

      The log names the bot and says that no group is set: it answers `/id` and `/help` and
      takes no marks.
   d. Somebody sends `/id` **in the work group**. The bot answers with the group's ID, a
      negative number. That answer is the only source of `TIME_BOT_CHAT_ID`: the number
      `1276772106` noted earlier was refused by Telegram («chat not found») and no prefix is
      to be guessed for it. No answer means the bot is not in that group or the token is not
      the new one; do not go on. An answer does not yet prove that privacy mode is off: a
      bot is shown commands either way, and step f is what shows that it reads the rest.
   e. `sed -i "s|^TIME_BOT_CHAT_ID=.*|TIME_BOT_CHAT_ID=<the negative ID>|" .env`, the same
      `up` again; the log names the bot and the group. One group, the verified one, no list
      of others.
   f. After step 3 below: a mark from an account granted Time is accepted; a made-up message
      from somebody without an account gets 🤷; the same message in any other group or in a
      private chat with the bot is not taken as a mark.

   If the bot is blocked at any of these, say so on its own line in the report: a bot that
   is off is not a Time service that is down.
3. Grant «Учёт времени» to the people who need it (viewer, manager, or employee for those
   who only mark attendance) and enter their Telegram IDs, in `/admin/`.
4. In Time, add the employees and bind each Telegram sender to an employee.
5. The Windows client 2.1.0 with the Hub entries: built and installed separately
   (`TIME_HUB_ADMIN_PROGRESS.md`, "Release session", says what was built and what was not
   run on Windows). It is published to the clients' update channel only after somebody has
   run it on a Windows desktop against this server: publishing offers it to every installed
   client. Until then Time has no interface for anyone, because its page lives in the
   client; the bot and the panel do not depend on it.

## 8. Rollback

Each layer goes back on its own, in the reverse order of section 5. The database is not
restored in any of them.

**R1. Time off.** `docker compose $TIME stop timelist`. Its data stays in the volume. Accounts
granted Time still see the entry; their page does not load (the gateway answers 502) and the
bot is silent. To remove the container as well: `docker compose $TIME rm -sf timelist` (the
volume stays).

**R2. The Caddy route.** `cat $BAK/Caddyfile.before > deploy/Caddyfile`, then the `validate`
and `reload` of step H. `/admin` goes to the frontend again, as before.

**R3. The backend image.**

    sed -i "s|^BACKEND_IMAGE=.*|BACKEND_IMAGE=$PREV_IMAGE|" .env
    docker compose up -d --no-deps backend celery_worker celery_ocr_worker celery_maintenance_worker auth_bot
    curl -fsS http://127.0.0.1:8000/health

The schema stays at 0035 and 2.0.3 runs on it (rehearsed). Sessions stay (rehearsed). The
six new lines of `.env` mean nothing to 2.0.3 and may stay. Two things to know while 2.0.3
runs on the new schema:

- Its `alembic` does not know revision 0035 and fails on any command; nothing starts it.
- It does not keep `global_admin`: an administrator it creates is not a CRM administrator
  for the new code, and one it demotes still is. **Before the new image is started again**:

      q "SELECT id, username, role, is_active, global_admin FROM users WHERE global_admin <> (lower(role) = 'admin') ORDER BY id"

  and settle each row with the owner (`q "UPDATE users SET global_admin = … WHERE id = …"`).
  After a first deployment with nothing changed in the panel, the query returns no rows.

**R4. The schema**, only if the columns themselves have to go; it needs the new image:

    BACKEND_IMAGE=parcer-backend:$TAG docker compose run --rm --no-deps backend \
        bash -lc 'cd /app && alembic -c alembic.ini downgrade 20261112_0033'
    sql $REL/src/deploy/time-hub/prechecks.sql           # 0033, "(none)"

This forgets who was granted Time, who was taken off Checks and who was made or unmade a CRM
administrator; with 2.0.3 that right is read from the Checks role again. Run it after R3,
never while the new backend is up.

**Grants without touching code or schema.** Everything granted after the deployment can be
taken back where it was given, in `/admin/`. In one statement, if the panel is not at hand:
`q "UPDATE users SET time_role = NULL"` closes Time for everyone at their next request.

**Clients.** The 2.0.3 client goes on working against the new server (seen in a window on
Linux, receipts and the user list). A new client against a 2.0.3 server shows Checks and
nothing else: it offers Time and administration only when the server says so (unit tests;
not run in a window).

**What is never a rollback:** restoring `before.dump`. It would lose the receipts, the
annotations and the attendance recorded since.

## 9. If nobody can administer

- An account locked by wrong passwords (five in a row):
  `q "UPDATE users SET locked_until = NULL, failed_attempts = 0 WHERE username = '<name>'"`
  With release 2.0.3 this is the only way out when no other administrator can sign in.
- No CRM administrator left who can sign in:
  `docker compose exec backend python scripts/create_admin.py --promote <username>` makes an
  existing active account one; its password and everything else stay.

## 10. Approval

**Approved by the owner on 2026-11-12, on conditions; not executed.** The owner approved this
plan for production once the Time UI adaptation is finished and after the preflight checks,
the tests and fresh verified backups. The owner also decided that both accounts `admin` and `stlee`
keep CRM administration. As the plan already has it, no credential is reset, nobody is signed out, Time is granted to
nobody by the deployment. The limits on the group are read as: the Time bot stays off (`TIME_BOT_TOKEN` and `TIME_BOT_CHAT_ID`
empty in `.env`) until the group's ID is verified and privacy mode is disabled.

State on 2026-11-14: the UI adaptation is finished and the tests were run again on the code
to deploy (`TIME_HUB_ADMIN_PROGRESS.md`, "Release session"); the sections above carry the
owner's answers. Still open before step C may be taken, none of which a sandbox can close:
who carries the plan out (the shared key is read-only), the window, the pre-checks of step B
on the day, and what step B shows of the administrators' second factor (section 2).
