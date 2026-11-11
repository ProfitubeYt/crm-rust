"""Tests for per-transaction bookkeeping annotations.

Each test here pins one invariant that is easy to break silently: provenance
protection, code folding across alphabets, month coercion, the rule-training
filter, and the importer's matching/idempotency behaviour.
"""
from __future__ import annotations

import uuid
from datetime import date, datetime
from decimal import Decimal

import pytest

from database.models import (
    AnnotationArticle,
    AnnotationPerson,
    Transaction,
    TransactionAnnotationEvent,
)
from services import annotation_service as A
from services import annotation_import_service as I


@pytest.fixture
def references(db_session):
    db_session.add_all([
        AnnotationArticle(code="р1", label="ФЗП,ФМП,МП", direction="expense", sort_order=1),
        AnnotationArticle(code="р3", label="МП,благотвор.", direction="expense", sort_order=3),
        AnnotationArticle(code="п2", label="банк, дивиденды", direction="income", sort_order=2),
        AnnotationPerson(code="СТ", sort_order=1),
        AnnotationPerson(code="ДиК", sort_order=2),
    ])
    db_session.commit()
    return A.load_reference_codes(db_session)


def _txn(db_session, **overrides):
    values = dict(
        uuid=uuid.uuid4(),
        raw_message="test receipt",
        source_type="AUTO",
        source_chat_id=100,
        source_message_id=overrides.pop("message_id", 1),
        transaction_date=datetime(2026, 3, 2, 17, 14),
        amount=Decimal("-282800.00"),
        currency="UZS",
        card_last_4="4428",
        operator_raw="BUSTONLIK BXO>Gazalkent",
        transaction_type="DEBIT",
    )
    values.update(overrides)
    txn = Transaction(**values)
    db_session.add(txn)
    db_session.commit()
    return txn


# --------------------------------------------------------------- code folding

@pytest.mark.parametrize(
    "raw,expected",
    [
        ("р1", "р1"),          # already Cyrillic
        ("p1", "р1"),          # Latin p
        ("P1", "Р1"),          # Latin capital P folds to Cyrillic capital
        ("CT", "СТ"),          # Latin C and T
        ("  р3  ", "р3"),
        (None, ""),
    ],
)
def test_normalize_code_folds_latin_homoglyphs(raw, expected):
    """The workbook is typed on a Russian keyboard: both spellings will appear."""
    assert A.normalize_code(raw) == expected


def test_unknown_code_is_rejected(references):
    with pytest.raises(A.AnnotationValidationError):
        A.normalize_patch({"article": "zz9"}, reference_codes=references)


def test_latin_code_resolves_to_reference_entry(references):
    patch = A.normalize_patch({"article": "p3"}, reference_codes=references)
    assert patch["article"] == "р3"


# ------------------------------------------------------------------- periods

@pytest.mark.parametrize(
    "raw,expected",
    [
        (date(2026, 3, 17), date(2026, 3, 1)),
        (datetime(2026, 3, 17, 9, 30), date(2026, 3, 1)),
        ("2026-03-17", date(2026, 3, 1)),
        ("2026-03", date(2026, 3, 1)),
        ("17.03.2026", date(2026, 3, 1)),
        ("", None),
        (None, None),
    ],
)
def test_period_is_always_first_of_month(raw, expected):
    """The period means a month; storing a day would imply precision we lack."""
    assert A.coerce_period(raw) == expected


# ---------------------------------------------------------------- provenance

def test_automation_cannot_overwrite_manual(db_session, references):
    txn = _txn(db_session)
    A.apply_annotation_patch(
        db_session, txn, {"details": "ручная пометка", "article": "р1"},
        source="manual", reference_codes=references,
    )
    db_session.commit()

    result = A.apply_annotation_patch(
        db_session, txn, {"details": "вп маг", "article": "р3"},
        source="rule", reference_codes=references,
    )

    assert result["changed"] == {}
    assert result["skipped"] == {
        "details": "protected_by:manual",
        "article": "protected_by:manual",
    }
    assert txn.annotation_details == "ручная пометка"
    assert txn.annotation_article_code == "р1"


