"""Who wrote «Приложение»: every path that sets it must leave a true stamp.

Synthetic data only. The API fixtures come from the modules that own them.
"""
from __future__ import annotations

import asyncio
from datetime import datetime
from decimal import Decimal

import pytest

from database.models import OperatorReference, Transaction
from parsers.operator_mapper import invalidate_operator_mapper_cache
from parsers.parser_orchestrator import ParserOrchestrator
from services import receipt_processor
from services.application_detection import invalidate_confirmed_cache

from test_excel_sync_service import apply, create, grant, tx, update  # noqa: F401  (fixtures and helpers)
from test_locked_periods import client  # noqa: F401  (fixture)


@pytest.fixture(autouse=True)
def _fresh_caches():
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()
    yield
    invalidate_operator_mapper_cache()
    invalidate_confirmed_cache()


def _dictionary(db):
    db.add(OperatorReference(operator_name="HAMKORBANK ATB, UZ", application_name="Hamkor", is_p2p=True, is_active=True))
    db.commit()


def _receipt(headline, operator):
    return (
        f"🔴 {headline}\n➖ 40 000.00 UZS\n💳 ***0001\n📍 {operator}\n"
        "🕓 14.04.26 23:11\n💵 1 000 000.00 UZS"
    )


# --- at parse time -----------------------------------------------------------------

def test_a_parsed_receipt_carries_the_evidence_tier_of_its_application(db_session):
    _dictionary(db_session)
    orchestrator = ParserOrchestrator(db_session=db_session)

    owned = orchestrator.process(_receipt("Spisanie c karty", "HAMKORBANK ATB, UZ"))
    assert (owned["application_mapped"], owned["application_src"], owned["application_confidence"]) == (
        "Hamkor", "dict", 1.0,
    )
    assert owned["is_p2p"] is True
    assert owned["app_resolution"]["method"] == "DICT" and owned["app_resolution"]["match_type"] == "EXACT"

    named = orchestrator.process(_receipt("E-Com oplata", "PAYME OPLATA, UZ"))
    assert (named["application_mapped"], named["application_src"], named["application_confidence"]) == (
        "Payme", "rule", 0.9,
    )
    assert named["app_resolution"] == {"method": "RULE", "reason": "brand_in_operator"}

    shop = orchestrator.process(_receipt("Pokupka", "SOME SHOP MCHJ, TOSHKENT, SOME STREET 1"))
    assert shop["application_mapped"] is None
    assert "application_src" not in shop
    assert shop["app_resolution"] == {"method": "HEURISTIC"}


# --- edited by a person ---------------------------------------------------------------

def test_an_application_typed_in_the_workbook_becomes_manual(db_session, grant, tx):  # noqa: F811
    tx.application_mapped = "Guessed"
    tx.application_src = "rule"
    db_session.commit()

    result = apply(db_session, grant, update(tx, {"application_mapped": "Typed in Excel"}))
    assert result["status"] == "applied"
    assert (tx.application_mapped, tx.application_src) == ("Typed in Excel", "manual")

    # Emptied by a person: still theirs, so no pass fills it back in.
    result = apply(db_session, grant, update(tx, {"application_mapped": None}))
    assert result["status"] == "applied"
    assert (tx.application_mapped, tx.application_src) == (None, "manual")


def test_an_untouched_application_keeps_its_stamp_through_a_workbook_edit(db_session, grant, tx):  # noqa: F811
    tx.application_mapped = "Payme"
    tx.application_src = "rule"
    db_session.commit()
    result = apply(db_session, grant, update(tx, {"operator_raw": "Changed merchant"}))
    assert result["status"] == "applied"
    assert (tx.application_mapped, tx.application_src) == ("Payme", "rule")


def test_a_row_created_in_the_workbook_with_an_application_is_manual(db_session, grant):  # noqa: F811
    result = apply(db_session, grant, create(application_mapped="Typed in Excel"))
    assert result["status"] == "applied"
    row = db_session.query(Transaction).filter_by(operator_raw="Created merchant").one()
    assert (row.application_mapped, row.application_src) == ("Typed in Excel", "manual")


