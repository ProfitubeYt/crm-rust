"""Tests for the learned operator -> annotation rules.

The central invariant is that a rule may never be trained on its own output:
that would drive purity to 1.0 by construction and freeze the first mistake in
permanently.
"""
from __future__ import annotations

import uuid
from datetime import date, datetime
from decimal import Decimal

import pytest

from database.models import AnnotationArticle, AnnotationPerson, AnnotationRule, Transaction
from services import annotation_rule_service as R
from services import annotation_service as A


@pytest.fixture(autouse=True)
def _clear_rule_cache():
    R.invalidate_rule_cache()
    yield
    R.invalidate_rule_cache()


@pytest.fixture
def references(db_session):
    db_session.add_all([
        AnnotationArticle(code="р1", label="ФЗП,ФМП,МП", direction="expense", sort_order=1),
        AnnotationArticle(code="р3", label="МП,благотвор.", direction="expense", sort_order=3),
        AnnotationPerson(code="СТ", sort_order=1),
    ])
    db_session.commit()
    return A.load_reference_codes(db_session)


def _annotated(db_session, operator, article, *, source="manual", message_id=None, day=2):
    txn = Transaction(
        uuid=uuid.uuid4(),
        raw_message="r",
        source_type="AUTO",
        source_chat_id=1,
        source_message_id=message_id if message_id is not None else int(uuid.uuid4().int % 10**8),
        transaction_date=datetime(2026, 3, day, 12, 0),
        amount=Decimal("-1000.00"),
        currency="UZS",
        card_last_4="1111",
        operator_raw=operator,
        transaction_type="DEBIT",
    )
    db_session.add(txn)
    db_session.flush()
    A.apply_annotation_patch(db_session, txn, {"article": article}, source=source, validate=False)
    db_session.commit()
    return txn


def test_rule_is_learned_from_repeated_manual_annotations(db_session, references):
    for i in range(4):
        _annotated(db_session, "OOO STREETPARK SYSTE", "р1", day=i + 1)

    summary = R.recompute(db_session, fields=("article",))
    assert summary["created"] == 1

    rule = db_session.query(AnnotationRule).one()
    assert rule.operator_key == "OOO STREETPARK SYSTE"
    assert rule.value == "р1"
    assert rule.support == 4
    assert float(rule.purity) == 1.0


def test_support_below_threshold_yields_no_rule(db_session, references):
    """One or two observations are a coincidence, not a rule."""
    _annotated(db_session, "RARE SHOP", "р1", day=1)
    _annotated(db_session, "RARE SHOP", "р1", day=2)

    summary = R.recompute(db_session, fields=("article",), min_support=3)
    assert summary["created"] == 0
    assert db_session.query(AnnotationRule).count() == 0


def test_impure_operator_yields_no_rule(db_session, references):
    """A merchant used for two different articles cannot be automated."""
    for i in range(3):
        _annotated(db_session, "MIXED SHOP", "р1", day=i + 1)
    for i in range(3):
        _annotated(db_session, "MIXED SHOP", "р3", day=i + 10)

    summary = R.recompute(db_session, fields=("article",), min_support=3, min_purity=0.9)
    assert summary["created"] == 0


def test_training_ignores_rule_and_agent_rows(db_session, references):
    """Without this filter a rule would confirm itself into permanence."""
    for i in range(5):
        _annotated(db_session, "AUTO SHOP", "р1", source="rule", day=i + 1)
    for i in range(5):
        _annotated(db_session, "AUTO SHOP", "р3", source="agent", day=i + 10)

    summary = R.recompute(db_session, fields=("article",))
    assert summary["created"] == 0
    assert db_session.query(AnnotationRule).count() == 0


def test_operator_key_collapses_punctuation_and_case(db_session, references):
    """Same merchant, differently typed, must train one rule."""
    _annotated(db_session, "  Korzinka-Market ", "р1", day=1)
    _annotated(db_session, "korzinka   market", "р1", day=2)
    _annotated(db_session, "KORZINKA MARKET", "р1", day=3)

    R.recompute(db_session, fields=("article",))
    rule = db_session.query(AnnotationRule).one()
    assert rule.operator_key == "KORZINKA MARKET"
    assert rule.support == 3


def test_apply_fills_empty_fields_only(db_session, references):
    for i in range(3):
        _annotated(db_session, "SHOP X", "р1", day=i + 1)
    R.recompute(db_session, fields=("article",))

    empty = Transaction(
        uuid=uuid.uuid4(), raw_message="r", source_type="AUTO", source_chat_id=1,
        source_message_id=9001, transaction_date=datetime(2026, 3, 20, 12, 0),
        amount=Decimal("-500.00"), currency="UZS", card_last_4="1111",
        operator_raw="SHOP X", transaction_type="DEBIT",
    )
    db_session.add(empty)
    db_session.commit()

    result = R.apply_rules(db_session, fields=("article",))
    assert result["updated"] == 1
    db_session.refresh(empty)
    assert empty.annotation_article_code == "р1"
    assert empty.annotation_article_src == "rule"


def test_apply_never_touches_a_manual_value(db_session, references):
    for i in range(3):
        _annotated(db_session, "SHOP Y", "р1", day=i + 1)
    R.recompute(db_session, fields=("article",))

    manual = _annotated(db_session, "SHOP Y", "р3", source="manual", day=20)
    R.apply_rules(db_session, fields=("article",))
    db_session.refresh(manual)

    assert manual.annotation_article_code == "р3"
    assert manual.annotation_article_src == "manual"


