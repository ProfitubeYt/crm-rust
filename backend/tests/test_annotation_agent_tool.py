"""Tests for the annotation suggestion tool and its confirmation path.

The tool must never write on its own, must refuse anything the model invents,
and must re-check authorization when the approved batch is finally applied.
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
from services.ai_agent.tools import annotation_tools as T


@pytest.fixture
def references(db_session):
    db_session.add_all([
        AnnotationArticle(code="р1", label="ФЗП,ФМП,МП", direction="expense", sort_order=1),
        AnnotationArticle(code="р3", label="МП,благотвор.", direction="expense", sort_order=3),
        AnnotationPerson(code="СТ", sort_order=1),
    ])
    db_session.commit()
    return A.load_reference_codes(db_session)


def _txn(db_session, operator, *, annotated=None, day=2, message_id=None):
    txn = Transaction(
        uuid=uuid.uuid4(), raw_message=f"receipt {operator}", source_type="AUTO",
        source_chat_id=1,
        source_message_id=message_id if message_id is not None else int(uuid.uuid4().int % 10**8),
        transaction_date=datetime(2026, 3, day, 12, 0),
        amount=Decimal("-1000.00"), currency="UZS", card_last_4="1111",
        operator_raw=operator, transaction_type="DEBIT",
    )
    db_session.add(txn)
    db_session.flush()
    if annotated:
        A.apply_annotation_patch(db_session, txn, annotated, source="manual", validate=False)
    db_session.commit()
    return txn


# ------------------------------------------------------------------ retrieval

def test_retrieval_prefers_the_same_merchant(db_session, references):
    for i in range(3):
        _txn(db_session, "ANGLESEY FOOD", annotated={"article": "р1", "details": "вп маг"}, day=i + 1)
    target = _txn(db_session, "ANGLESEY FOOD", day=20)

    corpus = T._load_annotated_corpus(db_session)
    context = T.build_context([target], corpus)[int(target.id)]

    assert context["exact_history"] is True
    assert len(context["neighbours"]) == 3
    assert context["stats"]["article"] == {"р1": 3}


def test_retrieval_falls_back_to_similar_names(db_session, references):
    """74 of the client's merchants appear once; the name is all there is."""
    _txn(db_session, "OOO AGAT SYSTEM TASHKENT", annotated={"article": "р1"}, day=1)
    _txn(db_session, "OOO AGAT SYSTEM CHILANZAR", annotated={"article": "р1"}, day=2)
    target = _txn(db_session, "OOO AGAT SYSTEM YUNUSABAD", day=20)

    corpus = T._load_annotated_corpus(db_session)
    context = T.build_context([target], corpus)[int(target.id)]

    assert context["exact_history"] is False
    assert context["neighbours"], "a never-seen merchant must still get examples"


def test_corpus_excludes_automation_written_rows(db_session, references):
    ruled = _txn(db_session, "SHOP RULE", day=1)
    A.apply_annotation_patch(db_session, ruled, {"article": "р1"}, source="rule", validate=False)
    agented = _txn(db_session, "SHOP AGENT", day=2)
    A.apply_annotation_patch(db_session, agented, {"article": "р3"}, source="agent", validate=False)
    db_session.commit()

    assert T._load_annotated_corpus(db_session) == []


# ----------------------------------------------------------------- validation

def _validate(item, *, allowed_ids={1}, codes=None, fields=("details", "article"), min_conf=0.7):
    return T._validate_suggestion(
        item,
        allowed_ids=allowed_ids,
        reference_codes=codes if codes is not None else {"article": {"р1"}, "person": {"СТ"}},
        fields=fields,
        min_confidence=min_conf,
    )


def test_invented_article_code_is_dropped():
    assert _validate({"id": 1, "article": "zz9", "confidence": 0.99}) is None


def test_low_confidence_suggestion_is_dropped():
    assert _validate({"id": 1, "article": "р1", "confidence": 0.4}) is None


def test_suggestion_for_an_untargeted_row_is_dropped():
    assert _validate({"id": 999, "article": "р1", "confidence": 0.9}) is None


def test_latin_code_in_a_suggestion_is_folded():
    result = _validate({"id": 1, "article": "p1", "confidence": 0.9})
    assert result["patch"] == {"annotation_article_code": "р1"}


def test_valid_suggestion_survives():
    result = _validate({"id": 1, "article": "р1", "details": "вп маг", "confidence": 0.86})
    assert result["patch"] == {"annotation_details": "вп маг", "annotation_article_code": "р1"}
    assert result["confidence"] == 0.86


def test_field_not_requested_is_ignored():
    result = _validate({"id": 1, "article": "р1", "person": "СТ", "confidence": 0.9},
                       fields=("article",))
    assert "annotation_person_code" not in result["patch"]


# ---------------------------------------------------------------- the tool

@pytest.mark.asyncio
async def test_tool_returns_a_preview_and_writes_nothing(db_session, references, monkeypatch):
    for i in range(3):
        _txn(db_session, "ANGLESEY FOOD", annotated={"article": "р1", "details": "вп маг"}, day=i + 1)
    target = _txn(db_session, "ANGLESEY FOOD", day=20)

    class _FakeProvider:
        enabled = True

        async def complete_json(self, **_kwargs):
            return {"items": [{
                "id": int(target.id), "article": "р1", "details": "вп маг",
                "confidence": 0.9, "reason": "тот же оператор",
            }]}

    monkeypatch.setattr(T, "get_text_ai_provider", lambda: _FakeProvider())
    monkeypatch.setattr(T, "current_agent_authorization", lambda db, uid: _AllowAll())
    monkeypatch.setattr(T, "actor_id", lambda user: 1)

    result = await T.suggest_transaction_annotations(
        db_session, {"user_id": 1, "role": "admin"}, None,
        {"transaction_ids": [int(target.id)], "fields": ["details", "article"]},
    )

    assert result["requires_confirmation"] is True
    assert result["confirmation_payload"]["action"] == "apply_transaction_annotations"
    assert len(result["confirmation_payload"]["args"]["items"]) == 1

    # Nothing may be written before the user confirms.
    db_session.refresh(target)
    assert target.annotation_article_code is None
    assert target.annotation_details is None