def test_rule_fills_an_empty_field(db_session, references):
    txn = _txn(db_session)
    result = A.apply_annotation_patch(
        db_session, txn, {"article": "р1"}, source="rule", reference_codes=references,
    )
    assert result["changed"] == {"article": "р1"}
    assert txn.annotation_article_src == "rule"


def test_rule_may_overwrite_agent_and_agent_may_overwrite_rule(db_session, references):
    txn = _txn(db_session)
    A.apply_annotation_patch(db_session, txn, {"article": "р1"}, source="rule",
                             reference_codes=references)
    A.apply_annotation_patch(db_session, txn, {"article": "р3"}, source="agent",
                             reference_codes=references)
    assert txn.annotation_article_code == "р3"
    assert txn.annotation_article_src == "agent"


def test_explicit_overwrite_replaces_manual(db_session, references):
    txn = _txn(db_session)
    A.apply_annotation_patch(db_session, txn, {"details": "ручная"}, source="manual",
                             reference_codes=references)
    result = A.apply_annotation_patch(
        db_session, txn, {"details": "из книги"}, source="import",
        reference_codes=references, overwrite_manual=True,
    )
    assert result["changed"] == {"details": "из книги"}
    assert txn.annotation_details_src == "import"


def test_rule_never_blanks_an_existing_value(db_session, references):
    txn = _txn(db_session)
    A.apply_annotation_patch(db_session, txn, {"details": "вп маг"}, source="rule",
                             reference_codes=references)
    result = A.apply_annotation_patch(db_session, txn, {"details": None}, source="rule",
                                      reference_codes=references)
    assert result["changed"] == {}
    assert txn.annotation_details == "вп маг"


def test_change_is_journalled(db_session, references):
    txn = _txn(db_session)
    A.apply_annotation_patch(db_session, txn, {"article": "р1"}, source="manual",
                             actor_user_id=7, reference_codes=references)
    db_session.commit()

    events = db_session.query(TransactionAnnotationEvent).all()
    assert len(events) == 1
    assert events[0].field == "article"
    assert events[0].before_value is None
    assert events[0].after_value == "р1"
    assert events[0].source == "manual"


# ------------------------------------------------------------ rule training

def test_rule_training_excludes_automation_written_rows(db_session, references):
    """A rule counting its own output would reach purity 1.0 by construction."""
    manual = _txn(db_session, message_id=1, operator_raw="SHOP A")
    imported = _txn(db_session, message_id=2, operator_raw="SHOP B")
    ruled = _txn(db_session, message_id=3, operator_raw="SHOP C")
    agented = _txn(db_session, message_id=4, operator_raw="SHOP D")

    for txn, source in ((manual, "manual"), (imported, "import"),
                        (ruled, "rule"), (agented, "agent")):
        A.apply_annotation_patch(db_session, txn, {"article": "р1"}, source=source,
                                 reference_codes=references)
    db_session.commit()

    operators = {op for op, _value in A.rows_for_rule_training(db_session, field="article")}
    assert operators == {"SHOP A", "SHOP B"}


# ------------------------------------------------------------------ importer

@pytest.mark.parametrize(
    "raw,expected",
    [
        ("➖ 40.000,00 UZS", Decimal("40000.00")),   # dot thousands, comma decimal
        ("➖ 5 000.00 UZS", Decimal("5000.00")),     # space thousands, dot decimal
        ("➕ 2.257.419,87 UZS", Decimal("2257419.87")),
        ("800000", Decimal("800000")),
        ("", None),
        (None, None),
    ],
)
def test_amount_parser_handles_both_conventions(raw, expected):
    """The workbook mixes decimal conventions between its two sheets."""
    assert I.parse_amount(raw) == expected


@pytest.mark.parametrize(
    "raw,expected",
    [("р1=ФЗП,ФМП,МП", "р1"), ("p7=фин.кред.опер.", "р7"), ("р3", "р3"), ("0", None), ("", None)],
)
def test_article_cell_yields_a_canonical_code(raw, expected):
    assert I.parse_article(raw) == expected


