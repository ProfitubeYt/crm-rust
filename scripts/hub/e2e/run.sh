#!/bin/sh
# The disposable stack for end-to-end checks of the Hub, and the rehearsal of
# migrations 0034 and 0035 on its PostgreSQL 15. Nothing here reaches the
# server, Telegram or the internet: see compose.yml.
#
#   scripts/hub/e2e/run.sh            everything below in order, then `down`
#   scripts/hub/e2e/run.sh up         PostgreSQL and Redis, the two config files, the schema of
#                                     release 2.0.3 with accounts in it
#   scripts/hub/e2e/run.sh rehearse   prechecks, 2.0.3 on its schema, upgrade, postchecks, 2.0.3 on
#                                     the new schema, downgrade, 2.0.3 again, upgrade, postchecks
#   scripts/hub/e2e/run.sh start      the stack as it would be deployed: backend, Time, Caddy and
#                                     the stand-in for Telegram; Caddy on 127.0.0.1:18080
#   scripts/hub/e2e/run.sh checks     checks.py against it
#   scripts/hub/e2e/run.sh down       containers, network and data removed
#   scripts/hub/e2e/run.sh compose …  docker compose with the stack's files, for a look inside
#
# Needs Docker with compose and the image timelist:local (docker build -t
# timelist:local time, about eight minutes). The backend's pinned environment
# is the volume hub-backend-venv; scripts/hub/backend-tests.sh makes it.
# Release 2.0.3 is taken from git into output/hub-e2e/, which git ignores.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
work="$root/output/hub-e2e"
legacy_commit="f5eede4"
legacy="$work/release-2.0.3/backend"

# The image pinned in backend/Dockerfile, as in scripts/hub/backend-tests.sh.
export E2E_PYTHON_IMAGE="docker.io/library/python:3.11-slim@sha256:0b23cfb7425d065008b778022a17b1551c82f8b4866ee5a7a200084b7e2eafbf"
E2E_UID="$(id -u)"
E2E_GID="$(id -g)"
export E2E_UID E2E_GID

compose() {
    docker compose --project-directory "$root" --env-file "$here/e2e.env" \
        -f "$here/compose.yml" -f "$root/deploy/time-hub/docker-compose.time.yml" "$@"
}

# One command in the backend's environment, with the current code at /app.
tool() { compose run --rm -T tool "$@"; }

# A file of SQL through psql inside the database's own container.
sql() { compose exec -T postgres psql -X -U hub -d hub_e2e -f - < "$1"; }

say() { printf '\n### %s\n' "$*"; }

# The backend from the code in $1, replacing whichever one is running.
backend_from() {
    E2E_BACKEND_DIR="$1" compose up -d --wait --force-recreate backend >/dev/null 2>&1 || {
        compose logs --tail 60 backend
        echo "the backend did not start" >&2
        exit 1
    }
}

backend_off() { compose rm -sf backend >/dev/null 2>&1; }

up() {
    docker image inspect timelist:local >/dev/null 2>&1 || {
        echo "the Time image is missing: docker build -t timelist:local time" >&2
        exit 1
    }
    docker run --rm -v hub-backend-venv:/venv "$E2E_PYTHON_IMAGE" test -x /venv/bin/pytest \
        || "$root/scripts/hub/backend-tests.sh" --version
    rm -rf "$work/release-2.0.3"
    mkdir -p "$work/release-2.0.3"
    git -C "$root" archive "$legacy_commit" backend | tar -x -C "$work/release-2.0.3"
    say "PostgreSQL 15 and Redis"
    compose up -d --wait postgres redis
    say "the system-access file and the Caddyfile"
    compose run --rm -T --user 0:0 tool /e2e/prepare.py config
    say "the schema of release 2.0.3 ($legacy_commit), accounts, revision"
    E2E_BACKEND_DIR="$legacy" compose run --rm -T tool /e2e/prepare.py legacy-schema
}

rehearse() {
    say "1. before: deploy/time-hub/prechecks.sql"
    sql "$root/deploy/time-hub/prechecks.sql"
    say "2. release 2.0.3 on its own schema"
    backend_from "$legacy"
    tool /e2e/checks.py legacy
    backend_off
    say "3. alembic upgrade head"
    tool -m alembic upgrade head
    say "4. after: deploy/time-hub/postchecks.sql"
    sql "$root/deploy/time-hub/postchecks.sql"
    say "5. release 2.0.3 on the new schema (what a rollback of the image alone runs on)"
    backend_from "$legacy"
    tool /e2e/checks.py legacy-writes
    backend_off
    say "   the accounts now; the administrator 2.0.3 created is not a CRM administrator"
    compose exec -T postgres psql -X -U hub -d hub_e2e -c \
        "SELECT id, username, role, is_active, checks_access, time_role, global_admin FROM users ORDER BY id"
    say "6. alembic downgrade 20261112_0033, then prechecks.sql again"
    tool -m alembic downgrade 20261112_0033
    sql "$root/deploy/time-hub/prechecks.sql"
    say "7. release 2.0.3 on the downgraded schema"
    backend_from "$legacy"
    tool /e2e/checks.py legacy
    backend_off
    say "8. alembic upgrade head again, then postchecks.sql"
    tool -m alembic upgrade head
    sql "$root/deploy/time-hub/postchecks.sql"
}

start() {
    say "the stack as deployed: current backend, Time, Caddy, the stand-in for Telegram"
    compose up -d --wait --force-recreate backend timelist faketg caddy >/dev/null 2>&1 || {
        compose ps -a
        compose logs --tail 40 backend timelist caddy
        echo "the stack did not start" >&2
        exit 1
    }
    # Only Caddy may show a published port.
    docker ps --filter label=com.docker.compose.project=hub-e2e --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
}

checks() {
    say "checks.py"
    tool /e2e/checks.py "$@"
}

down() {
    compose --profile tool down -v --remove-orphans
}

case "${1:-all}" in
    up) up ;;
    rehearse) rehearse ;;
    start) start ;;
    checks) shift; checks "$@" ;;
    down) down ;;
    compose) shift; compose "$@" ;;
    all)
        up
        rehearse
        start
        checks || { echo "checks failed; the stack is left running, '$0 down' removes it" >&2; exit 1; }
        down
        ;;
    *) sed -n '2,20p' "$0"; exit 2 ;;
esac
