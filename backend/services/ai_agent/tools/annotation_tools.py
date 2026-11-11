"""Agent tools for the bookkeeping annotations.

The deterministic rules cover the merchants that repeat. What is left is the
long tail: 74 of the client's 181 merchants appear exactly once, so there is no
history to copy from and the value has to be inferred from the merchant name,
the amount and the direction of the operation. That is the job of this tool.

It never writes. A suggestion goes back as a `confirmation_payload` and is only
applied by `confirm_service` after a person approves it -- unlike the older
`description_tools`, which write straight through. Annotations feed accounting
reports, so a wrong guess that lands silently is worse than no guess.
"""
from __future__ import annotations

import hashlib
import json
import logging
from datetime import date, datetime
from decimal import Decimal
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

from sqlalchemy import or_
from sqlalchemy.orm import Session

from database.models import AnnotationArticle, AnnotationPerson, Transaction
from parsers.operator_mapper import OperatorMapper
from services import annotation_service
from services.ai_agent.authorization import actor_id, current_agent_authorization
from services.ai_provider import get_text_ai_provider

logger = logging.getLogger(__name__)

# One LLM call handles a small batch: a bigger prompt buys little accuracy and
# makes a single failure expensive.
_BATCH_SIZE = 12
_MAX_TARGETS = 100
_NEIGHBOURS_SAME_OPERATOR = 8
_NEIGHBOURS_FALLBACK = 10
_MAX_CONFIRM_ITEMS = 200

_SUGGESTABLE_FIELDS = ("details", "person", "article")


def _json_default(value: Any) -> Any:
    if isinstance(value, Decimal):
        return str(value)
    if isinstance(value, (datetime, date)):
        return value.isoformat()
    return str(value)


def _idempotency_key(action: str, payload: Dict[str, Any]) -> str:
    raw = json.dumps({"action": action, "payload": payload}, sort_keys=True, default=_json_default)
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()[:32]


def _warning(title: str, body: str) -> Dict[str, Any]:
    return {
        "summary": body,
        "assistant_message": body,
        "cards": [{"type": "warning", "title": title, "body": body}],
    }


# --------------------------------------------------------------------------
# Retrieval
# --------------------------------------------------------------------------

def _operator_key(value: Optional[str]) -> str:
    return OperatorMapper.normalize_operator(value or "")


def _tokens(key: str) -> set:
    return {token for token in key.split() if len(token) > 2}


def _describe_target(txn: Transaction) -> Dict[str, Any]:
    return {
        "id": int(txn.id),
        "operator": txn.operator_raw,
        "application": txn.application_mapped,
        "amount": str(abs(txn.amount)) if txn.amount is not None else None,
        "direction": "expense" if (txn.amount or 0) < 0 else "income",
        "type": txn.transaction_type,
        "currency": txn.currency,
        "card": txn.card_last_4,
        "date": txn.transaction_date.isoformat() if txn.transaction_date else None,
        # The raw receipt often carries the hint ("конв.", "парковка") that the
        # merchant name alone does not.
        "raw": (txn.raw_message or "")[:300],
    }


def _describe_neighbour(txn: Transaction) -> Dict[str, Any]:
    return {
        "operator": txn.operator_raw,
        "amount": str(abs(txn.amount)) if txn.amount is not None else None,
        "direction": "expense" if (txn.amount or 0) < 0 else "income",
        "details": txn.annotation_details,
        "person": txn.annotation_person_code,
        "article": txn.annotation_article_code,
    }


def _load_annotated_corpus(db: Session, limit: int = 4000) -> List[Transaction]:
    """Rows a human wrote or the workbook supplied -- the only trustworthy examples."""
    return (
        db.query(Transaction)
        .filter(
            or_(
                Transaction.annotation_details_src.in_(("manual", "import")),
                Transaction.annotation_article_src.in_(("manual", "import")),
            )
        )
        .order_by(Transaction.transaction_date.desc())
        .limit(limit)
        .all()
    )


