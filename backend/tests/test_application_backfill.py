from datetime import date, datetime
from decimal import Decimal

import pytest

from database.models import LockedPeriod, OperatorReference, Transaction
from parsers.operator_mapper import invalidate_operator_mapper_cache
from services.application_backfill import backfill_applications


@pytest.fixture(autouse=True)
def _fresh_mapper_cache():
    # The matcher caches the dictionary per process; keep these rows out of
    # the other tests' mapper.
    invalidate_operator_mapper_cache()
    yield
    invalidate_operator_mapper_cache()


def _tx(n, operator, app=None, when=datetime(2026, 5, 10, 12), is_p2p=False):
    return Transaction(
        raw_message=f"m{n}",
        source_type="AUTO",
        source_chat_id=1,
        source_message_id=n,
        transaction_date=when,
        amount=Decimal("-1000"),
        transaction_type="DEBIT",
        operator_raw=operator,
        application_mapped=app,
        is_p2p=is_p2p,
    )


def _seed(db):
    db.add_all([
        OperatorReference(operator_name="HAMKORBANK ATB, UZ", application_name="Hamkor", is_p2p=True, is_active=True),
        OperatorReference(operator_name="PAYNET", application_name="Paynet", is_p2p=False, is_active=True),
        OperatorReference(operator_name="OFF", application_name="Off", is_p2p=False, is_active=False),
    ])
    db.add_all([
        _tx(1, "HAMKORBANK ATB, UZ"),
        _tx(2, "PAYNET HUM2UZC"),
        _tx(3, "PAYNET HUM2UZC", app="Typed by hand"),
        _tx(4, "ANGLESEY FOOD, Toshkent"),
        _tx(5, "OFF"),
        _tx(6, "PAYNET X", app="  "),
        _tx(7, "PAYNET Y", when=datetime(2026, 1, 15, 9)),
    ])
    db.add(LockedPeriod(date_from=date(2026, 1, 1), date_to=date(2026, 1, 31), is_active=True, locked_by_tg_id=1))
    db.commit()


def _apps(db):
    return {t.source_message_id: t.application_mapped for t in db.query(Transaction).all()}


def test_dry_run_counts_but_writes_nothing(db_session):
    _seed(db_session)
    report = backfill_applications(db_session)
    assert not report.applied
    assert report.candidates == 6
    assert report.matched == 3
    assert report.locked == 1
    assert report.unmatched == 2
    assert report.by_application == {"Hamkor": 1, "Paynet": 2}
    assert _apps(db_session)[1] is None


def test_apply_fills_only_empty_unlocked_matches(db_session):
    _seed(db_session)
    report = backfill_applications(db_session, apply=True)
    apps = _apps(db_session)
    assert report.matched == 3
    assert apps[1] == "Hamkor"
    assert apps[2] == "Paynet"
    assert apps[3] == "Typed by hand"
    assert apps[4] is None
    assert apps[5] is None
    assert apps[6] == "Paynet"
    assert apps[7] is None


def test_p2p_flag_changes_only_when_asked(db_session):
    _seed(db_session)
    backfill_applications(db_session, apply=True)
    hamkor = db_session.query(Transaction).filter_by(source_message_id=1).one()
    assert hamkor.is_p2p is False

    hamkor.application_mapped = None
    db_session.commit()
    report = backfill_applications(db_session, apply=True, set_p2p=True)
    assert report.p2p_changed == 1
    db_session.refresh(hamkor)
    assert hamkor.is_p2p is True


def test_a_second_run_finds_nothing_left(db_session):
    _seed(db_session)
    backfill_applications(db_session, apply=True)
    report = backfill_applications(db_session, apply=True)
    assert report.matched == 0