def _api_tx(db_session, **overrides):
    values = dict(
        raw_message="synthetic", source_type="AUTO", source_chat_id=555, source_message_id=1,
        transaction_date=datetime(2026, 3, 10, 10, 0), amount=Decimal("-100.00"), currency="UZS",
        card_last_4="0001", operator_raw="PAYME OPLATA, UZ", transaction_type="DEBIT",
        parsing_method="REGEX_SMS",
    )
    values.update(overrides)
    row = Transaction(**values)
    db_session.add(row)
    db_session.commit()
    db_session.refresh(row)
    return row


def test_the_api_marks_an_edited_application_manual_and_reports_the_stamp(client, db_session):  # noqa: F811
    row = _api_tx(db_session, application_mapped="Payme", application_src="rule", application_confidence=Decimal("0.9"))

    shown = client.get(f"/api/transactions/{row.id}")
    assert shown.status_code == 200, shown.text
    assert (shown.json()["application_src"], shown.json()["application_confidence"]) == ("rule", 0.9)

    # Sending the same value back is not an edit.
    same = client.put(f"/api/transactions/{row.id}", json={"application_mapped": "Payme"})
    assert same.status_code == 200, same.text
    db_session.refresh(row)
    assert row.application_src == "rule"

    edited = client.put(f"/api/transactions/{row.id}", json={"application_mapped": "Mine"})
    assert edited.status_code == 200, edited.text
    db_session.refresh(row)
    assert (row.application_mapped, row.application_src, row.application_confidence) == ("Mine", "manual", None)
    assert client.get(f"/api/transactions/{row.id}").json()["application_src"] == "manual"


def test_the_bulk_edit_marks_applications_manual(client, db_session):  # noqa: F811
    row = _api_tx(db_session)
    done = client.patch(
        "/api/transactions/bulk-update",
        json={"updates": [{"id": row.id, "fields": {"application_mapped": "Mine"}}]},
    )
    assert done.status_code == 200, done.text
    db_session.refresh(row)
    assert (row.application_mapped, row.application_src) == ("Mine", "manual")


def test_a_receipt_created_by_hand_is_manual_or_detected(client, db_session):  # noqa: F811
    body = {
        "datetime": "2026-03-20T12:00:00", "amount": "150000", "card_last4": "0001",
        "transaction_type": "DEBIT", "currency": "UZS",
    }
    typed = client.post("/api/transactions/", json={**body, "operator": "Some operator", "app": "Demo"})
    assert typed.status_code == 200, typed.text
    assert (typed.json()["application_mapped"], typed.json()["application_src"]) == ("Demo", "manual")

    detected = client.post("/api/transactions/", json={**body, "operator": "XAZNA PAYMENTS, UZ"})
    assert detected.status_code == 200, detected.text
    assert (detected.json()["application_mapped"], detected.json()["application_src"]) == ("Xazna", "rule")

    empty = client.post("/api/transactions/", json={**body, "operator": "SOME SHOP MCHJ, TOSHKENT"})
    assert empty.status_code == 200, empty.text
    assert (empty.json()["application_mapped"], empty.json()["application_src"]) == (None, None)


# --- a forced re-parse ------------------------------------------------------------------

class _Manager:
    async def get_message(self, chat_id, message_id):  # noqa: ARG002
        return {"text": "receipt", "date": 1_768_214_400}


def _reparse(db_session, monkeypatch, *, stored, source, parsed_application):
    row = _api_tx(db_session, source_chat_id=5, source_message_id=50, application_mapped=stored, application_src=source)
    parsed = {
        "transaction_date": datetime(2026, 3, 10, 10, 0), "amount": Decimal("100.00"), "currency": "UZS",
        "card_last_4": "0001", "operator_raw": "PAYME OPLATA, UZ", "transaction_type": "DEBIT",
        "parsing_method": "REGEX_SMS", "parsing_confidence": 0.95,
        "application_mapped": parsed_application,
        "application_src": "rule" if parsed_application else None,
        "application_confidence": 0.9 if parsed_application else None,
    }
    monkeypatch.setattr(receipt_processor, "_parse_text_in_worker", lambda *_: parsed)
    asyncio.run(receipt_processor.process_tdlib_message(5, 50, True, db_session, _Manager()))
    db_session.expire_all()
    row = db_session.get(Transaction, row.id)
    return row.application_mapped, row.application_src