def build_context(
    targets: Sequence[Transaction],
    corpus: Sequence[Transaction],
) -> Dict[int, Dict[str, Any]]:
    """Assemble per-target retrieval: exact merchant history, then similar names.

    Exact-key history is by far the strongest signal. When a merchant has none
    -- the single-occurrence long tail -- fall back to merchants whose
    normalized names share tokens, which at least anchors the category.
    """
    by_key: Dict[str, List[Transaction]] = {}
    for txn in corpus:
        by_key.setdefault(_operator_key(txn.operator_raw), []).append(txn)

    token_index = {key: _tokens(key) for key in by_key}

    context: Dict[int, Dict[str, Any]] = {}
    for target in targets:
        key = _operator_key(target.operator_raw)
        exact = by_key.get(key, [])[:_NEIGHBOURS_SAME_OPERATOR]

        neighbours = list(exact)
        if not exact and key:
            target_tokens = _tokens(key)
            scored: List[Tuple[float, str]] = []
            for other_key, other_tokens in token_index.items():
                if not other_tokens or not target_tokens:
                    continue
                overlap = len(target_tokens & other_tokens)
                if not overlap:
                    continue
                union = len(target_tokens | other_tokens)
                score = overlap / union
                if other_key.startswith(key[:8]) or key.startswith(other_key[:8]):
                    score += 0.25
                scored.append((score, other_key))
            scored.sort(reverse=True)
            for _score, other_key in scored[:5]:
                neighbours.extend(by_key[other_key][:2])
            neighbours = neighbours[:_NEIGHBOURS_FALLBACK]

        # Absolute counts, not just examples: the model can see how lopsided the
        # merchant's history is.
        stats: Dict[str, Dict[str, int]] = {"article": {}, "person": {}, "details": {}}
        for txn in by_key.get(key, []):
            for field, value in (
                ("article", txn.annotation_article_code),
                ("person", txn.annotation_person_code),
                ("details", txn.annotation_details),
            ):
                if value:
                    stats[field][value] = stats[field].get(value, 0) + 1

        context[int(target.id)] = {
            "neighbours": [_describe_neighbour(t) for t in neighbours],
            "stats": {k: v for k, v in stats.items() if v},
            "exact_history": bool(exact),
        }
    return context


# --------------------------------------------------------------------------
# Prompting
# --------------------------------------------------------------------------

_SYSTEM_PROMPT = """Ты помогаешь бухгалтеру размечать банковские транзакции.

Для каждой транзакции предложи:
- article: код статьи СТРОГО из справочника ниже
- person: код подотчётного лица СТРОГО из справочника ниже
- details: короткая пометка на русском в стиле уже существующих пометок этого
  бухгалтера (примеры даны в neighbours)

Правила:
1. Никогда не выдумывай коды. Если подходящего кода нет — верни null.
2. Если уверенности мало — верни null для этого поля и низкий confidence.
3. Расходные статьи (префикс "р") только для списаний, приходные ("п") только
   для поступлений. Направление указано в поле direction.
4. details копируй по смыслу с neighbours того же оператора, если они есть.

Ответ строго в формате JSON:
{"items": [{"id": 123, "article": "р1", "person": "СТ", "details": "вп маг",
            "confidence": 0.86, "reason": "тот же оператор, 12 раз р1"}]}"""


def _build_user_prompt(
    batch: Sequence[Transaction],
    context: Dict[int, Dict[str, Any]],
    articles: Sequence[Dict[str, str]],
    persons: Sequence[str],
    fields: Sequence[str],
) -> str:
    payload = {
        "articles": articles,
        "persons": list(persons),
        "fields_requested": list(fields),
        "targets": [
            {**_describe_target(txn), **context.get(int(txn.id), {})}
            for txn in batch
        ],
    }
    return json.dumps(payload, ensure_ascii=False, default=_json_default)


def _coerce_items(raw: Any) -> List[Dict[str, Any]]:
    if isinstance(raw, dict):
        items = raw.get("items")
        if isinstance(items, list):
            return [item for item in items if isinstance(item, dict)]
    if isinstance(raw, list):
        return [item for item in raw if isinstance(item, dict)]
    return []


