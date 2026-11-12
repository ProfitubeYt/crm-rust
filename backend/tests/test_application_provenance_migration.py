"""Revision 0033 (application provenance): columns come and go, receipts stay.

The same round trip was run with the alembic command line on a disposable
PostgreSQL; that run is recorded in docs/handoff/ENRICHMENT_PROGRESS.md.
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
TABLE = "transactions"


def _revision():
    name = "20261112_0033_application_provenance.py"
    spec = importlib.util.spec_from_file_location(name.removesuffix(".py"), ALEMBIC / "versions" / name)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _columns(connection) -> set:
    return {item["name"] for item in inspect(connection).get_columns(TABLE)}


def _rows(connection) -> list:
    return connection.execute(sa.text(f"SELECT id, operator_raw, application_mapped FROM {TABLE} ORDER BY id")).all()


def test_the_provenance_revision_is_the_only_head_and_follows_workbook_names():
    script = ScriptDirectory(str(ALEMBIC))
    # One head, whichever it is by now: later revisions follow this one.
    assert len(script.get_heads()) == 1
    assert script.get_revision("20261112_0033").down_revision == "20261111_0032"


def test_provenance_columns_arrive_empty_and_leave_without_touching_a_receipt(monkeypatch):
    module = _revision()
    engine = create_engine("sqlite://")
    with engine.begin() as connection:
        monkeypatch.setattr(module, "op", Operations(MigrationContext.configure(connection)))
        # The table as the previous release has it, as far as this revision cares.
        connection.execute(sa.text(
            f"CREATE TABLE {TABLE} (id INTEGER PRIMARY KEY, operator_raw TEXT, application_mapped VARCHAR(100))"))
        connection.execute(sa.text(
            f"INSERT INTO {TABLE} (id, operator_raw, application_mapped) VALUES "
            "(1, 'GATE ONE, UZ', 'Typed before provenance'), (2, 'SOME SHOP', NULL)"))
        before = _rows(connection)

        module.upgrade()
        module.upgrade()  # a second run is a no-op
        assert {"application_src", "application_confidence"} <= _columns(connection)
        assert _rows(connection) == before
        # Every existing receipt starts with an unknown origin: nothing is guessed.
        assert connection.execute(sa.text(
            f"SELECT COUNT(*) FROM {TABLE} WHERE application_src IS NOT NULL OR application_confidence IS NOT NULL"
        )).scalar() == 0

        connection.execute(sa.text(
            f"UPDATE {TABLE} SET application_src = 'manual' WHERE id = 1"))
        with pytest.raises(IntegrityError):
            with connection.begin_nested():
                connection.execute(sa.text(f"UPDATE {TABLE} SET application_src = 'guess' WHERE id = 2"))
        with pytest.raises(IntegrityError):
            with connection.begin_nested():
                connection.execute(sa.text(f"UPDATE {TABLE} SET application_confidence = 1.5 WHERE id = 2"))

        module.downgrade()
        module.downgrade()
        assert not {"application_src", "application_confidence"} & _columns(connection)
        assert _rows(connection) == before