@pytest.mark.parametrize("stored, source, parsed_application, expected", [
    ("Mine", "manual", "Payme", ("Mine", "manual")),       # a person's value survives
    (None, "manual", "Payme", (None, "manual")),           # so does a person's "none"
    ("Legacy", None, "Payme", ("Legacy", None)),           # unknown origin may be a person's
    ("Legacy", None, None, ("Legacy", None)),              # and is not blanked either
    ("Old guess", "ai", "Payme", ("Payme", "rule")),       # automation's value is re-derived
    ("Payme", "rule", None, (None, None)),                 # ... or cleared when nothing supports it
    (None, None, "Payme", ("Payme", "rule")),              # an empty cell is filled
])
def test_a_forced_reparse_replaces_only_what_automation_wrote(
    db_session, monkeypatch, stored, source, parsed_application, expected,
):
    assert _reparse(db_session, monkeypatch, stored=stored, source=source, parsed_application=parsed_application) == expected


# --- the optional AI opinion --------------------------------------------------------

class _Ai:
    """Stands in for the provider: no network, records what it was shown."""

    enabled = True

    def __init__(self, answer=None, error=None):
        self.answer, self.error, self.calls = answer, error, []

    def resolve_application(self, operator_raw, raw_text, known_apps, hints):
        self.calls.append({"operator": operator_raw, "text": raw_text, "known": list(known_apps), "hints": hints})
        if self.error:
            raise self.error
        return self.answer


def _with_ai(db_session, monkeypatch, ai):
    monkeypatch.setenv("AI_APPLICATION_MAPPING_ENABLED", "true")
    _dictionary(db_session)
    orchestrator = ParserOrchestrator(db_session=db_session)
    orchestrator.ai_parser = ai
    return orchestrator


def test_without_ai_the_rules_work_and_nothing_is_asked(db_session, monkeypatch):
    monkeypatch.delenv("AI_APPLICATION_MAPPING_ENABLED", raising=False)
    _dictionary(db_session)
    orchestrator = ParserOrchestrator(db_session=db_session)
    orchestrator.ai_parser = ai = _Ai(error=AssertionError("AI must stay off unless switched on"))
    assert orchestrator.process(_receipt("E-Com oplata", "PAYME OPLATA, UZ"))["application_mapped"] == "Payme"
    assert orchestrator.process(_receipt("E-Com oplata", "VIA PAYMENT, UZ"))["application_mapped"] is None
    assert ai.calls == []


def test_ai_is_not_asked_when_the_operator_line_already_answers(db_session, monkeypatch):
    ai = _Ai(error=AssertionError("evidence first: no paid call for a line the rules resolve"))
    orchestrator = _with_ai(db_session, monkeypatch, ai)
    assert orchestrator.process(_receipt("E-Com oplata", "PAYME OPLATA, UZ"))["application_src"] == "rule"
    assert orchestrator.process(_receipt("Spisanie c karty", "HAMKORBANK ATB, UZ"))["application_src"] == "dict"
    assert ai.calls == []


@pytest.mark.parametrize("operator", [
    "XAZNA PAYNET TOLOV>T",                                  # ambiguous: not the model's call
    "XKKM 00014 ATM 000001, UZ",                             # cash
    "YATT SOMEBODY SOMEONE, TOSHKENT, SOME STREET 4",        # a person's name and address
    "SOME SHOP MCHJ, TOSHKENT, SOME DISTRICT, SOME STREET",  # an address
])
def test_ai_is_never_shown_an_ambiguous_cash_or_personal_line(db_session, monkeypatch, operator):
    ai = _Ai(answer={"application_name": "Payme", "confidence": 0.99, "is_p2p": False})
    result = _with_ai(db_session, monkeypatch, ai).process(_receipt("Pokupka", operator))
    assert result["application_mapped"] is None and "application_src" not in result
    assert ai.calls == []