def _validate_suggestion(
    item: Dict[str, Any],
    *,
    allowed_ids: set,
    reference_codes: Dict[str, set],
    fields: Sequence[str],
    min_confidence: float,
) -> Optional[Dict[str, Any]]:
    """Drop anything the model invented before it can reach a confirmation card."""
    try:
        transaction_id = int(item.get("id"))
    except (TypeError, ValueError):
        return None
    if transaction_id not in allowed_ids:
        return None

    try:
        confidence = float(item.get("confidence", 0))
    except (TypeError, ValueError):
        confidence = 0.0
    if confidence < min_confidence:
        return None

    patch: Dict[str, Any] = {}
    for field in fields:
        value = item.get(field)
        if value in (None, "", "null"):
            continue
        if field == "details":
            text = str(value).strip()[:annotation_service.DETAILS_MAX_LENGTH]
            if text:
                patch["annotation_details"] = text
        else:
            code = annotation_service.normalize_code(value)
            if code and code in reference_codes.get(field, set()):
                column = annotation_service.ANNOTATION_FIELDS[field][0]
                patch[column] = code

    if not patch:
        return None
    return {
        "transaction_id": transaction_id,
        "patch": patch,
        "confidence": round(confidence, 3),
        "reason": str(item.get("reason") or "")[:300],
    }


# --------------------------------------------------------------------------
# Tool
# --------------------------------------------------------------------------

async def suggest_transaction_annotations(
    db: Session,
    current_user: Dict[str, Any],
    scope: Optional[dict],
    args: Dict[str, Any],
) -> Dict[str, Any]:
    """Propose bookkeeping annotations for transactions that lack them.

    Returns a preview only; nothing is written until the user confirms.
    """
    provider = get_text_ai_provider()
    if not getattr(provider, "enabled", False):
        return _warning("Разметка", "AI-провайдер не настроен: нет ключа API.")

    auth = current_agent_authorization(db, actor_id(current_user))
    auth.require_dashboard()

    fields = [f for f in (args.get("fields") or ["details", "article"]) if f in _SUGGESTABLE_FIELDS]
    if not fields:
        fields = ["details", "article"]
    limit = max(1, min(int(args.get("limit") or 30), _MAX_TARGETS))
    min_confidence = float(args.get("min_confidence") or 0.7)

    # ---- pick the targets ------------------------------------------------
    query = db.query(Transaction)
    raw_ids = args.get("transaction_ids") or []
    if raw_ids:
        ids = []
        for value in raw_ids[:_MAX_TARGETS]:
            try:
                ids.append(int(value))
            except (TypeError, ValueError):
                continue
        query = query.filter(Transaction.id.in_(ids))
    else:
        if args.get("date_from"):
            query = query.filter(Transaction.transaction_date >= args["date_from"])
        if args.get("date_to"):
            query = query.filter(Transaction.transaction_date <= args["date_to"])
        if args.get("only_missing", True):
            missing = [
                getattr(Transaction, annotation_service.ANNOTATION_FIELDS[f][0]).is_(None)
                for f in fields
            ]
            query = query.filter(or_(*missing))
        query = query.order_by(Transaction.transaction_date.desc())

    targets = query.limit(limit).all()
    if not targets:
        return _warning("Разметка", "Не найдено транзакций, требующих разметки.")

    # Authorize every target, not a sample: each one is a separate write.
    for txn in targets:
        auth.authorize_transaction(db, txn)

    # ---- retrieval + references -----------------------------------------
    corpus = _load_annotated_corpus(db)
    if not corpus:
        return _warning(
            "Разметка",
            "Нет размеченных транзакций для сравнения. Сначала импортируйте книгу клиента.",
        )
    context = build_context(targets, corpus)

    articles = [
        {"code": row.code, "label": row.label, "direction": row.direction}
        for row in db.query(AnnotationArticle)
        .filter(AnnotationArticle.is_active.is_(True))
        .order_by(AnnotationArticle.sort_order)
        .all()
    ]
    persons = [
        row.code for row in db.query(AnnotationPerson)
        .filter(AnnotationPerson.is_active.is_(True))
        .order_by(AnnotationPerson.sort_order)
        .all()
    ]
    reference_codes = annotation_service.load_reference_codes(db)
    allowed_ids = {int(t.id) for t in targets}

    progress = args.get("_progress_callback")

    # ---- ask the model in batches ---------------------------------------
    suggestions: List[Dict[str, Any]] = []
    batches = [targets[i:i + _BATCH_SIZE] for i in range(0, len(targets), _BATCH_SIZE)]
    for index, batch in enumerate(batches, start=1):
        if callable(progress):
            try:
                progress(f"Разметка: пачка {index}/{len(batches)}")
            except Exception:  # noqa: BLE001 - progress must never break the run
                pass
        try:
            raw = await provider.complete_json(
                messages=[
                    {"role": "system", "content": _SYSTEM_PROMPT},
                    {"role": "user", "content": _build_user_prompt(
                        batch, context, articles, persons, fields
                    )},
                ],
                temperature=0.1,
                max_tokens=1500,
            )
        except Exception as exc:  # noqa: BLE001 - a failed batch must not kill the rest
            logger.warning("annotation suggestion batch %s failed: %s", index, exc)
            continue

        for item in _coerce_items(raw):
            validated = _validate_suggestion(
                item,
                allowed_ids=allowed_ids,
                reference_codes=reference_codes,
                fields=fields,
                min_confidence=min_confidence,
            )
            if validated:
                suggestions.append(validated)

    if not suggestions:
        return _warning(
            "Разметка",
            f"Модель не предложила значений с уверенностью ≥ {min_confidence:g}.",
        )

    # ---- build the preview ----------------------------------------------
    by_id = {int(t.id): t for t in targets}
    lines: List[str] = []
    for item in suggestions[:20]:
        txn = by_id.get(item["transaction_id"])
        operator = (txn.operator_raw or "—")[:28] if txn else "—"
        parts = [
            f"{annotation_service.ANNOTATION_FIELDS[f][0].replace('annotation_', '')}="
            f"{item['patch'].get(annotation_service.ANNOTATION_FIELDS[f][0])}"
            for f in fields
            if item["patch"].get(annotation_service.ANNOTATION_FIELDS[f][0])
        ]
        lines.append(f"• #{item['transaction_id']} {operator} → {', '.join(parts)} ({item['confidence']})")
    if len(suggestions) > 20:
        lines.append(f"… и ещё {len(suggestions) - 20}")

    payload_args = {
        "items": suggestions[:_MAX_CONFIRM_ITEMS],
        "overwrite_manual": False,
    }
    return {
        "summary": f"Подготовлена разметка для {len(suggestions)} транзакций.",
        "assistant_message": "Проверьте предложенную разметку и нажмите Подтвердить.",
        "requires_confirmation": True,
        "confirmation_payload": {
            "action": "apply_transaction_annotations",
            "args": payload_args,
            "idempotency_key": _idempotency_key(
                "apply_transaction_annotations",
                {"ids": sorted(i["transaction_id"] for i in suggestions)},
            ),
        },
        "cards": [
            {
                "type": "preview_annotations",
                "title": f"Разметка: {len(suggestions)} транзакций",
                "body": "\n".join(lines),
                "payload": {
                    "action": "apply_transaction_annotations",
                    "items": suggestions[:_MAX_CONFIRM_ITEMS],
                },
            }
        ],
    }