def test_matching_tolerates_a_truncated_operator(db_session):
    """The card sheet truncates operators to 20 chars, so ours is longer."""
    txn = _txn(db_session, operator_raw="BUSTONLIK BXO>Gazalkent filiali")
    row = I.WorkbookRow(
        sheet="HUMO Card", row_number=4,
        transaction_date=datetime(2026, 3, 2, 17, 14),
        operator="BUSTONLIK BXO>Gazalk",   # truncated, as the workbook stores it
        amount=Decimal("282800.00"),       # positive in the file, negative for us
        card_last4="4428",
        details="Андр._обнал в банком.", person="СТ",
        period=date(2026, 3, 1), article="р1",
    )
    matched, unmatched, ambiguous = I.match_rows(db_session, [row])
    assert not unmatched and not ambiguous
    assert matched[0][1].id == txn.id


def test_ambiguity_is_reported_not_guessed(db_session):
    """Two receipts of the same amount, minute and card cannot be told apart."""
    _txn(db_session, message_id=1, operator_raw="SHOP ONE")
    _txn(db_session, message_id=2, operator_raw="SHOP TWO")
    row = I.WorkbookRow(
        sheet="CardXabar", row_number=4,
        transaction_date=datetime(2026, 3, 2, 17, 14),
        operator=None, amount=Decimal("282800.00"), card_last4="4428",
        details="что-то", person=None, period=None, article=None,
    )
    matched, unmatched, ambiguous = I.match_rows(db_session, [row])
    assert not matched and not unmatched
    assert len(ambiguous[0][1]) == 2


def test_import_never_creates_a_transaction(db_session):
    row = I.WorkbookRow(
        sheet="CardXabar", row_number=4,
        transaction_date=datetime(2020, 1, 1, 0, 0),
        operator="NOBODY", amount=Decimal("1.00"), card_last4="0000",
        details="сирота", person=None, period=None, article=None,
    )
    before = db_session.query(Transaction).count()
    matched, unmatched, _ambiguous = I.match_rows(db_session, [row])
    assert not matched and len(unmatched) == 1
    assert db_session.query(Transaction).count() == before


def test_row_hash_changes_with_the_annotation(db_session):
    """The hash is what makes a re-import of an unchanged workbook a no-op."""
    base = dict(
        sheet="S", row_number=4, transaction_date=datetime(2026, 3, 2, 17, 14),
        operator="X", amount=Decimal("1.00"), card_last4="0001",
        person="СТ", period=date(2026, 3, 1), article="р1",
    )
    first = I.WorkbookRow(details="a", **base)
    same = I.WorkbookRow(details="a", **base)
    other = I.WorkbookRow(details="b", **base)

    assert first.row_hash(42) == same.row_hash(42)
    assert first.row_hash(42) != other.row_hash(42)
    assert first.row_hash(42) != first.row_hash(43)


# ------------------------------------------------------------------ filters

def _build(db_session, **overrides):
    """Call the shared query builder the way the agent tools do.

    They pass only the arguments they care about. Any new parameter must
    therefore carry a default, or `navigation_service` and `analytics_tools`
    break at runtime -- in code paths the HTTP tests never reach.
    """
    from api.routes.transactions import _build_filtered_transactions_query

    kwargs = dict(
        current_user={"user_id": 1, "role": "admin"},
        scope=None,
        date_from=None, date_to=None,
        operator=None, operators=None, app=None, apps=None,
        amount_min=None, amount_max=None,
        parsing_method=None, confidence_min=None, confidence_max=None,
        search=None, source_type=None, source_channel=None, source_chat_ids=None,
        transaction_type=None, transaction_types=None, currency=None,
        card=None, days_of_week=None,
    )
    kwargs.update(overrides)
    query, _notice = _build_filtered_transactions_query(db_session, **kwargs)
    return query


def test_builder_still_callable_without_annotation_arguments(db_session, references):
    """Guards the agent call sites, which name every argument explicitly."""
    _annotate_fixture(db_session, references)
    assert _build(db_session).count() == 3


