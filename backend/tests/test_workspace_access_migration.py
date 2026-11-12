"""Revision 0034 (workspace access): two columns arrive, nobody's access moves.

Run here on SQLite through the same Operations API the revision uses. The
round trip with the alembic command line on a disposable PostgreSQL is recorded
in docs/handoff/TIME_HUB_ADMIN_PROGRESS.md.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest
import sqlalchemy as sa
from alembic.migration import MigrationContext
from alembic.operations import Operations
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect
from sqlalchemy.exc import IntegrityError

ALEMBIC = Path(__file__).resolve().parents[1] / "alembic"
TABLE = "users"


def _revision():
    name = "20261112_0034_workspace_access.py"
    spec = importlib.util.spec_from_file_location(name.removesuffix(".py"), ALEMBIC / "versions" / name)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _columns(connection) -> set:
    return {item["name"] for item in inspect(connection).get_columns(TABLE)}


def _accounts(connection) -> list:
    return connection.execute(sa.text(
        f"SELECT id, username, role, is_active, allowed_tabs, allowed_folders, permissions_version FROM {TABLE} ORDER BY id"
    )).all()


def test_the_workspace_revision_is_the_only_head_and_follows_production():
    script = ScriptDirectory(str(ALEMBIC))
    assert len(script.get_heads()) == 1
    # Production is on 0033; this revision is the single step from there.
    assert script.get_revision("20261112_0034").down_revision == "20261112_0033"


def test_existing_accounts_keep_checks_and_nobody_gets_time(monkeypatch):
    module = _revision()
    engine = create_engine("sqlite://")
    with engine.begin() as connection:
        monkeypatch.setattr(module, "op", Operations(MigrationContext.configure(connection)))
        # The table as release 2.0.2 has it, as far as this revision cares.
        connection.execute(sa.text(
            f"CREATE TABLE {TABLE} (id INTEGER PRIMARY KEY, username VARCHAR(50) NOT NULL, role VARCHAR(20) NOT NULL, "
            "is_active BOOLEAN NOT NULL, allowed_tabs TEXT NOT NULL, allowed_folders TEXT NOT NULL, "
            "permissions_version INTEGER NOT NULL)"))
        connection.execute(sa.text(
            f"INSERT INTO {TABLE} (id, username, role, is_active, allowed_tabs, allowed_folders, permissions_version) VALUES "
            "(1, 'owner', 'admin', 1, '[\"dashboard\",\"admin\"]', '[]', 4), "
            "(2, 'operator', 'operator', 1, '[\"dashboard\"]', '[1,2]', 2), "
            "(3, 'gone', 'operator', 0, '[\"dashboard\"]', '[]', 9)"))
        before = _accounts(connection)

        module.upgrade()
        module.upgrade()  # a second run is a no-op
        assert {"checks_access", "time_role"} <= _columns(connection)
        # Roles, tabs, folders, activity and the permissions version are as they were.
        assert _accounts(connection) == before
        granted = connection.execute(sa.text(
            f"SELECT id, checks_access, time_role FROM {TABLE} ORDER BY id")).all()
        assert [(row[0], bool(row[1]), row[2]) for row in granted] == [(1, True, None), (2, True, None), (3, True, None)]

        # An account inserted by the previous release, which knows neither column,
        # is a Checks account as before.
        connection.execute(sa.text(
            f"INSERT INTO {TABLE} (id, username, role, is_active, allowed_tabs, allowed_folders, permissions_version) "
            "VALUES (4, 'made-by-old-code', 'operator', 1, '[\"dashboard\"]', '[]', 1)"))
        row = connection.execute(sa.text(f"SELECT checks_access, time_role FROM {TABLE} WHERE id = 4")).one()
        assert (bool(row[0]), row[1]) == (True, None)

        # Only the three roles, or none.
        for role in ("employee", "viewer", "manager"):
            connection.execute(sa.text(f"UPDATE {TABLE} SET time_role = :role WHERE id = 2"), {"role": role})
        with pytest.raises(IntegrityError):
            with connection.begin_nested():
                connection.execute(sa.text(f"UPDATE {TABLE} SET time_role = 'owner' WHERE id = 2"))
        with pytest.raises(IntegrityError):
            with connection.begin_nested():
                connection.execute(sa.text(f"UPDATE {TABLE} SET checks_access = NULL WHERE id = 2"))

        module.downgrade()
        module.downgrade()  # likewise
        assert not {"checks_access", "time_role"} & _columns(connection)
        assert _accounts(connection)[:3] == before


def test_the_model_and_the_revision_describe_the_same_columns():
    from database.models import User

    module = _revision()
    table = User.__table__
    assert table.c.checks_access.nullable is False
    assert str(table.c.checks_access.server_default.arg) == "true"
    assert table.c.time_role.nullable is True and table.c.time_role.type.length == 20
    constraint = next(item for item in table.constraints if getattr(item, "name", None) == module.TIME_ROLE_CHECK)
    assert str(constraint.sqltext) == module.TIME_ROLE_CONDITION