async def rollback_transaction_annotations(
    db: Session,
    current_user: Dict[str, Any],
    scope: Optional[dict],
    args: Dict[str, Any],
) -> Dict[str, Any]:
    """Undo annotations the agent wrote, restoring what stood before.

    Restores `before_value` from the journal rather than blanking the field: the
    agent may have overwritten a value a rule had produced, and blanking would
    lose it.
    """
    from database.models import TransactionAnnotationEvent

    auth = current_agent_authorization(db, actor_id(current_user))
    auth.require_admin("rollback_transaction_annotations")

    limit = max(1, min(int(args.get("limit") or 200), 1000))
    events = (
        db.query(TransactionAnnotationEvent)
        .filter(TransactionAnnotationEvent.source == "agent")
        .order_by(TransactionAnnotationEvent.id.desc())
        .limit(limit)
        .all()
    )
    if not events:
        return _warning("Откат разметки", "Нет изменений, сделанных агентом.")

    restored = 0
    seen: set = set()
    for event in events:
        identity = (int(event.transaction_id), event.field)
        if identity in seen:
            continue  # only the newest event per field matters
        seen.add(identity)
        txn = db.get(Transaction, int(event.transaction_id))
        if txn is None:
            continue
        value_col, src_col = annotation_service.ANNOTATION_FIELDS[event.field]
        if getattr(txn, src_col, None) != "agent":
            continue  # someone edited it after the agent; leave it alone
        value = event.before_value
        if event.field == "period" and value:
            value = annotation_service.coerce_period(value)
        setattr(txn, value_col, value or None)
        setattr(txn, src_col, event.before_src)
        restored += 1

    db.commit()
    body = f"Восстановлено полей: {restored}."
    return {
        "summary": body,
        "assistant_message": body,
        "cards": [{"type": "info", "title": "Откат разметки", "body": body}],
    }