def test_apply_is_idempotent(db_session, references):
    for i in range(3):
        _annotated(db_session, "SHOP Z", "р1", day=i + 1)
    R.recompute(db_session, fields=("article",))

    db_session.add(Transaction(
        uuid=uuid.uuid4(), raw_message="r", source_type="AUTO", source_chat_id=1,
        source_message_id=9002, transaction_date=datetime(2026, 3, 21, 12, 0),
        amount=Decimal("-500.00"), currency="UZS", card_last_4="1111",
        operator_raw="SHOP Z", transaction_type="DEBIT",
    ))
    db_session.commit()

    first = R.apply_rules(db_session, fields=("article",))
    second = R.apply_rules(db_session, fields=("article",))
    assert first["updated"] == 1
    assert second["updated"] == 0


def test_pinned_rule_survives_recompute(db_session, references):
    for i in range(3):
        _annotated(db_session, "SHOP P", "р1", day=i + 1)
    R.recompute(db_session, fields=("article",))

    rule = db_session.query(AnnotationRule).one()
    rule.value = "р3"
    rule.is_pinned = True
    db_session.commit()

    R.recompute(db_session, fields=("article",))
    db_session.refresh(rule)
    assert rule.value == "р3"


def test_defaults_for_new_uses_the_operation_month(db_session, references):
    patch = R.defaults_for_new(
        db_session, operator_raw="ANY", transaction_date=datetime(2026, 3, 17, 9, 0)
    )
    assert patch["period"] == date(2026, 3, 1)


def test_coverage_report_counts_annotated_rows(db_session, references):
    for i in range(3):
        _annotated(db_session, "SHOP C", "р1", day=i + 1)
    _annotated(db_session, "ONE OFF", "р3", day=10)

    report = R.coverage_report(db_session, fields=("article",), min_support=3)
    assert report["article"]["annotated_rows"] == 4
    assert report["article"]["distinct_operators"] == 2
    # Only the 3-row merchant clears min_support.
    assert report["article"]["coverage"]["1"]["rows"] == 3


def _receipt(db_session, operator, when, message_id):
    txn = Transaction(
        uuid=uuid.uuid4(), raw_message="r", source_type="AUTO", source_chat_id=1,
        source_message_id=message_id, transaction_date=when,
        amount=Decimal("-500.00"), currency="UZS", card_last_4="1111",
        operator_raw=operator, transaction_type="DEBIT",
    )
    db_session.add(txn)
    db_session.commit()
    return txn


def test_fill_defaults_gives_new_and_old_receipts_their_defaults(db_session, references):
    for i in range(3):
        _annotated(db_session, "SHOP F", "р1", day=i + 1)
    R.recompute(db_session, fields=("article",))
    old = _receipt(db_session, "SHOP F", datetime(2026, 1, 15, 9, 0), 9101)
    new = _receipt(db_session, "shop  f", datetime(2026, 11, 10, 18, 30), 9102)
    stranger = _receipt(db_session, "NEVER SEEN", datetime(2026, 11, 11, 8, 0), 9103)

    # A batch smaller than the backlog makes the pass page through all of it.
    result = R.fill_defaults(db_session, batch_size=2)

    for txn, month in ((old, date(2026, 1, 1)), (new, date(2026, 11, 1))):
        db_session.refresh(txn)
        assert txn.annotation_article_code == "р1"
        assert txn.annotation_article_src == "rule"
        assert txn.annotation_period == month
        assert txn.annotation_period_src == "rule"
    db_session.refresh(stranger)
    assert stranger.annotation_period == date(2026, 11, 1)
    assert stranger.annotation_article_code is None
    # The three training rows only lacked a period.
    assert result["updated"] == 6
    assert R.fill_defaults(db_session)["updated"] == 0


def test_fill_defaults_leaves_manual_values_and_locked_periods(db_session, references):
    from database.models import LockedPeriod

    db_session.add(LockedPeriod(date_from=date(2026, 10, 1), date_to=date(2026, 10, 31), locked_by_tg_id=1))
    db_session.commit()
    locked = _receipt(db_session, "SHOP L", datetime(2026, 10, 20, 12, 0), 9201)
    manual = _receipt(db_session, "SHOP L", datetime(2026, 11, 5, 12, 0), 9202)
    A.apply_annotation_patch(db_session, manual, {"period": date(2026, 10, 1)}, source="manual", validate=False)
    db_session.commit()

    result = R.fill_defaults(db_session)

    db_session.refresh(locked)
    db_session.refresh(manual)
    assert locked.annotation_period is None
    assert manual.annotation_period == date(2026, 10, 1)
    assert manual.annotation_period_src == "manual"
    assert result["skipped_locked"] == 1


def test_learned_person_beats_the_default_person(db_session, references, monkeypatch):
    monkeypatch.setenv("ANNOTATION_DEFAULT_PERSON", "СТ")
    db_session.add(AnnotationRule(operator_key="SHOP Q", field="person", value="ДР", support=5, purity=1))
    db_session.commit()
    when = datetime(2026, 11, 10, 9, 0)

    assert R.defaults_for_new(db_session, operator_raw="shop q", transaction_date=when)["person"] == "ДР"
    assert R.defaults_for_new(db_session, operator_raw="ELSEWHERE", transaction_date=when)["person"] == "СТ"


def test_apply_with_a_limit_takes_the_newest_receipts(db_session, references):
    for i in range(3):
        _annotated(db_session, "SHOP N", "р1", day=i + 1)
    R.recompute(db_session, fields=("article",))
    older = _receipt(db_session, "SHOP N", datetime(2026, 3, 20, 12, 0), 9301)
    newer = _receipt(db_session, "SHOP N", datetime(2026, 11, 10, 12, 0), 9302)

    R.apply_rules(db_session, fields=("article",), limit=1)

    db_session.refresh(older)
    db_session.refresh(newer)
    assert newer.annotation_article_code == "р1"
    assert older.annotation_article_code is None
