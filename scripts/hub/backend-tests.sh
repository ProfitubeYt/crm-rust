#!/bin/sh
# Runs the backend tests the way the server runs its code: Python 3.11 from the
# image pinned in backend/Dockerfile and the pinned requirements, on SQLite in
# memory with dummy secrets. No network service is needed or reached.
#
#   scripts/hub/backend-tests.sh                      the whole suite
#   scripts/hub/backend-tests.sh tests/test_workspace_access.py -q
#
# The dependencies are installed once into the Docker volume hub-backend-venv.
set -eu

root="$(cd "$(dirname "$0")/../.." && pwd)"
image="docker.io/library/python:3.11-slim@sha256:0b23cfb7425d065008b778022a17b1551c82f8b4866ee5a7a200084b7e2eafbf"
volume="hub-backend-venv"

if ! docker run --rm -v "$volume":/venv "$image" test -x /venv/bin/pytest; then
    docker run --rm -v "$root/backend/requirements.txt":/requirements.txt:ro -v "$volume":/venv "$image" \
        sh -c 'python -m venv /venv && /venv/bin/pip install --no-cache-dir -q -r /requirements.txt && chown -R 1000:1000 /venv'
fi

[ "$#" -gt 0 ] || set -- tests -q

exec docker run --rm -u "$(id -u):$(id -g)" \
    -e HOME=/tmp -e PYTHONDONTWRITEBYTECODE=1 \
    -e DATABASE_URL=sqlite:// \
    -e JWT_SECRET=local-test-only-secret-0123456789abcdef0123456789 \
    -e TELEGRAM_API_ID=1 -e TELEGRAM_API_HASH=local-test-only \
    -v "$root":/src -v "$volume":/venv -w /src/backend \
    "$image" /venv/bin/python -m pytest -p no:cacheprovider "$@"
