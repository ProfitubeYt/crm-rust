"""«Подробности» from confirmed similar operations. Synthetic receipts only.

What these pin: a note is learned from people and the workbook alone, per
operator, own card and direction, only while the latest confirmed receipts
agree; it is copied whole; nothing a person wrote moves; a wrong match does
not spread and does not outlive the evidence against it.
"""
from __future__ import annotations

import uuid
from datetime import date, datetime, timedelta
from decimal import Decimal

import pytest

from database.models import AnnotationRule, LockedPeriod, Transaction, TransactionAnnotationEvent
from services import annotation_rule_service as R
from services import annotation_service as A

GATE = "GATE PAY TO VISA, UZ"
START = datetime(2026, 3, 1, 12, 0)


@pytest.fixture(autouse=True)
def _clear_rule_cache():
    R.invalidate_rule_cache()
    yield
    R.invalidate_rule_cache()


def _receipt(db, n, *, operator=GATE, card="1111", kind="DEBIT", note=None, source=None, when=None):
    txn = Transaction(
        uuid=uuid.uuid4(), raw_message="synthetic", source_type="AUTO", source_chat_id=1, source_message_id=n,
        transaction_date=when or START + timedelta(days=n), amount=Decimal("-1000.00") if kind == "DEBIT" else Decimal("1000.00"),
        currency="UZS", card_last_4=card, operator_raw=operator, transaction_type=kind,
    )
    db.add(txn)
    db.flush()
    if note is not None:
        A.apply_annotation_patch(db, txn, {"details": note}, source=source or "manual", validate=False)
    db.commit()
    return txn


def _confirmed(db, count, note, *, first=1, **kwargs):
    return [_receipt(db, first + i, note=note, **kwargs) for i in range(count)]


def _learn_and_apply(db, **kwargs):
    R.recompute(db, fields=("details",))
    return R.apply_rules(db, fields=("details",), **kwargs)


# --- the decision itself -----------------------------------------------------------

@pytest.mark.parametrize("history, tier, reason", [
    ([], "none", "no_confirmed_similar"),
    (["a", "a"], "none", "too_few_confirmed"),
    (["a", "a", "a"], "suggestion", "few_confirmed"),
    (["a", "a", "a", "a"], "suggestion", "few_confirmed"),
    (["a", "a", "a", "a", "a"], "confident", "confirmed_similar"),
    (["x", "a", "a", "a", "a"], "confident", "confirmed_similar"),            # 4 of 5
    (["x", "y", "a", "a", "a"], "suggestion", "mixed_history"),               # 3 of 5
    (["x", "y", "z"] + ["a"] * 9, "suggestion", "mixed_history"),             # 9 of 12
    (["x", "y"] + ["a"] * 8, "confident", "confirmed_similar"),               # 8 of 10
    (["a", "a", "a", "a", "a", "b"], "none", "recent_values_differ"),         # the pattern just broke
    (["a", "a", "a", "a", "b", "a"], "none", "recent_values_differ"),
    (["a", "a", "a", "b", "b", "b"], "suggestion", "mixed_history"),          # a new note, not yet a rule
])
def test_a_note_needs_support_purity_and_recent_agreement(history, tier, reason):
    decision = R.details_decision(history)
    assert (decision.tier, decision.reason) == (tier, reason)
    assert (decision.value is not None) == (tier != "none")


def test_spelling_variants_are_one_note_and_the_latest_spelling_is_kept():
    history = ["вн.пер.на 1234", "Вн. пер. на1234", "вн пер на 1234", "вн.пер.на 1234", "Вн.пер.на 1234"]
    decision = R.details_decision(history)
    assert (decision.tier, decision.support, decision.total) == ("confident", 5, 5)
    assert decision.value == "Вн.пер.на 1234"


# --- learning and filling ------------------------------------------------------------

def test_a_confirmed_pattern_fills_the_same_operator_card_and_direction(db_session):
    _confirmed(db_session, 5, "internal top-up")
    target = _receipt(db_session, 50)

    result = _learn_and_apply(db_session)

    rule = db_session.query(AnnotationRule).one()
    assert rule.operator_key == "GATE PAY TO VISA UZ | ПК 1111 | DEBIT"
    assert (rule.field, rule.value, rule.support, float(rule.purity)) == ("details", "internal top-up", 5, 1.0)
    db_session.refresh(target)
    assert (target.annotation_details, target.annotation_details_src) == ("internal top-up", "rule")
    assert result["updated"] == 1


