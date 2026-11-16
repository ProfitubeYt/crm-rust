#!/usr/bin/env bash
#
# Roll out the transaction-annotations release and point the agent at an
# OpenAI-compatible gateway (9router / Claude).
#
# Differs from the earlier AI-agent rollout script (removed, in git history) in three ways that matter:
#
#  1. The migration runs in /app, not /app/backend. The backend image is built
#     with context ./backend and WORKDIR /app, so `COPY . .` puts alembic.ini at
#     /app/alembic.ini and /app/backend never exists. The older script's
#     `cd /app/backend` fails, and because of `set -e` it aborts the deploy
#     *after* the images are built and postgres/redis are up — leaving the stack
#     half-swapped. Verified against backend/Dockerfile.
#
#  2. It dumps the database before migrating. This release adds columns and
#     indexes to `transactions`; a dump is the only cheap way back.
#
#  3. It verifies the outcome instead of assuming it: schema head, tool
#     registration, and whether the LLM provider actually came up configured.
#
# Usage:
#   AI_TEXT_BASE_URL=https://<gateway>/v1 \
#   AI_TEXT_API_KEY=<key> \
#   AI_TEXT_MODEL=<model> \
#   ./deploy/server-rollout-annotations.sh
#
# Add DRY_RUN=1 to sync and check without restarting anything.
set -euo pipefail

REMOTE_HOST="${REMOTE_HOST:-64.188.106.221}"
REMOTE_USER="${REMOTE_USER:-root}"
REMOTE_DIR="${REMOTE_DIR:-/opt/receipt-parser}"
APP_VERSION="${APP_VERSION:-1.5.0}"
NIP_IO_HOST="${NIP_IO_HOST:-64.188.106.221.nip.io}"
DRY_RUN="${DRY_RUN:-0}"

# Fail before touching anything: a half-configured gateway is worse than none.
for var in AI_TEXT_BASE_URL AI_TEXT_API_KEY AI_TEXT_MODEL; do
  if [[ -z "${!var:-}" ]]; then
    echo "ERROR: $var is required." >&2
    echo "All three must be set together — a base URL from one provider with a" >&2
    echo "model name from another is the usual way this breaks." >&2
    exit 1
  fi
done

echo "==> syncing source to ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}"
rsync -az --delete \
  --exclude '.git' \
  --exclude '.env' \
  --exclude 'node_modules' \
  --exclude 'frontend/node_modules' \
  --exclude 'frontend/dist' \
  --exclude 'frontend/release' \
  --exclude 'frontend/dist_backup_*' \
  --exclude 'backend/.venv' \
  --exclude 'backend/sessions' \
  --exclude 'security/*.json' \
  `# The live Caddyfile carries routes this repo does not know about: the box` \
  `# co-hosts TBS Inform CRM on 172.18.0.1:5555, and the 9router dashboard.` \
  `# Pushing the repo copy would silently unroute another product.` \
  --exclude 'deploy/Caddyfile' \
  --exclude 'windows-customer-bundle' \
  --exclude 'windows-test-bundle' \
  --exclude 'parcer-source.tar.gz' \
  ./ "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DIR}/"

ssh "${REMOTE_USER}@${REMOTE_HOST}" bash <<REMOTE
set -euo pipefail
cd "${REMOTE_DIR}"

echo "==> updating .env"
python3 - <<'PY'
from pathlib import Path

path = Path('.env')
text = path.read_text() if path.exists() else ''
updates = {
    'APP_VERSION': '${APP_VERSION}',
    # Provider-neutral text endpoint. The DEEPSEEK_* values stay untouched as
    # the fallback, so clearing these three reverts the agent to DeepSeek.
    'AI_TEXT_BASE_URL': '${AI_TEXT_BASE_URL}',
    'AI_TEXT_API_KEY': '${AI_TEXT_API_KEY}',
    'AI_TEXT_MODEL': '${AI_TEXT_MODEL}',
    # Annotation rules: a value must repeat 3+ times and hold 90%+ of a
    # merchant's history before it becomes a rule.
    'ANNOTATION_RULE_MIN_SUPPORT': '3',
    'ANNOTATION_RULE_MIN_PURITY': '0.9',
    'ANNOTATION_RULE_CACHE_TTL': '120',
    'ANNOTATION_DEFAULT_PERSON': '',
}
lines = text.splitlines()
seen = set()
out = []
for line in lines:
    if '=' not in line or line.lstrip().startswith('#'):
        out.append(line)
        continue
    key = line.split('=', 1)[0].strip()
    if key in updates:
        out.append(f"{key}={updates[key]}")
        seen.add(key)
    else:
        out.append(line)