@pytest.mark.asyncio
async def test_tool_reports_when_the_provider_is_unconfigured(db_session, references, monkeypatch):
    class _Disabled:
        enabled = False

    monkeypatch.setattr(T, "get_text_ai_provider", lambda: _Disabled())
    result = await T.suggest_transaction_annotations(db_session, {"user_id": 1}, None, {})
    assert "requires_confirmation" not in result
    assert result["cards"][0]["type"] == "warning"


@pytest.mark.asyncio
async def test_tool_refuses_without_a_corpus(db_session, references, monkeypatch):
    """With nothing annotated there is nothing to retrieve, so guessing is all
    that is left -- better to say so than to invent bookkeeping."""
    target = _txn(db_session, "BRAND NEW SHOP", day=1)

    class _FakeProvider:
        enabled = True

        async def complete_json(self, **_kwargs):  # pragma: no cover - must not run
            raise AssertionError("the model must not be called without a corpus")

    monkeypatch.setattr(T, "get_text_ai_provider", lambda: _FakeProvider())
    monkeypatch.setattr(T, "current_agent_authorization", lambda db, uid: _AllowAll())
    monkeypatch.setattr(T, "actor_id", lambda user: 1)

    result = await T.suggest_transaction_annotations(
        db_session, {"user_id": 1, "role": "admin"}, None,
        {"transaction_ids": [int(target.id)]},
    )
    assert result["cards"][0]["type"] == "warning"


class _AllowAll:
    user_id = 1

    def require_dashboard(self):
        return None

    def require_admin(self, _action):
        return None

    def authorize_transaction(self, db, tx, proposed_date=None):
        return None


# --------------------------------------------------------------- confirmation

def test_confirm_applies_the_batch_with_agent_provenance(db_session, references):
    from services.ai_agent import confirm_service

    a = _txn(db_session, "SHOP A", day=1)
    b = _txn(db_session, "SHOP B", day=2)

    result = confirm_service._confirm_apply_transaction_annotations(
        db_session,
        args={"items": [
            {"transaction_id": int(a.id), "patch": {"annotation_article_code": "р1"}},
            {"transaction_id": int(b.id), "patch": {"annotation_details": "вп маг"}},
        ]},
        auth=_AllowAll(),
    )

    db_session.refresh(a)
    db_session.refresh(b)
    assert a.annotation_article_code == "р1"
    assert a.annotation_article_src == "agent"
    assert b.annotation_details == "вп маг"
    assert result["confirmation_result"]["changed_fields"] == 2


def test_confirm_does_not_overwrite_manual_values(db_session, references):
    from services.ai_agent import confirm_service

    txn = _txn(db_session, "SHOP M", annotated={"article": "р3"}, day=1)

    result = confirm_service._confirm_apply_transaction_annotations(
        db_session,
        args={"items": [{"transaction_id": int(txn.id), "patch": {"annotation_article_code": "р1"}}]},
        auth=_AllowAll(),
    )

    db_session.refresh(txn)
    assert txn.annotation_article_code == "р3"
    assert txn.annotation_article_src == "manual"
    assert result["confirmation_result"]["skipped"]


def test_confirm_rejects_an_empty_batch(db_session, references):
    from services.ai_agent import confirm_service

    with pytest.raises(ValueError, match="missing_annotation_items"):
        confirm_service._confirm_apply_transaction_annotations(
            db_session, args={"items": []}, auth=_AllowAll()
        )


def test_confirm_rejects_a_missing_transaction(db_session, references):
    from services.ai_agent import confirm_service

    with pytest.raises(ValueError, match="annotation_missing_transactions"):
        confirm_service._confirm_apply_transaction_annotations(
            db_session,
            args={"items": [{"transaction_id": 999999, "patch": {"annotation_article_code": "р1"}}]},
            auth=_AllowAll(),
        )


def test_confirm_action_is_registered_in_the_dispatcher():
    """Without the dispatcher branch every confirmation answers 422."""
    import inspect
    from services.ai_agent import confirm_service

    source = inspect.getsource(confirm_service.execute_confirmation_for_run)
    assert "apply_transaction_annotations" in source


def test_rollback_restores_the_previous_value(db_session, references):
    import asyncio

    txn = _txn(db_session, "SHOP R", day=1)
    A.apply_annotation_patch(db_session, txn, {"article": "р1"}, source="rule", validate=False)
    A.apply_annotation_patch(db_session, txn, {"article": "р3"}, source="agent", validate=False)
    db_session.commit()
    assert txn.annotation_article_code == "р3"

    import services.ai_agent.tools.annotation_tools as tools
    original_auth = tools.current_agent_authorization
    original_actor = tools.actor_id
    tools.current_agent_authorization = lambda db, uid: _AllowAll()
    tools.actor_id = lambda user: 1
    try:
        asyncio.run(tools.rollback_transaction_annotations(
            db_session, {"user_id": 1, "role": "admin"}, None, {}
        ))
    finally:
        tools.current_agent_authorization = original_auth
        tools.actor_id = original_actor

    db_session.refresh(txn)
    # Restored to what the rule had written, not blanked.
    assert txn.annotation_article_code == "р1"
    assert txn.annotation_article_src == "rule"