def test_similar_looking_receipts_do_not_share_a_note(db_session):
    _confirmed(db_session, 6, "internal top-up")
    other_card = _receipt(db_session, 50, card="2222")
    other_direction = _receipt(db_session, 51, kind="CREDIT")
    longer_operator = _receipt(db_session, 52, operator="GATE PAY TO VISA NEW, UZ")
    shorter_operator = _receipt(db_session, 53, operator="GATE PAY, UZ")

    result = _learn_and_apply(db_session)

    assert result["updated"] == 0
    for txn in (other_card, other_direction, longer_operator, shorter_operator):
        db_session.refresh(txn)
        assert txn.annotation_details is None and txn.annotation_details_src is None


def test_a_label_with_an_identifier_is_copied_whole_and_never_adapted(db_session):
    # The number in such a label is opaque: it is not read and not rewritten.
    _confirmed(db_session, 5, "мп ДиК 0626")
    same = _receipt(db_session, 50)
    other_card = _receipt(db_session, 51, card="0626")

    _learn_and_apply(db_session)

    db_session.refresh(same)
    db_session.refresh(other_card)
    assert same.annotation_details == "мп ДиК 0626"
    assert other_card.annotation_details is None


def test_weaker_evidence_is_not_written(db_session):
    _confirmed(db_session, 4, "four times is a suggestion")
    _confirmed(db_session, 5, "old note", first=20, card="2222")
    _receipt(db_session, 26, card="2222", note="new note")          # the pattern just broke
    few = _receipt(db_session, 50)
    broken = _receipt(db_session, 51, card="2222")

    result = _learn_and_apply(db_session)

    assert result["updated"] == 0 and db_session.query(AnnotationRule).count() == 0
    for txn in (few, broken):
        db_session.refresh(txn)
        assert txn.annotation_details is None


def test_the_system_never_learns_from_its_own_notes(db_session):
    _confirmed(db_session, 6, "a rule wrote this", source="rule")
    _confirmed(db_session, 6, "the agent wrote this", first=20, card="2222", source="agent")
    target = _receipt(db_session, 50)

    result = _learn_and_apply(db_session)

    assert db_session.query(AnnotationRule).count() == 0 and result["updated"] == 0
    db_session.refresh(target)
    assert target.annotation_details is None


def test_one_false_match_does_not_become_evidence(db_session):
    _confirmed(db_session, 5, "internal top-up")
    filled = [_receipt(db_session, 50 + i) for i in range(20)]
    _learn_and_apply(db_session)

    # Twenty rule-written copies later the rule still stands on the five confirmed ones.
    R.recompute(db_session, fields=("details",))
    rule = db_session.query(AnnotationRule).one()
    assert (rule.support, float(rule.purity)) == (5, 1.0)
    assert all(db_session.get(Transaction, txn.id).annotation_details_src == "rule" for txn in filled)


def test_what_a_person_or_the_workbook_wrote_is_never_replaced(db_session):
    _confirmed(db_session, 5, "internal top-up")
    manual = _receipt(db_session, 50, note="my own note", source="manual")
    imported = _receipt(db_session, 51, note="workbook note", source="import")

    _learn_and_apply(db_session, revalidate=True)

    db_session.refresh(manual)
    db_session.refresh(imported)
    assert (manual.annotation_details, manual.annotation_details_src) == ("my own note", "manual")
    assert (imported.annotation_details, imported.annotation_details_src) == ("workbook note", "import")


