"""Write path for per-transaction bookkeeping annotations.

Four fields carry the client's manual bookkeeping: a free-text note, an
accountable person, a bookkeeping month and a chart-of-accounts article. Unlike
the merchant `descriptions` registry (which is keyed by operator and applies
retroactively to every receipt of that merchant), these live on the transaction
itself: the same merchant legitimately carries different notes.

Every writer -- the API, the importer, the rule pass and the AI agent -- goes
through `apply_annotation_patch` so the provenance discipline exists in exactly
one place:

    manual  a human typed it        -- never overwritten except on explicit request
    import  came from the workbook  -- overwritable by a later import
    rule    a learned operator rule -- overwritable by anything
    agent   proposed by the LLM     -- overwritable by anything

Rules and imports only ever fill a field that is empty or was written by
`rule`/`agent`. That single invariant is what keeps automation from silently
eating hand-entered accounting.
"""
from __future__ import annotations

import logging
import unicodedata
from datetime import date, datetime
from typing import Any, Dict, Iterable, List, Optional, Tuple

from sqlalchemy.orm import Session

from database.models import (
    AnnotationArticle,
    AnnotationPerson,
    Transaction,
    TransactionAnnotationEvent,
)

logger = logging.getLogger(__name__)

# field name -> (value column, provenance column)
ANNOTATION_FIELDS: Dict[str, Tuple[str, str]] = {
    "details": ("annotation_details", "annotation_details_src"),
    "person": ("annotation_person_code", "annotation_person_src"),
    "period": ("annotation_period", "annotation_period_src"),
    "article": ("annotation_article_code", "annotation_article_src"),
}

VALUE_COLUMNS = {value_col for value_col, _ in ANNOTATION_FIELDS.values()}
SRC_COLUMNS = {src_col for _, src_col in ANNOTATION_FIELDS.values()}

VALID_SOURCES = ("manual", "import", "rule", "agent")

# Sources that automation is allowed to overwrite. A field written by a human
# ("manual") is only replaced when the caller explicitly asks for it.
_OVERWRITABLE_BY_AUTOMATION = (None, "", "rule", "agent")

DETAILS_MAX_LENGTH = 500

# The workbook is typed by hand on a Russian keyboard, so codes arrive in both
# alphabets -- 'р1' with a Cyrillic er and 'p1' with a Latin pee look identical
# on screen and are different strings. Every code is folded to Cyrillic before
# lookup or storage so the two spellings resolve to one reference entry.
_LATIN_TO_CYRILLIC = str.maketrans({
    "A": "А", "B": "В", "E": "Е", "K": "К", "M": "М", "H": "Н", "O": "О",
    "P": "Р", "C": "С", "T": "Т", "Y": "У", "X": "Х",
    "a": "а", "e": "е", "o": "о", "p": "р", "c": "с", "y": "у", "x": "х",
})


def normalize_code(value: Optional[str]) -> str:
    """Fold a hand-typed reference code to its canonical Cyrillic spelling."""
    if value is None:
        return ""
    text = unicodedata.normalize("NFKC", str(value)).strip()
    if not text:
        return ""
    return text.translate(_LATIN_TO_CYRILLIC)


def coerce_period(value: Any) -> Optional[date]:
    """Coerce an incoming bookkeeping period to the first day of its month.

    The period is a month, not a day: storing anything else would make grouping
    and the `check_annotation_period_first_day` intent inconsistent.
    """
    if value in (None, ""):
        return None
    if isinstance(value, datetime):
        return date(value.year, value.month, 1)
    if isinstance(value, date):
        return date(value.year, value.month, 1)
    text = str(value).strip()
    if not text:
        return None
    for fmt in ("%Y-%m-%d", "%Y-%m", "%d.%m.%Y", "%m.%Y"):
        try:
            parsed = datetime.strptime(text, fmt)
        except ValueError:
            continue
        return date(parsed.year, parsed.month, 1)
    raise ValueError(f"unrecognized annotation period: {value!r}")


