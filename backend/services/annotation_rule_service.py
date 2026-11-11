"""Learned operator -> annotation rules.

Half of the client's bookkeeping is mechanical: `OOO STREETPARK SYSTE` is always
"ст парковка", and roughly half of all articles follow from the merchant alone.
This module learns those regularities from rows a human wrote or the workbook
supplied, and fills empty fields with them so the accountant only handles what
actually needs judgement.

Two rules keep it honest:

* **Training excludes automation.** Only `manual` and `import` rows count. If a
  rule's own output fed back into its purity, purity would reach 1.0 by
  construction and the first mistake would be frozen in permanently.
* **Applying only ever fills a gap.** A rule writes to a field that is empty or
  was itself written by automation; a human's entry is never touched.

Rules are applied by batch passes, not by a hook on transaction creation: the
admin's explicit `apply_rules`, and `fill_defaults`, which the scheduler runs
every few minutes so new receipts get their defaults without anyone pressing a
button. Rows are created in at least four places (the receipt processor, the
Celery worker, the SMS ingest and the agent's create tool), and wiring an
optional convenience into each of them would put four new failure points in the
ingestion path.
"""
from __future__ import annotations

import logging
import os
import threading
import time
from collections import defaultdict
from datetime import date, datetime
from decimal import Decimal
from typing import Any, Dict, Iterable, List, Optional, Tuple

from sqlalchemy.orm import Session

from database.models import AnnotationRule, Transaction
from parsers.operator_mapper import OperatorMapper
from services import annotation_service

logger = logging.getLogger(__name__)

RULE_FIELDS = ("details", "person", "article")

DEFAULT_MIN_SUPPORT = int(os.getenv("ANNOTATION_RULE_MIN_SUPPORT", "3"))
DEFAULT_MIN_PURITY = float(os.getenv("ANNOTATION_RULE_MIN_PURITY", "0.9"))

_CACHE_TTL_SECONDS = int(os.getenv("ANNOTATION_RULE_CACHE_TTL", "120"))
_cache_lock = threading.Lock()
_cache: Dict[str, Any] = {"data": None, "expires_at": 0.0}


def _normalized_key(operator_raw: Optional[str]) -> str:
    """Merchant key, shared with the descriptions registry.

    Reusing `OperatorMapper.normalize_operator` keeps annotations and merchant
    descriptions agreeing on what counts as one merchant.
    """
    return OperatorMapper.normalize_operator(operator_raw or "")


def _value_text(value: Any) -> Optional[str]:
    if value is None:
        return None
    if isinstance(value, date):
        return value.isoformat()
    text = str(value).strip()
    return text or None


# --------------------------------------------------------------------------
# Recompute
# --------------------------------------------------------------------------

def compute_candidates(
    db: Session,
    *,
    fields: Iterable[str] = RULE_FIELDS,
    min_support: int = DEFAULT_MIN_SUPPORT,
    min_purity: float = DEFAULT_MIN_PURITY,
) -> Dict[str, List[Dict[str, Any]]]:
    """Derive rule candidates without writing anything.

    Grouping happens in Python because the normalized operator key is not stored
    on the row and its normalization (NFKC, case folding, punctuation collapse)
    has no SQL equivalent. The training set is small -- annotated rows only.
    """
    result: Dict[str, List[Dict[str, Any]]] = {}

    for field in fields:
        if field not in RULE_FIELDS:
            continue
        counts: Dict[str, Dict[str, int]] = defaultdict(lambda: defaultdict(int))
        for operator_raw, value in annotation_service.rows_for_rule_training(db, field=field):
            key = _normalized_key(operator_raw)
            text = _value_text(value)
            if not key or not text:
                continue
            counts[key][text] += 1

        candidates: List[Dict[str, Any]] = []
        for key, values in counts.items():
            total = sum(values.values())
            if total < min_support:
                continue
            value, support = max(values.items(), key=lambda item: item[1])
            purity = support / total
            if purity < min_purity:
                continue
            candidates.append({
                "operator_key": key,
                "field": field,
                "value": value,
                "support": support,
                "total": total,
                "purity": round(purity, 3),
            })
        result[field] = candidates

    return result