for key, value in updates.items():
    if key not in seen:
        out.append(f"{key}={value}")
path.write_text('\n'.join(out) + '\n')
print('   .env updated')
PY

echo "==> backing up the database"
BACKUP="/root/parcer-backup-\$(date +%Y%m%d-%H%M%S).sql.gz"
docker compose exec -T postgres pg_dump -U "\$(grep -E '^POSTGRES_USER=' .env | cut -d= -f2)" \
  "\$(grep -E '^POSTGRES_DB=' .env | cut -d= -f2)" | gzip > "\$BACKUP"
echo "   backup: \$BACKUP (\$(du -h "\$BACKUP" | cut -f1))"

echo "==> building the overlay image (no TDLib recompile)"
# The box has ~2.9 GB RAM and an empty BuildKit cache; a full rebuild would
# recompile TDLib from source and OOM. This release only changes Python code,
# so layer the new code over the existing image instead.
BASE_IMAGE="\${BASE_IMAGE:-\$(docker images --format '{{.Repository}}:{{.Tag}}' \
  | grep '^parcer-backend:' | grep -v ':latest' | head -1)}"
if [[ -z "\$BASE_IMAGE" ]]; then
  echo "ERROR: no parcer-backend base image found to layer onto." >&2
  exit 1
fi
NEW_IMAGE="parcer-backend:${APP_VERSION}-annotations"
echo "   base: \$BASE_IMAGE  ->  \$NEW_IMAGE"
docker build -f backend/Dockerfile.overlay --build-arg BASE_IMAGE="\$BASE_IMAGE" \
  -t "\$NEW_IMAGE" ./backend

# docker-compose.yml already reads \${BACKEND_IMAGE}; point every backend-derived
# service at the new tag in one place.
python3 - <<PYIMG
from pathlib import Path
path = Path('.env')
text = path.read_text()
lines = [l for l in text.splitlines()]
out, seen = [], False
for line in lines:
    if line.split('=', 1)[0].strip() == 'BACKEND_IMAGE':
        out.append("BACKEND_IMAGE=\$NEW_IMAGE"); seen = True
    else:
        out.append(line)
if not seen:
    out.append("BACKEND_IMAGE=\$NEW_IMAGE")
path.write_text('\n'.join(out) + '\n')
print('   BACKEND_IMAGE=\$NEW_IMAGE')
PYIMG

docker compose up -d postgres redis
sleep 5

echo "==> pre-flight: confirm the migration path exists in the image"
docker compose run --rm backend bash -lc 'test -f /app/alembic.ini || { echo "alembic.ini not at /app — check the Dockerfile build context"; exit 1; }'

if [[ "${DRY_RUN}" == "1" ]]; then
  echo "==> DRY_RUN=1: stopping before migration and restart"
  exit 0
fi

echo "==> migrating schema"
# /app, not /app/backend — see the header comment.
# The trgm index is built CONCURRENTLY, so this does not lock `transactions`.
docker compose run --rm backend bash -lc 'cd /app && alembic -c alembic.ini upgrade head'

echo "==> restarting services"
# No 'frontend' service here on purpose: this host serves the API only
# (Caddy routes /api and /health to backend:8000, everything else is the API
# banner). The UI reaches users as a new desktop build.
docker compose up -d backend celery_worker celery_ocr_worker celery_maintenance_worker auth_bot caddy

sleep 8

echo "==> verifying"
docker compose run --rm backend bash -lc 'cd /app && alembic -c alembic.ini current' | tail -1

docker compose exec -T backend python -c "
from services.ai_provider import get_text_ai_provider
from services.ai_agent.tool_registry import TOOLS
p = get_text_ai_provider()
print(f'   provider base_url : {p.base_url}')
print(f'   provider model    : {p.default_model}')
print(f'   provider enabled  : {p.enabled}')
assert p.enabled, 'LLM provider came up without a key'
for name in ('suggest_transaction_annotations', 'rollback_transaction_annotations'):
    assert name in TOOLS, f'tool missing: {name}'
print(f'   agent tools       : {len(TOOLS)}')
"

curl -fsS http://127.0.0.1:8000/health && echo
curl -fsS "https://${NIP_IO_HOST}/health" && echo || true

echo "==> done. Rollback: docker compose down && gunzip -c \$BACKUP | docker compose exec -T postgres psql -U ... "
REMOTE

echo
echo "Rolled out. Next, from the UI:"
echo "  1. Settings -> the annotation reference now lists 14 articles / 11 people."
echo "  2. Import the client's workbook with dry_run first and read the report."
echo "     A large 'unmatched' count most likely means a timezone mismatch."
echo "  3. Recompute the rules, then try the agent's annotation tool."