def test_locked_periods_are_left_alone(db_session):
    _confirmed(db_session, 5, "internal top-up")
    db_session.add(LockedPeriod(date_from=date(2026, 10, 1), date_to=date(2026, 10, 31), locked_by_tg_id=1))
    db_session.commit()
    locked = _receipt(db_session, 50, when=datetime(2026, 10, 20, 12, 0))
    open_ = _receipt(db_session, 51, when=datetime(2026, 11, 5, 12, 0))
    stale = _receipt(db_session, 52, card="9999", note="stale", source="rule", when=datetime(2026, 10, 21, 12, 0))

    result = _learn_and_apply(db_session, revalidate=True)

    for txn in (locked, open_, stale):
        db_session.refresh(txn)
    assert locked.annotation_details is None
    assert open_.annotation_details == "internal top-up"
    assert (stale.annotation_details, stale.annotation_details_src) == ("stale", "rule")
    assert result["skipped_locked"] == 2 and result["retracted"] == 0


def test_a_second_run_changes_nothing(db_session):
    _confirmed(db_session, 5, "internal top-up")
    [_receipt(db_session, 50 + i) for i in range(3)]

    first = _learn_and_apply(db_session, revalidate=True)
    events = db_session.query(TransactionAnnotationEvent).count()
    second = _learn_and_apply(db_session, revalidate=True)

    assert first["updated"] == 3
    assert (second["updated"], second["changed_fields"], second["retracted"]) == (0, 0, 0)
    assert db_session.query(TransactionAnnotationEvent).count() == events
    assert db_session.query(AnnotationRule).count() == 1


# --- a wrong match does not outlive the evidence against it -----------------------------

def test_revalidate_takes_back_a_note_the_confirmed_receipts_no_longer_support(db_session):
    _confirmed(db_session, 5, "internal top-up")
    written = _receipt(db_session, 50)
    _learn_and_apply(db_session)
    db_session.refresh(written)
    assert written.annotation_details_src == "rule"

    # People then confirm something else for the same operator, card and direction.
    _receipt(db_session, 60, note="salary")
    _receipt(db_session, 61, note="rent")

    untouched = _learn_and_apply(db_session)                      # not asked: nothing is taken back
    db_session.refresh(written)
    assert untouched["retracted"] == 0 and written.annotation_details == "internal top-up"
    assert db_session.query(AnnotationRule).filter_by(is_enabled=True).count() == 0

    result = _learn_and_apply(db_session, revalidate=True)
    db_session.refresh(written)
    assert result["retracted"] == 1
    assert written.annotation_details is None and written.annotation_details_src is None

    event = (
        db_session.query(TransactionAnnotationEvent)
        .filter_by(transaction_id=written.id, field="details")
        .order_by(TransactionAnnotationEvent.id.desc())
        .first()
    )
    assert (event.before_value, event.after_value, event.before_src, event.source) == (
        "internal top-up", None, "rule", "rule",
    )


def test_revalidate_corrects_a_note_written_by_an_older_rule(db_session):
    _confirmed(db_session, 5, "internal top-up")
    legacy = _receipt(db_session, 50, note="what the operator-only rule wrote", source="rule")
    agent = _receipt(db_session, 51, note="what the agent wrote", source="agent")

    result = _learn_and_apply(db_session, revalidate=True)

    db_session.refresh(legacy)
    db_session.refresh(agent)
    assert (legacy.annotation_details, legacy.annotation_details_src) == ("internal top-up", "rule")
    assert agent.annotation_details == "what the agent wrote"       # not a rule's note: not rechecked
    assert (result["updated"], result["retracted"]) == (1, 0)


# --- rules of earlier releases and rules people pinned -----------------------------------

def test_an_operator_only_note_rule_of_an_earlier_release_is_no_longer_used(db_session):
    db_session.add(AnnotationRule(operator_key="GATE PAY TO VISA UZ", field="details", value="legacy", support=3, purity=1))
    db_session.commit()
    target = _receipt(db_session, 50)

    assert R.apply_rules(db_session, fields=("details",))["updated"] == 0
    db_session.refresh(target)
    assert target.annotation_details is None

    R.recompute(db_session, fields=("details",))
    assert db_session.query(AnnotationRule).one().is_enabled is False     # kept as history, switched off


def test_a_note_rule_a_person_pinned_to_an_operator_stands(db_session):
    db_session.add(AnnotationRule(
        operator_key="GATE PAY TO VISA UZ", field="details", value="pinned by hand", support=0, purity=1, is_pinned=True,
    ))
    db_session.commit()
    _confirmed(db_session, 5, "learned note")
    target = _receipt(db_session, 50, card="7777")

    _learn_and_apply(db_session, revalidate=True)

    db_session.refresh(target)
    assert target.annotation_details == "pinned by hand"
    assert db_session.query(AnnotationRule).filter_by(is_pinned=True).one().is_enabled is True