def recompute(
    db: Session,
    *,
    fields: Iterable[str] = RULE_FIELDS,
    min_support: int = DEFAULT_MIN_SUPPORT,
    min_purity: float = DEFAULT_MIN_PURITY,
    created_by_user_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Recompute and persist the rule set. Pinned rules are left alone."""
    candidates = compute_candidates(
        db, fields=fields, min_support=min_support, min_purity=min_purity
    )
    fields = [f for f in fields if f in RULE_FIELDS]

    existing = {
        (row.operator_key, row.field): row
        for row in db.query(AnnotationRule).filter(AnnotationRule.field.in_(fields)).all()
    }

    now = datetime.utcnow()
    created = updated = disabled = pinned_kept = 0
    seen: set = set()

    for field in fields:
        for candidate in candidates.get(field, []):
            identity = (candidate["operator_key"], field)
            seen.add(identity)
            row = existing.get(identity)
            if row is None:
                db.add(AnnotationRule(
                    operator_key=candidate["operator_key"],
                    field=field,
                    value=candidate["value"],
                    support=candidate["support"],
                    purity=Decimal(str(candidate["purity"])),
                    is_enabled=True,
                    computed_at=now,
                    created_by_user_id=created_by_user_id,
                ))
                created += 1
                continue
            if row.is_pinned:
                pinned_kept += 1
                continue
            row.value = candidate["value"]
            row.support = candidate["support"]
            row.purity = Decimal(str(candidate["purity"]))
            row.is_enabled = True
            row.computed_at = now
            updated += 1

    # A rule that no longer clears the bar is disabled, not deleted: the history
    # of what the data once supported is worth keeping.
    for identity, row in existing.items():
        if identity in seen or row.is_pinned or not row.is_enabled:
            continue
        row.is_enabled = False
        row.computed_at = now
        disabled += 1

    db.commit()
    invalidate_rule_cache()

    return {
        "created": created,
        "updated": updated,
        "disabled": disabled,
        "pinned_kept": pinned_kept,
        "total_enabled": db.query(AnnotationRule).filter(AnnotationRule.is_enabled.is_(True)).count(),
        "min_support": min_support,
        "min_purity": min_purity,
    }


# --------------------------------------------------------------------------
# Lookup + apply
# --------------------------------------------------------------------------

def _load_rules(db: Session) -> Dict[str, Dict[str, str]]:
    """`{operator_key: {field: value}}` for every enabled rule."""
    mapping: Dict[str, Dict[str, str]] = defaultdict(dict)
    for row in db.query(AnnotationRule).filter(AnnotationRule.is_enabled.is_(True)).all():
        mapping[row.operator_key][row.field] = row.value
    return mapping


def get_rules(db: Session) -> Dict[str, Dict[str, str]]:
    """Cached rule lookup, refreshed on a TTL like the descriptions registry."""
    now = time.monotonic()
    with _cache_lock:
        if _cache["data"] is not None and now < _cache["expires_at"]:
            return _cache["data"]
    data = _load_rules(db)
    with _cache_lock:
        _cache["data"] = data
        _cache["expires_at"] = time.monotonic() + _CACHE_TTL_SECONDS
    return data


def invalidate_rule_cache() -> None:
    with _cache_lock:
        _cache["data"] = None
        _cache["expires_at"] = 0.0


def defaults_for_new(
    db: Session,
    *,
    operator_raw: Optional[str],
    transaction_date: Optional[datetime],
) -> Dict[str, Any]:
    """Annotation defaults for a freshly created transaction.

    The period defaults to the month of the operation. That is a guess -- the
    client's own ledger books some operations into a different month -- so it is
    written with a 'rule' provenance that any later import may correct.

    A learned rule is specific to the merchant, so it wins over the global
    default person.
    """
    patch: Dict[str, Any] = dict(get_rules(db).get(_normalized_key(operator_raw), {}))
    if transaction_date is not None:
        patch.setdefault("period", date(transaction_date.year, transaction_date.month, 1))

    default_person = (os.getenv("ANNOTATION_DEFAULT_PERSON") or "").strip()
    if default_person:
        patch.setdefault("person", annotation_service.normalize_code(default_person))
    return patch


def _locked_ranges(db: Session) -> List[Tuple[date, date]]:
    from services.period_lock_service import PeriodLockService

    return [(lock.date_from, lock.date_to) for lock in PeriodLockService().get_active_locks(db)]


def fill_defaults(
    db: Session,
    *,
    batch_size: int = 500,
    limit: Optional[int] = None,
    actor_user_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Give receipts their annotation defaults: the background counterpart of `apply_rules`.

    Writes `defaults_for_new` (operation month, learned rules, default person)
    into fields that are still empty, newest receipts first, so new receipts and
    old ones that predate a rule both get filled. Values already present, by
    anyone, are left alone. Locked periods are skipped: a closed month must not
    change behind the accountant's back.
    """
    from sqlalchemy import or_

    locks = _locked_ranges(db)
    value_columns = [getattr(Transaction, col) for col, _ in annotation_service.ANNOTATION_FIELDS.values()]
    base = db.query(Transaction).filter(or_(*[col.is_(None) for col in value_columns]))
    scanned = updated = changed_fields = skipped_locked = 0
    last_id: Optional[int] = None
    while limit is None or scanned < limit:
        query = base if last_id is None else base.filter(Transaction.id < last_id)
        size = batch_size if limit is None else min(batch_size, limit - scanned)
        batch = query.order_by(Transaction.id.desc()).limit(size).all()
        if not batch:
            break
        for txn in batch:
            scanned += 1
            when = txn.transaction_date
            if when is not None and any(start <= when.date() <= end for start, end in locks):
                skipped_locked += 1
                continue
            patch = {
                field: value
                for field, value in defaults_for_new(db, operator_raw=txn.operator_raw, transaction_date=when).items()
                if getattr(txn, annotation_service.ANNOTATION_FIELDS[field][0]) in (None, "")
            }
            if not patch:
                continue
            result = annotation_service.apply_annotation_patch(
                db, txn, patch, source="rule", actor_user_id=actor_user_id, validate=False,
            )
            if result["changed"]:
                updated += 1
                changed_fields += len(result["changed"])
        last_id = batch[-1].id
        db.commit()

    return {
        "scanned": scanned,
        "updated": updated,
        "changed_fields": changed_fields,
        "skipped_locked": skipped_locked,
    }


def apply_rules(
    db: Session,
    *,
    limit: int = 5000,
    date_from: Optional[datetime] = None,
    date_to: Optional[datetime] = None,
    fields: Iterable[str] = RULE_FIELDS,
    dry_run: bool = False,
    actor_user_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Fill empty annotation fields on existing transactions from the rules.

    Idempotent and safe to re-run: it writes only where a field is empty or was
    itself written by automation, so nothing a person entered is disturbed.
    """
    fields = [f for f in fields if f in RULE_FIELDS]
    rules = get_rules(db)
    if not rules:
        return {"scanned": 0, "updated": 0, "changed_fields": 0, "dry_run": dry_run}

    query = db.query(Transaction).filter(Transaction.operator_raw.isnot(None))
    if date_from is not None:
        query = query.filter(Transaction.transaction_date >= date_from)
    if date_to is not None:
        query = query.filter(Transaction.transaction_date <= date_to)

    # Only rows still missing at least one of the requested fields.
    gaps = [
        getattr(Transaction, annotation_service.ANNOTATION_FIELDS[f][0]).is_(None)
        for f in fields
    ]
    if gaps:
        from sqlalchemy import or_
        query = query.filter(or_(*gaps))

    scanned = updated = changed_fields = 0
    # Newest first: with an arbitrary order the limit could leave out exactly the
    # receipts that just arrived.
    for txn in query.order_by(Transaction.id.desc()).limit(limit).all():
        scanned += 1
        rule = rules.get(_normalized_key(txn.operator_raw))
        if not rule:
            continue
        patch = {field: rule[field] for field in fields if field in rule}
        if not patch:
            continue
        result = annotation_service.apply_annotation_patch(
            db, txn, patch, source="rule", actor_user_id=actor_user_id, validate=False,
        )
        if result["changed"]:
            updated += 1
            changed_fields += len(result["changed"])

    if dry_run:
        db.rollback()
    else:
        db.commit()

    return {
        "scanned": scanned,
        "updated": updated,
        "changed_fields": changed_fields,
        "dry_run": dry_run,
    }


def coverage_report(
    db: Session,
    *,
    fields: Iterable[str] = RULE_FIELDS,
    min_support: int = DEFAULT_MIN_SUPPORT,
    purities: Tuple[float, ...] = (1.0, 0.9, 0.8),
) -> Dict[str, Any]:
    """How much of the annotated corpus each purity threshold would cover.

    Useful as a sanity check after an import: a sharp drop against the expected
    figures usually means operator-key normalization broke.
    """
    report: Dict[str, Any] = {}
    for field in fields:
        if field not in RULE_FIELDS:
            continue
        counts: Dict[str, Dict[str, int]] = defaultdict(lambda: defaultdict(int))
        for operator_raw, value in annotation_service.rows_for_rule_training(db, field=field):
            key = _normalized_key(operator_raw)
            text = _value_text(value)
            if key and text:
                counts[key][text] += 1

        total_rows = sum(sum(v.values()) for v in counts.values())
        per_threshold = {}
        for threshold in purities:
            covered = 0
            for values in counts.values():
                group_total = sum(values.values())
                best = max(values.values())
                if group_total >= min_support and best / group_total >= threshold:
                    covered += group_total
            per_threshold[f"{threshold:g}"] = {
                "rows": covered,
                "pct": round(covered * 100 / total_rows, 1) if total_rows else 0.0,
            }
        report[field] = {
            "annotated_rows": total_rows,
            "distinct_operators": len(counts),
            "coverage": per_threshold,
        }
    return report