def test_ai_sees_the_operator_line_and_the_vocabulary_and_nothing_else(db_session, monkeypatch):
    ai = _Ai(answer={"application_name": "hamkor", "confidence": 0.9, "is_p2p": True})
    result = _with_ai(db_session, monkeypatch, ai).process(_receipt("Spisanie c karty", "VIA PAYMENT, UZ"))

    (call,) = ai.calls
    assert call["operator"] == "VIA PAYMENT, UZ"
    assert call["text"] == ""                      # no card, amount, balance or receipt text
    assert "Hamkor" in call["known"] and "Payme" in call["known"]
    # The model's spelling is folded to the owner's; the stamp says who it was.
    assert (result["application_mapped"], result["application_src"], result["application_confidence"]) == (
        "Hamkor", "ai", 0.9,
    )
    assert result["app_resolution"]["method"] == "AI"


def test_an_ai_name_outside_the_vocabulary_is_a_suggestion_not_a_value(db_session, monkeypatch):
    ai = _Ai(answer={"application_name": "Brand New App", "confidence": 0.95, "is_p2p": False,
                     "recommended_operator_name": "VIA PAYMENT"})
    result = _with_ai(db_session, monkeypatch, ai).process(_receipt("E-Com oplata", "VIA PAYMENT, UZ"))
    assert result["application_mapped"] is None and "application_src" not in result
    assert result["app_resolution"]["method"] == "AI_SUGGESTION"
    assert result["operator_reference_suggestion"] == {
        "operator_name": "VIA PAYMENT", "application_name": "Brand New App", "is_p2p": False,
    }


@pytest.mark.parametrize("answer", [
    None,                                                                   # the provider returned nothing
    {"application_name": "Unknown", "confidence": 0.99, "is_p2p": False},   # insufficient evidence
    {"application_name": "Hamkor", "confidence": 0.4, "is_p2p": False},     # not confident
])
def test_an_unsure_ai_leaves_the_application_empty(db_session, monkeypatch, answer):
    result = _with_ai(db_session, monkeypatch, _Ai(answer=answer)).process(_receipt("E-Com oplata", "VIA PAYMENT, UZ"))
    assert result["application_mapped"] is None and "application_src" not in result
    assert result["app_resolution"] == {"method": "HEURISTIC"}


def test_an_ai_failure_does_not_lose_the_receipt(db_session, monkeypatch):
    ai = _Ai(error=TimeoutError("provider timed out"))
    result = _with_ai(db_session, monkeypatch, ai).process(_receipt("E-Com oplata", "VIA PAYMENT, UZ"))
    assert len(ai.calls) == 1
    assert result is not None and result["operator_raw"] == "VIA PAYMENT, UZ"
    assert result["application_mapped"] is None and "application_src" not in result


def test_revalidation_replaces_an_ai_guess_only_with_evidence(db_session):
    from services.application_backfill import backfill_applications

    _dictionary(db_session)
    guessed = _api_tx(db_session, source_message_id=1, operator_raw="PAYME OPLATA, UZ",
                      application_mapped="Humans", application_src="ai")
    unsupported = _api_tx(db_session, source_message_id=2, operator_raw="VIA PAYMENT, UZ",
                          application_mapped="Humans", application_src="ai")
    report = backfill_applications(db_session, apply=True, revalidate=True)
    db_session.refresh(guessed)
    db_session.refresh(unsupported)
    assert report.revalidated == 1
    assert (guessed.application_mapped, guessed.application_src) == ("Payme", "rule")
    assert (unsupported.application_mapped, unsupported.application_src) == ("Humans", "ai")