class AnnotationValidationError(ValueError):
    """Raised when a patch references a code the reference tables do not know."""


def load_reference_codes(db: Session) -> Dict[str, set]:
    """Active article and person codes, normalized for comparison."""
    articles = {
        normalize_code(row[0])
        for row in db.query(AnnotationArticle.code)
        .filter(AnnotationArticle.is_active.is_(True))
        .all()
    }
    persons = {
        normalize_code(row[0])
        for row in db.query(AnnotationPerson.code)
        .filter(AnnotationPerson.is_active.is_(True))
        .all()
    }
    return {"article": articles, "person": persons}


def register_auto_codes(
    db: Session,
    *,
    articles: Optional[set] = None,
    persons: Optional[set] = None,
) -> List[Dict[str, Any]]:
    """
    Add codes the customer's workbook uses but the dictionaries do not know yet.

    They are recorded with `is_auto=True` and no label, which is what that column
    was always for: a code that arrived from an import and still needs a human to
    say what it means. Without this an unfamiliar code was written onto the
    transaction and then vanished — it matched no dictionary entry, so the grid
    showed a bare code and no report ever mentioned it.

    Returns one entry per registered code so the caller can surface them.
    """
    registered: List[Dict[str, Any]] = []

    for code in sorted(articles or set()):
        if not code:
            continue
        direction = "income" if code.startswith("п") else "expense"
        db.add(AnnotationArticle(code=code, label="", direction=direction, is_auto=True))
        registered.append({"kind": "article", "code": code})

    for code in sorted(persons or set()):
        if not code:
            continue
        db.add(AnnotationPerson(code=code, full_name=None, is_auto=True))
        registered.append({"kind": "person", "code": code})

    if registered:
        db.flush()
    return registered


def normalize_patch(
    patch: Dict[str, Any],
    *,
    reference_codes: Optional[Dict[str, set]] = None,
) -> Dict[str, Any]:
    """Validate and canonicalize a `{field: value}` annotation patch.

    Accepts either short field names (`details`) or full column names
    (`annotation_details`) so callers do not have to translate.
    """
    normalized: Dict[str, Any] = {}
    for key, raw in patch.items():
        field = key
        if key.startswith("annotation_"):
            field = next(
                (name for name, (value_col, _) in ANNOTATION_FIELDS.items() if value_col == key),
                None,
            )
        if field not in ANNOTATION_FIELDS:
            continue

        if raw in (None, ""):
            normalized[field] = None
            continue

        if field == "details":
            text = str(raw).strip()
            normalized[field] = text[:DETAILS_MAX_LENGTH] or None
        elif field == "period":
            normalized[field] = coerce_period(raw)
        else:  # person, article
            code = normalize_code(raw)
            if not code:
                normalized[field] = None
                continue
            if reference_codes is not None and code not in reference_codes.get(field, set()):
                raise AnnotationValidationError(f"unknown_{field}_code:{code}")
            normalized[field] = code
    return normalized


