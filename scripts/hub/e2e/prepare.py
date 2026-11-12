#!/usr/bin/env python3
"""Puts the disposable stack of compose.yml in order before it starts. Run
through the `tool` service, so the backend's own code and its pinned
environment do the work; run.sh gives the order.

    prepare.py config          as root: the system-access file and the Caddyfile, into /run/hub-e2e
    prepare.py legacy-schema   with the code of release 2.0.3 mounted at /app: its schema, accounts
                               shaped like the server's, the revision stamped 20261112_0033
    prepare.py revision        the revision the database is at

Every name and password here is made up for this stack.
"""

import json
import os
import sys
from pathlib import Path

RUN = Path("/run/hub-e2e")
LEGACY_REVISION = "20261112_0033"
ADMIN_TABS = '["dashboard","reference","automation","userbot","logs","admin"]'

# What the server held before the Time migrations, by role and state (read
# with deploy/time-hub/prechecks.sql): two active administrators and one
# operator. The disabled administrator is added to show what 0035 does with one.
LEGACY_ACCOUNTS = (
    ("e2e_admin", "admin", True, "Synthetic administrator"),
    ("e2e_admin2", "admin", True, "Second synthetic administrator"),
    ("e2e_operator", "operator", True, "Synthetic operator"),
    ("e2e_old_admin", "admin", False, "Disabled synthetic administrator"),
)


def config():
    """The two files the server keeps outside the image, for this stack."""
    from services.root_access_config_service import hash_password_pbkdf2

    token_hash, salt = hash_password_pbkdf2(os.environ["SYSTEM_ACCESS_TOKEN"])
    access = {
        "version": 1,
        "system_access": {
            "enforced": True,
            "tokens": [{"id": "e2e", "kind": "pbkdf2_sha256", "hash": token_hash, "salt": salt, "active": True}],
        },
        "scopes": [],
    }
    RUN.mkdir(parents=True, exist_ok=True)
    (RUN / "root-access.server.json").write_text(json.dumps(access), encoding="utf-8")

    # deploy/Caddyfile as it is, except for the address: the real one would ask
    # Let's Encrypt for a certificate in the server's name.
    lines = Path("/deploy/Caddyfile").read_text(encoding="utf-8").splitlines(keepends=True)
    sites = [
        index
        for index, line in enumerate(lines)
        if line.rstrip().endswith("{") and not line[:1].isspace() and not line.startswith("#")
    ]
    if len(sites) != 1:
        sys.exit(f"deploy/Caddyfile: expected one site block, found {len(sites)}")
    address = lines[sites[0]].rstrip()[:-1].strip()
    lines[sites[0]] = ":80 {\n"
    (RUN / "Caddyfile").write_text("".join(lines), encoding="utf-8")
    for name in ("root-access.server.json", "Caddyfile"):
        (RUN / name).chmod(0o644)
    # Where checks.py keeps a session from one step of the rehearsal to the
    # next; the checks do not run as root.
    (RUN / "state").mkdir(exist_ok=True)
    (RUN / "state").chmod(0o777)
    print(f"config: system-access file and Caddyfile written ({address} -> :80, {len(lines)} lines otherwise unchanged)")


def alembic_config():
    from alembic.config import Config

    return Config("/app/alembic.ini")


def legacy_schema():
    """The database as release 2.0.3 leaves it, with accounts in it."""
    from alembic import command
    from alembic.script import ScriptDirectory
    from sqlalchemy import text

    from database.connection import SessionLocal, engine
    from database.models import Base, User
    from services.access_control_service import hash_password

    heads = ScriptDirectory.from_config(alembic_config()).get_heads()
    newer = [name for name in ("checks_access", "time_role", "global_admin") if hasattr(User, name)]
    if heads != [LEGACY_REVISION] or newer:
        sys.exit(f"legacy-schema needs the code of release 2.0.3 at /app; this one has heads {heads}, columns {newer}")

    # create_all() in place of the migrations that built the server's schema;
    # the one thing they add that it does not is the extension the text indexes use.
    with engine.begin() as connection:
        connection.execute(text("CREATE EXTENSION IF NOT EXISTS pg_trgm"))
    Base.metadata.create_all(bind=engine)
    password = os.environ["E2E_PASSWORD"]
    with SessionLocal() as session:
        for username, role, active, display_name in LEGACY_ACCOUNTS:
            password_hash, salt = hash_password(password)
            session.add(
                User(
                    username=username,
                    password_hash=password_hash,
                    salt=salt,
                    role=role,
                    display_name=display_name,
                    allowed_tabs=ADMIN_TABS if role == "admin" else '["dashboard"]',
                    allowed_folders="[]",
                    forbidden_periods="[]",
                    allowed_sources="[]",
                    can_toggle_sources=(role == "admin"),
                    permissions_version=1,
                    is_active=active,
                )
            )
        session.commit()
    command.stamp(alembic_config(), LEGACY_REVISION)
    print(f"legacy-schema: {len(Base.metadata.tables)} tables, {len(LEGACY_ACCOUNTS)} accounts, stamped {LEGACY_REVISION}")


def revision():
    from sqlalchemy import text

    from database.connection import engine

    with engine.connect() as connection:
        print(connection.execute(text("SELECT version_num FROM alembic_version")).scalar())


STEPS = {"config": config, "legacy-schema": legacy_schema, "revision": revision}

if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] not in STEPS:
        print(__doc__)
        sys.exit(2)
    STEPS[sys.argv[1]]()
