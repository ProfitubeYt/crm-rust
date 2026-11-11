"""Revision 0032 (workbook names) on SQLite: the column comes and goes, the records stay.

Follows the migration checks in test_excel_sync_service.py. The same round trip
was run by hand with the alembic command line on a disposable PostgreSQL; that
run is recorded in docs/handoff/BACKEND_PROGRESS.md and is not repeated here.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path

import sqlalchemy as sa
from alembic.migration import MigrationContext
from alembic.operations import Operations
from alembic.script import ScriptDirectory
from sqlalchemy import create_engine, inspect
from sqlalchemy.orm import Session

from database.excel_sync_models import ExcelSyncActivation
from database.models import User

ALEMBIC = Path(__file__).resolve().parents[1] / "alembic"
TABLE = "excel_sync_activations"


def revision(filename: str):
    spec = importlib.util.spec_from_file_location(filename.removesuffix(".py"), ALEMBIC / "versions" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def columns(connection) -> dict[str, dict]:
    return {item["name"]: item for item in inspect(connection).get_columns(TABLE)}


def records(connection) -> list[tuple]:
    return connection.execute(sa.text(
        f"SELECT workbook_id, user_id, password_hash, salt, permissions_version, failed_attempts FROM {TABLE} "
        "ORDER BY workbook_id")).all()


def test_the_names_revision_follows_device_activation_on_a_single_line():
    # Revision 0033 (application provenance) now follows the names revision, so
    # the head is asserted where it is added; here: one line, unbroken at 0032.
    script = ScriptDirectory(str(ALEMBIC))
    assert len(script.get_heads()) == 1
    assert script.get_revision("20261111_0032").down_revision == "20260923_0031"


def test_names_reach_existing_records_and_leave_without_taking_them(monkeypatch):
    grants = revision("20260909_0028_excel_sync.py")
    activations = revision("20260923_0031_excel_device_activation.py")
    names = revision("20261111_0032_excel_workbook_names.py")
    engine = create_engine("sqlite://")
    User.__table__.create(engine)
    with engine.begin() as connection:
        operations = Operations(MigrationContext.configure(connection))
        for module in (grants, activations, names):
            monkeypatch.setattr(module, "op", operations)
        # The database of the previous release: a password record without a name.
        grants.upgrade()
        activations.upgrade()
        assert "name" not in columns(connection)
        with Session(bind=connection) as session:
            session.add(User(id=7, username="migration-owner", password_hash="unused", salt="unused", role="admin"))
            session.flush()
        connection.execute(sa.text(
            f"INSERT INTO {TABLE} (workbook_id, user_id, password_hash, salt, permissions_version) "
            "VALUES ('connected-by-hand', 7, 'hash-1', 'salt-1', 3)"))
        before = records(connection)

        names.upgrade()
        names.upgrade()
        added = columns(connection)["name"]
        assert added["nullable"] is True and added["type"].length == 120
        assert records(connection) == before
        with Session(bind=connection) as session:
            # The model of this release reads the old record and stores a named one.
            assert session.get(ExcelSyncActivation, "connected-by-hand").name is None
            session.add(ExcelSyncActivation(workbook_id="made-by-the-server", user_id=7, name="Касса",
                                            password_hash="hash-2", salt="salt-2", permissions_version=3))
            session.flush()
        kept = records(connection)
        assert [row[0] for row in kept] == ["connected-by-hand", "made-by-the-server"]

        names.downgrade()
        names.downgrade()
        # Only the names are gone: the previous release still finds both password records.
        assert "name" not in columns(connection)
        assert records(connection) == kept
        described = inspect(connection)
        assert described.get_pk_constraint(TABLE)["constrained_columns"] == ["workbook_id"]
        assert [item["name"] for item in described.get_indexes(TABLE)] == ["idx_excel_sync_activations_user"]
        assert [item["referred_table"] for item in described.get_foreign_keys(TABLE)] == ["users"]

        names.upgrade()
        assert connection.execute(sa.text(f"SELECT name FROM {TABLE}")).scalars().all() == [None, None]
        assert records(connection) == kept