def apply_annotation_patch(
    db: Session,
    transaction: Transaction,
    patch: Dict[str, Any],
    *,
    source: str,
    actor_user_id: Optional[int] = None,
    agent_run_id: Optional[Any] = None,
    import_batch_id: Optional[Any] = None,
    overwrite_manual: bool = False,
    reference_codes: Optional[Dict[str, set]] = None,
    validate: bool = True,
) -> Dict[str, Any]:
    """Apply an annotation patch to one transaction, honouring provenance.

    Returns a report of what changed, what was skipped and why -- callers use it
    to build the import/agent conflict lists rather than guessing.

    Does not commit: the caller owns the transaction boundary.
    """
    if source not in VALID_SOURCES:
        raise ValueError(f"invalid annotation source: {source!r}")

    normalized = normalize_patch(
        patch,
        reference_codes=reference_codes if validate else None,
    )

    changed: Dict[str, Any] = {}
    skipped: Dict[str, str] = {}
    automation = source in ("rule", "import", "agent")

    for field, new_value in normalized.items():
        value_col, src_col = ANNOTATION_FIELDS[field]
        current = getattr(transaction, value_col, None)
        current_src = getattr(transaction, src_col, None)

        if automation and not overwrite_manual:
            if current_src not in _OVERWRITABLE_BY_AUTOMATION:
                # A human owns this field. Report it instead of clobbering it.
                if current != new_value:
                    skipped[field] = f"protected_by:{current_src}"
                continue
            # A rule must never blank a value that is already there.
            if source == "rule" and new_value is None:
                continue

        if current == new_value:
            continue

        db.add(
            TransactionAnnotationEvent(
                transaction_id=transaction.id,
                field=field,
                before_value=_as_text(current),
                after_value=_as_text(new_value),
                before_src=current_src,
                source=source,
                actor_user_id=actor_user_id,
                agent_run_id=agent_run_id,
                import_batch_id=import_batch_id,
            )
        )
        setattr(transaction, value_col, new_value)
        setattr(transaction, src_col, source if new_value is not None else None)
        changed[field] = new_value

    if changed:
        transaction.annotation_updated_at = datetime.utcnow()
        transaction.annotation_updated_by = actor_user_id

    return {"changed": changed, "skipped": skipped}


def _as_text(value: Any) -> Optional[str]:
    if value is None:
        return None
    if isinstance(value, date):
        return value.isoformat()
    return str(value)[:DETAILS_MAX_LENGTH]


def annotation_payload(transaction: Transaction) -> Dict[str, Any]:
    """Serialize the annotation block of one transaction for API responses."""
    period = getattr(transaction, "annotation_period", None)
    return {
        "annotation_details": getattr(transaction, "annotation_details", None),
        "annotation_person_code": getattr(transaction, "annotation_person_code", None),
        "annotation_period": period.isoformat() if isinstance(period, date) else None,
        "annotation_article_code": getattr(transaction, "annotation_article_code", None),
        "annotation_details_src": getattr(transaction, "annotation_details_src", None),
        "annotation_person_src": getattr(transaction, "annotation_person_src", None),
        "annotation_period_src": getattr(transaction, "annotation_period_src", None),
        "annotation_article_src": getattr(transaction, "annotation_article_src", None),
    }


def rows_for_rule_training(
    db: Session,
    *,
    field: str,
    limit: Optional[int] = None,
) -> Iterable[Tuple[Optional[str], Any]]:
    """Yield `(operator_raw, value)` pairs eligible to train an operator rule.

    Only rows a human wrote or the workbook supplied are eligible. Counting
    rule- or agent-written rows would let a rule confirm its own output: purity
    would climb to 1.0 by construction and the first mistake would be frozen in
    permanently.
    """
    value_col, src_col = ANNOTATION_FIELDS[field]
    query = (
        db.query(Transaction.operator_raw, getattr(Transaction, value_col))
        .filter(getattr(Transaction, value_col).isnot(None))
        .filter(getattr(Transaction, src_col).in_(("manual", "import")))
        .filter(Transaction.operator_raw.isnot(None))
    )
    if limit:
        query = query.limit(limit)
    return query.yield_per(1000)


def rows_for_details_training(db: Session) -> Iterable[Tuple[Any, ...]]:
    """Yield `(operator_raw, card_last_4, transaction_type, details)`, oldest first.

    The same eligibility as `rows_for_rule_training`: only what a person wrote
    or the workbook supplied. The order matters here, because a note is trusted
    only while the most recent confirmed receipts still agree on it.
    """
    return (
        db.query(
            Transaction.operator_raw,
            Transaction.card_last_4,
            Transaction.transaction_type,
            Transaction.annotation_details,
        )
        .filter(Transaction.annotation_details.isnot(None))
        .filter(Transaction.annotation_details_src.in_(("manual", "import")))
        .filter(Transaction.operator_raw.isnot(None))
        .order_by(Transaction.transaction_date.asc(), Transaction.id.asc())
        .yield_per(1000)
    )
