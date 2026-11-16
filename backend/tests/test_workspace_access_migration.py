"""Revisions 0034 (workspace access) and 0035 (CRM administration).

0034: two columns arrive, nobody's access moves. 0035: the right to administer
accounts gets a column of its own, and exactly the accounts that had the right
keep it.

Run here on SQLite through the same Operations API the revisions use. The
round trip with the alembic command line on a disposable PostgreSQL was run
by hand and is not repeated here.
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


def _revision(name: str = "20261112_0034_workspace_access.py"):
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


def test_the_two_revisions_are_one_line_from_production_to_the_only_head():
    script = ScriptDirectory(str(ALEMBIC))
    assert script.get_heads() == ["20261112_0035"]
    # Production is on 0033; these are the two steps from there, in this order.
    assert script.get_revision("20261112_0034").down_revision == "20261112_0033"
    assert script.get_revision("20261112_0035").down_revision == "20261112_0034"


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

    assert table.c.global_admin.nullable is False
    assert str(table.c.global_admin.server_default.arg) == "false"


def test_whoever_administered_accounts_still_does_and_nobody_else_starts(monkeypatch):
    workspace = _revision()
    module = _revision("20261112_0035_global_admin.py")
    engine = create_engine("sqlite://")
    with engine.begin() as connection:
        operations = Operations(MigrationContext.configure(connection))
        monkeypatch.setattr(workspace, "op", operations)
        monkeypatch.setattr(module, "op", operations)
        connection.execute(sa.text(
            f"CREATE TABLE {TABLE} (id INTEGER PRIMARY KEY, username VARCHAR(50) NOT NULL, role VARCHAR(20) NOT NULL, "
            "is_active BOOLEAN NOT NULL, allowed_tabs TEXT NOT NULL, allowed_folders TEXT NOT NULL, "
            "permissions_version INTEGER NOT NULL)"))
        connection.execute(sa.text(
            f"INSERT INTO {TABLE} (id, username, role, is_active, allowed_tabs, allowed_folders, permissions_version) VALUES "
            "(1, 'owner', 'admin', 1, '[\"dashboard\",\"admin\"]', '[]', 4), "
            "(2, 'operator', 'operator', 1, '[\"dashboard\"]', '[1,2]', 2), "
            "(3, 'gone', 'operator', 0, '[\"dashboard\"]', '[]', 9), "
            # Disabled, and written the way an old hand-made row might be: the
            # right it had is the right it gets back when it is enabled again.
            "(4, 'former', 'Admin', 0, '[\"dashboard\",\"admin\"]', '[]', 3)"))
        workspace.upgrade()
        # A Time manager from the step before: a workspace role, however senior.
        connection.execute(sa.text(f"UPDATE {TABLE} SET time_role = 'manager' WHERE id = 2"))
        before = _accounts(connection)
        grants_before = connection.execute(sa.text(
            f"SELECT id, checks_access, time_role FROM {TABLE} ORDER BY id")).all()

        module.upgrade()
        module.upgrade()  # a second run is a no-op
        assert "global_admin" in _columns(connection)
        flags = connection.execute(sa.text(f"SELECT id, global_admin FROM {TABLE} ORDER BY id")).all()
        assert [(row[0], bool(row[1])) for row in flags] == [(1, True), (2, False), (3, False), (4, True)]
        # Nothing else about anybody moved: roles, tabs, folders, versions, workspaces.
        assert _accounts(connection) == before
        assert connection.execute(sa.text(
            f"SELECT id, checks_access, time_role FROM {TABLE} ORDER BY id")).all() == grants_before

        # An account inserted by code that knows nothing of the column, an
        # administrator of Checks included, does not administer the CRM.
        connection.execute(sa.text(
            f"INSERT INTO {TABLE} (id, username, role, is_active, allowed_tabs, allowed_folders, permissions_version) "
            "VALUES (5, 'made-by-old-code', 'admin', 1, '[\"dashboard\"]', '[]', 1)"))
        assert bool(connection.execute(sa.text(f"SELECT global_admin FROM {TABLE} WHERE id = 5")).scalar()) is False
        with pytest.raises(IntegrityError):
            with connection.begin_nested():
                connection.execute(sa.text(f"UPDATE {TABLE} SET global_admin = NULL WHERE id = 2"))

        # A column that is already there was filled on purpose: it is left alone.
        connection.execute(sa.text(f"UPDATE {TABLE} SET global_admin = 0 WHERE id = 1"))
        module.upgrade()
        assert bool(connection.execute(sa.text(f"SELECT global_admin FROM {TABLE} WHERE id = 1")).scalar()) is False

        module.downgrade()
        module.downgrade()  # likewise
        assert "global_admin" not in _columns(connection)
        assert {"checks_access", "time_role"} <= _columns(connection)
        assert _accounts(connection)[:4] == before