def test_codes_still_follow_the_operator_alone(db_session):
    for n in range(3):
        txn = _receipt(db_session, n + 1, card=f"{n}{n}{n}{n}")
        A.apply_annotation_patch(db_session, txn, {"article": "р1"}, source="manual", validate=False)
    db_session.commit()
    R.recompute(db_session, fields=("article",))
    assert db_session.query(AnnotationRule).one().operator_key == "GATE PAY TO VISA UZ"


def test_new_receipts_get_the_note_from_the_background_pass(db_session):
    _confirmed(db_session, 5, "internal top-up")
    R.recompute(db_session, fields=("details",))
    same = _receipt(db_session, 50)
    other_card = _receipt(db_session, 51, card="2222")

    R.fill_defaults(db_session)

    db_session.refresh(same)
    db_session.refresh(other_card)
    assert (same.annotation_details, same.annotation_details_src) == ("internal top-up", "rule")
    assert other_card.annotation_details is None
    when = datetime(2026, 11, 10, 9, 0)
    assert R.defaults_for_new(
        db_session, operator_raw=GATE, transaction_date=when, card_last_4="1111", transaction_type="DEBIT",
    )["details"] == "internal top-up"
    assert "details" not in R.defaults_for_new(db_session, operator_raw=GATE, transaction_date=when)


# --- the dry run ---------------------------------------------------------------------------

def test_the_dry_run_counts_everything_and_writes_nothing(db_session):
    _confirmed(db_session, 5, "internal top-up")                                    # a rule
    _confirmed(db_session, 3, "three times", first=10, card="2222")                 # a suggestion
    _confirmed(db_session, 2, "twice", first=20, card="3333")                       # too few
    for n, card in enumerate(["1111", "1111", "2222", "3333", "4444"], start=50):
        _receipt(db_session, n, card=card)                                          # empty ones
    _receipt(db_session, 60, note="internal top-up", source="rule")                 # still supported
    _receipt(db_session, 61, card="4444", note="unsupported", source="rule")        # would be taken back
    _receipt(db_session, 62, note="Internal top-up!", source="agent")               # the agent agrees
    _receipt(db_session, 63, note="something else", source="agent")                 # the agent differs
    before = db_session.query(TransactionAnnotationEvent).count()

    report = R.details_report(db_session)

    assert (report["receipts"], report["confirmed"], report["keys"], report["rules"]) == (19, 10, 3, 1)
    assert (report["empty"], report["would_fill"], report["locked"]) == (5, 2, 0)
    assert report["suggestions"] == {"few_confirmed": 1}
    assert report["unresolved"] == {"too_few_confirmed": 1, "no_confirmed_similar": 1}
    assert (report["rule_written"], report["rule_supported"], report["rule_would_retract"]) == (2, 1, 1)
    assert (report["agent_written"], report["agent_agrees"], report["agent_differs"]) == (2, 1, 1)
    assert db_session.query(AnnotationRule).count() == 0
    assert db_session.query(TransactionAnnotationEvent).count() == before

    text = "\n".join(R.details_report_lines(report))
    assert "would be filled:                     2" in text
    # Counts and reasons only: no note, operator or card reaches the report.
    for private in ("internal top-up", "GATE", "1111", "something else"):
        assert private not in text


def test_the_dry_run_agrees_with_what_apply_then_does(db_session):
    _confirmed(db_session, 5, "internal top-up")
    for n in range(50, 54):
        _receipt(db_session, n)
    _receipt(db_session, 60, card="4444", note="unsupported", source="rule")

    report = R.details_report(db_session)
    applied = _learn_and_apply(db_session, revalidate=True)

    assert report["would_fill"] == 4 and report["rule_would_retract"] == 1
    assert (applied["updated"], applied["retracted"]) == (5, 1)
    after = R.details_report(db_session)
    assert (after["would_fill"], after["rule_would_retract"], after["rule_would_change"]) == (0, 0, 0)
    assert after["rule_supported"] == 4
