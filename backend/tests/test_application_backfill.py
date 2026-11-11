from datetime import date, datetime
from decimal import Decimal

import pytest

from database.models import LockedPeriod, OperatorReference, Transaction
from parsers.operator_mapper import invalidate_operator_mapper_cache
from services import application_backfill
from services.application_backfill import backfill_applications, report_lines
from services.application_detection import invalidate_confirmed_cache


@pytest.fixture(autouse=True)
def _fresh_caches():
    # The matcher caches the dictionary per process; keep these rows out of
    # the other tests' mapper.
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()
    yield
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()


def _tx(n, operator, app=None, when=datetime(2026, 5, 10, 12), is_p2p=False, src=None):
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
        application_src=src,
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


def _sources(db):
    return {t.source_message_id: t.application_src for t in db.query(Transaction).all()}


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
    assert set(_sources(db_session).values()) == {None}


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


# --- provenance, reasons and the things that must not move ----------------------

def test_every_fill_carries_its_source_and_confidence(db_session):
    _seed(db_session)
    db_session.add(_tx(8, "XAZNA PAYMENTS, UZ"))
    db_session.commit()
    report = backfill_applications(db_session, apply=True)
    rows = {t.source_message_id: t for t in db_session.query(Transaction).all()}
    assert (rows[1].application_src, float(rows[1].application_confidence)) == ("dict", 1.0)
    assert (rows[2].application_src, float(rows[2].application_confidence)) == ("dict", 0.85)
    assert (rows[8].application_mapped, rows[8].application_src, float(rows[8].application_confidence)) == (
        "Xazna", "rule", 0.9,
    )
    # The hand-typed value keeps its unknown origin: nobody stamped it for them.
    assert rows[3].application_src is None and rows[3].application_confidence is None
    assert report.by_reason == {"dictionary_exact": 1, "dictionary_substring": 2, "brand_in_operator": 1}


def test_unresolved_receipts_are_counted_by_reason_not_by_name(db_session):
    _seed(db_session)
    db_session.add_all([
        _tx(8, "XAZNA PAYNET TOLOV>T"),
        _tx(9, "XKKM 00014 ATM 000001, UZ"),
        _tx(10, "Unknown"),
    ])
    db_session.commit()
    report = backfill_applications(db_session)
    assert report.unresolved == {
        "no_evidence": 2, "ambiguous": 1, "cash_or_terminal": 1, "no_operator_in_receipt": 1,
    }
    assert report.ambiguous == {"Paynet | Xazna": 1}

    text = "\n".join(report_lines(report))
    assert "no_evidence" in text and "Paynet | Xazna" in text
    # Operator lines can carry a person's name: the default report prints none.
    assert "ANGLESEY" not in text and "XKKM" not in text
    assert "ANGLESEY FOOD" in "\n".join(report_lines(report, top=5))


def test_a_stored_value_that_differs_is_reported_and_left_alone(db_session):
    _seed(db_session)
    report = backfill_applications(db_session, apply=True)
    assert report.disagreements == {("Typed by hand", "Paynet"): 1}
    assert _apps(db_session)[3] == "Typed by hand"


def test_what_a_person_set_or_emptied_is_never_filled(db_session):
    _seed(db_session)
    db_session.add_all([
        _tx(8, "PAYNET Z", app="Mine", src="manual"),
        _tx(9, "PAYNET Q", app=None, src="manual"),     # a person said: no application
    ])
    db_session.commit()
    report = backfill_applications(db_session, apply=True, revalidate=True)
    apps = _apps(db_session)
    assert report.manual == 2
    assert (apps[8], apps[9]) == ("Mine", None)
    assert _sources(db_session)[9] == "manual"
    assert ("Mine", "Paynet") not in report.disagreements


def test_revalidate_corrects_only_what_this_module_wrote(db_session):
    _seed(db_session)
    backfill_applications(db_session, apply=True)

    # The dictionary line turns out to be wrong and is switched off.
    db_session.query(OperatorReference).filter_by(operator_name="HAMKORBANK ATB, UZ").update({"is_active": False})
    db_session.query(OperatorReference).filter_by(operator_name="PAYNET").update({"application_name": "Paynet 2"})
    db_session.commit()

    dry = backfill_applications(db_session, revalidate=True)
    assert (dry.stale, dry.revalidated, dry.matched) == (3, 3, 0)
    assert _apps(db_session)[1] == "Hamkor"            # a dry run writes nothing

    without = backfill_applications(db_session, apply=True)
    assert (without.stale, without.revalidated) == (3, 0)
    assert _apps(db_session)[1] == "Hamkor"            # not asked: reported only

    report = backfill_applications(db_session, apply=True, revalidate=True)
    apps, sources = _apps(db_session), _sources(db_session)
    assert report.revalidated == 3
    assert apps[1] is None and sources[1] is None      # no evidence any more: cleared
    # "PAYNET HUM2UZC" names Paynet itself, and the dictionary now says Paynet 2.
    assert apps[2] is None and apps[6] is None
    assert apps[3] == "Typed by hand"                  # never this module's value

    again = backfill_applications(db_session, apply=True, revalidate=True)
    assert (again.stale, again.revalidated, again.matched) == (0, 0, 0)


def test_revalidate_leaves_locked_periods_alone(db_session):
    _seed(db_session)
    db_session.add(_tx(8, "HAMKORBANK ATB, UZ", app="Hamkor", src="dict", when=datetime(2026, 1, 20, 9)))
    db_session.commit()
    db_session.query(OperatorReference).filter_by(operator_name="HAMKORBANK ATB, UZ").update({"is_active": False})
    db_session.commit()
    report = backfill_applications(db_session, apply=True, revalidate=True)
    assert report.stale == 1 and report.revalidated == 0
    assert _apps(db_session)[8] == "Hamkor"


def test_the_dry_run_reads_a_database_that_has_no_provenance_columns_yet(db_session, monkeypatch):
    # Production before revision 0033, read through a role with few columns.
    _seed(db_session)
    monkeypatch.setattr(application_backfill, "has_provenance", lambda _db: False)
    report = backfill_applications(db_session)
    assert (report.candidates, report.matched, report.locked) == (6, 3, 1)


def test_limit_counts_receipts_without_an_application(db_session):
    _seed(db_session)
    report = backfill_applications(db_session, limit=2)
    assert report.candidates == 2