def _annotate_fixture(db_session, references):
    a = _txn(db_session, message_id=1, operator_raw="SHOP A")
    b = _txn(db_session, message_id=2, operator_raw="SHOP B")
    c = _txn(db_session, message_id=3, operator_raw="SHOP C")
    A.apply_annotation_patch(db_session, a, {"details": "ст парковка", "article": "р1", "person": "СТ",
                                             "period": date(2026, 3, 1)},
                             source="manual", reference_codes=references)
    A.apply_annotation_patch(db_session, b, {"details": "вп маг", "article": "р3", "person": "ДиК",
                                             "period": date(2026, 4, 1)},
                             source="manual", reference_codes=references)
    db_session.commit()
    return a, b, c


def test_filter_by_article_folds_latin_input(db_session, references):
    _annotate_fixture(db_session, references)
    assert _build(db_session, annotation_article="р1").count() == 1
    # A Latin 'p' must resolve to the same reference entry.
    assert _build(db_session, annotation_article="p1").count() == 1


def test_filter_by_multiple_articles(db_session, references):
    _annotate_fixture(db_session, references)
    assert _build(db_session, annotation_articles="р1,р3").count() == 2


def test_filter_by_details_substring(db_session, references):
    _annotate_fixture(db_session, references)
    assert _build(db_session, annotation_details="парковка").count() == 1


def test_filter_by_person_and_period(db_session, references):
    _annotate_fixture(db_session, references)
    assert _build(db_session, annotation_person="СТ").count() == 1
    assert _build(db_session, annotation_period_from=date(2026, 4, 1)).count() == 1


def test_filter_by_annotation_state(db_session, references):
    _annotate_fixture(db_session, references)
    assert _build(db_session, annotation_state="annotated").count() == 2
    assert _build(db_session, annotation_state="missing").count() == 1


def test_global_search_also_covers_the_annotation(db_session, references):
    """The note is what a person remembers; raw_message alone is not enough."""
    _annotate_fixture(db_session, references)
    assert _build(db_session, search="парковка").count() == 1


def test_annotation_columns_are_sortable(db_session):
    from api.routes.transactions import _resolve_transactions_sort_column
    from database.models import Transaction as T

    assert _resolve_transactions_sort_column("annotation_article_code") is T.annotation_article_code
    assert _resolve_transactions_sort_column("annotation_period") is T.annotation_period


class TestUnknownCodeRegistration:
    """
    A code the workbook uses but the dictionaries have never seen used to be
    written onto the transaction and then vanish: it matched no dictionary entry,
    so the grid showed a bare code and no report mentioned it. `is_auto` was
    plumbed all the way to the API contract for exactly this and never set.
    """

    def test_unknown_article_is_registered_as_auto(self, db_session):
        from database.models import AnnotationArticle
        from services import annotation_service

        registered = annotation_service.register_auto_codes(
            db_session, articles={"р99"}, persons=set(),
        )
        assert registered == [{"kind": "article", "code": "р99"}]

        row = db_session.query(AnnotationArticle).filter_by(code="р99").one()
        assert row.is_auto is True
        assert row.label == ""

    def test_direction_is_taken_from_the_prefix(self, db_session):
        from database.models import AnnotationArticle
        from services import annotation_service

        annotation_service.register_auto_codes(db_session, articles={"п3", "р3"})
        assert db_session.query(AnnotationArticle).filter_by(code="п3").one().direction == "income"
        assert db_session.query(AnnotationArticle).filter_by(code="р3").one().direction == "expense"

    def test_unknown_person_is_registered(self, db_session):
        from database.models import AnnotationPerson
        from services import annotation_service

        annotation_service.register_auto_codes(db_session, persons={"НовЛицо"})
        assert db_session.query(AnnotationPerson).filter_by(code="НовЛицо").one().is_auto is True

    def test_registering_nothing_is_a_no_op(self, db_session):
        from services import annotation_service

        assert annotation_service.register_auto_codes(db_session) == []

    def test_registered_codes_become_known(self, db_session):
        """The import reloads the dictionaries afterwards, so the very rows that
        introduced a code can then be written with it."""
        from services import annotation_service

        annotation_service.register_auto_codes(db_session, articles={"р77"})
        codes = annotation_service.load_reference_codes(db_session)
        assert "р77" in codes["article"]
