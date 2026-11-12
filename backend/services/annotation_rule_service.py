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

«Подробности» is learned more narrowly than the codes. A note is not a property
of the merchant: replayed in date order over the workbook-confirmed notes, the
merchant alone predicted the next note correctly 84 times in 100, which means
one wrong note in six. Three things bring that to 95 in 100, and they are what
`details_decision` requires:

* the key is the operator *and* the own card *and* the direction — the same
  gateway carries one note from one card and another from the next;
* at least five confirmed receipts, four in five of them with that note;
* the three most recent confirmed receipts agree — notes drift, and a pattern
  that has just changed is not a pattern.

A note is copied whole or not at all. Whatever identifier it carries (a card,
a person, a number) is opaque here: nothing is interpreted and nothing is
generated. Weaker evidence is reported as a suggestion and never written.
"""
from __future__ import annotations

import logging
import os
import threading
import time
import unicodedata
from collections import Counter, defaultdict
from datetime import date, datetime
from decimal import Decimal
from typing import Any, Dict, Iterable, List, NamedTuple, Optional, Sequence, Tuple

from sqlalchemy.orm import Session

from database.models import AnnotationRule, Transaction, TransactionAnnotationEvent
from parsers.operator_mapper import OperatorMapper
from services import annotation_service

logger = logging.getLogger(__name__)

RULE_FIELDS = ("details", "person", "article")

DEFAULT_MIN_SUPPORT = int(os.getenv("ANNOTATION_RULE_MIN_SUPPORT", "3"))
DEFAULT_MIN_PURITY = float(os.getenv("ANNOTATION_RULE_MIN_PURITY", "0.9"))

#: What a note needs before it is written (see the module docstring). Measured
#: on the confirmed notes in date order, as predictions / right / empty receipts
#: reached: purity 0.9 — 68 / 65 / 23; 0.8 — 79 / 75 / 59; 0.7 — 96 / 87 / 66.
#: 0.8 is where coverage grows and precision has not yet fallen.
DETAILS_MIN_SUPPORT = int(os.getenv("ANNOTATION_DETAILS_MIN_SUPPORT", "5"))
DETAILS_MIN_PURITY = float(os.getenv("ANNOTATION_DETAILS_MIN_PURITY", "0.8"))
DETAILS_RECENT_AGREE = int(os.getenv("ANNOTATION_DETAILS_RECENT_AGREE", "3"))
#: What it needs to be mentioned as a suggestion: about 86 right in 100.
DETAILS_SUGGEST_SUPPORT = 3

#: How old the note rules may grow before the background pass relearns them, in
#: hours. 0 (the default) leaves relearning to a person: the first relearning on
#: a database also lets `fill_defaults` write notes into old receipts, and that
#: is done once, with a reviewed manifest and a backup, not by a timer.
DETAILS_RULES_REFRESH_HOURS = float(os.getenv("ANNOTATION_DETAILS_RULES_REFRESH_HOURS", "0"))

#: Separates the parts of a details key. A normalized operator key holds only
#: letters, digits and single spaces, so it can never contain this.
_DETAILS_KEY_SEPARATOR = " | "

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
# Details: operator + own card + direction, with recent agreement
# --------------------------------------------------------------------------

def details_key(operator_raw: Optional[str], card_last_4: Optional[str], transaction_type: Optional[str]) -> str:
    """`OPERATOR KEY | ПК 1234 | DEBIT`: what two receipts must share to share a note."""
    operator = _normalized_key(operator_raw)
    if not operator:
        return ""
    card = "".join(ch for ch in str(card_last_4 or "") if ch.isdigit())[-4:] or "-"
    return _DETAILS_KEY_SEPARATOR.join((operator, f"ПК {card}", str(transaction_type or "-").upper()))


def is_details_key(key: Optional[str]) -> bool:
    return _DETAILS_KEY_SEPARATOR in (key or "")


def _loose(value: Optional[str]) -> str:
    """A note without spacing, punctuation and case: `вн.пер.на1234` is `Вн. пер. на 1234`."""
    text = unicodedata.normalize("NFKC", value or "").casefold()
    return "".join(ch for ch in text if ch.isalnum())


class DetailsDecision(NamedTuple):
    #: The note as its most recent confirmed receipt spells it, or None.
    value: Optional[str]
    #: `confident` is written; `suggestion` is only reported; `none` is neither.
    tier: str
    reason: str
    support: int
    total: int


def details_decision(
    history: Sequence[str],
    *,
    min_support: int = DETAILS_MIN_SUPPORT,
    min_purity: float = DETAILS_MIN_PURITY,
    recent: int = DETAILS_RECENT_AGREE,
) -> DetailsDecision:
    """What the confirmed notes of one key, oldest first, allow.

    The candidate is always the note of the newest confirmed receipt: an older
    majority that the latest receipts have left is the past, not a rule.
    """
    total = len(history)
    if not total:
        return DetailsDecision(None, "none", "no_confirmed_similar", 0, 0)
    candidate = _loose(history[-1])
    support = sum(1 for note in history if _loose(note) == candidate)
    if total < DETAILS_SUGGEST_SUPPORT:
        return DetailsDecision(None, "none", "too_few_confirmed", support, total)
    tail = history[-max(recent, 1):]
    if len(tail) < recent or any(_loose(note) != candidate for note in tail):
        return DetailsDecision(None, "none", "recent_values_differ", support, total)
    if total >= min_support and support / total >= min_purity:
        return DetailsDecision(history[-1], "confident", "confirmed_similar", support, total)
    reason = "few_confirmed" if total < min_support else "mixed_history"
    return DetailsDecision(history[-1], "suggestion", reason, support, total)


def _details_histories(db: Session) -> Dict[str, List[str]]:
    """Confirmed notes per details key, oldest first."""
    histories: Dict[str, List[str]] = defaultdict(list)
    for operator_raw, card, kind, note in annotation_service.rows_for_details_training(db):
        key = details_key(operator_raw, card, kind)
        text = _value_text(note)
        if key and text and _loose(text):
            histories[key].append(text)
    return histories


def _details_candidates(db: Session, *, min_support: int, min_purity: float) -> List[Dict[str, Any]]:
    candidates: List[Dict[str, Any]] = []
    for key, history in _details_histories(db).items():
        decision = details_decision(history, min_support=min_support, min_purity=min_purity)
        if decision.tier != "confident":
            continue
        candidates.append({
            "operator_key": key,
            "field": "details",
            "value": decision.value,
            "support": decision.support,
            "total": decision.total,
            "purity": round(decision.support / decision.total, 3),
        })
    return candidates


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
        if field == "details":
            # Notes have their own key and their own measured bar (DETAILS_*);
            # `min_support` and `min_purity` are the bar for the codes.
            result[field] = _details_candidates(
                db, min_support=DETAILS_MIN_SUPPORT, min_purity=DETAILS_MIN_PURITY,
            )
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

def refresh_details_rules(db: Session, *, max_age_hours: Optional[float] = None) -> Optional[Dict[str, Any]]:
    """Relearn the note rules once they are older than `max_age_hours`.

    New receipts get their notes from the rules as they stand, so rules that
    are never relearned keep writing a note the confirmed receipts have since
    left. Relearning on a schedule switches such a rule off (and brings in the
    patterns people confirmed meanwhile) without anyone pressing a button.
    Only the rules change here: a note already written is taken back only by
    `apply_rules(revalidate=True)`, on request.

    Returns the `recompute` summary, or None when the rules are fresh enough or
    the refresh is off (`ANNOTATION_DETAILS_RULES_REFRESH_HOURS`, 0 by default).
    """
    from sqlalchemy import func

    hours = DETAILS_RULES_REFRESH_HOURS if max_age_hours is None else max_age_hours
    if hours <= 0:
        return None
    latest = (
        db.query(func.max(AnnotationRule.computed_at))
        .filter(AnnotationRule.field == "details")
        .filter(AnnotationRule.operator_key.contains(_DETAILS_KEY_SEPARATOR))
        .scalar()
    )
    if latest is not None and (datetime.utcnow() - latest).total_seconds() < hours * 3600:
        return None
    return recompute(db, fields=("details",))


def _load_rules(db: Session) -> Dict[str, Dict[str, str]]:
    """`{key: {field: value}}` for every enabled rule.

    The key is the operator for a code, and `details_key` for a note. A note
    rule under a bare operator key is honoured only when a person pinned it:
    the computed ones of earlier releases were learned from the operator alone
    and are no longer trusted (the next recompute disables them).
    """
    mapping: Dict[str, Dict[str, str]] = defaultdict(dict)
    for row in db.query(AnnotationRule).filter(AnnotationRule.is_enabled.is_(True)).all():
        if row.field == "details" and not is_details_key(row.operator_key) and not row.is_pinned:
            continue
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


def rule_patch(
    rules: Dict[str, Dict[str, str]],
    *,
    operator_raw: Optional[str],
    card_last_4: Optional[str] = None,
    transaction_type: Optional[str] = None,
) -> Dict[str, Any]:
    """What the rules say about one receipt: codes by operator, the note by its own key."""
    patch: Dict[str, Any] = dict(rules.get(_normalized_key(operator_raw), {}))
    # A note a person pinned to the operator stands; otherwise the learned one.
    if "details" not in patch:
        note = rules.get(details_key(operator_raw, card_last_4, transaction_type), {}).get("details")
        if note:
            patch["details"] = note
    return patch


def defaults_for_new(
    db: Session,
    *,
    operator_raw: Optional[str],
    transaction_date: Optional[datetime],
    card_last_4: Optional[str] = None,
    transaction_type: Optional[str] = None,
) -> Dict[str, Any]:
    """Annotation defaults for a freshly created transaction.

    The period defaults to the month of the operation. That is a guess -- the
    client's own ledger books some operations into a different month -- so it is
    written with a 'rule' provenance that any later import may correct.

    A learned rule is specific to the merchant, so it wins over the global
    default person.
    """
    patch = rule_patch(
        get_rules(db), operator_raw=operator_raw, card_last_4=card_last_4, transaction_type=transaction_type,
    )
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
                for field, value in defaults_for_new(
                    db, operator_raw=txn.operator_raw, transaction_date=when,
                    card_last_4=txn.card_last_4, transaction_type=txn.transaction_type,
                ).items()
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


def _retract_rule_note(db: Session, txn: Transaction, actor_user_id: Optional[int]) -> None:
    """Take back a note a rule wrote and the confirmed receipts no longer support.

    `apply_annotation_patch` refuses to blank a value on a rule's behalf, and
    rightly so for everything else. This is the one case where blanking is the
    correction; it is journalled like any other change, so it can be undone.
    """
    db.add(TransactionAnnotationEvent(
        transaction_id=txn.id,
        field="details",
        before_value=txn.annotation_details,
        after_value=None,
        before_src=txn.annotation_details_src,
        source="rule",
        actor_user_id=actor_user_id,
    ))
    txn.annotation_details = None
    txn.annotation_details_src = None
    txn.annotation_updated_at = datetime.utcnow()
    txn.annotation_updated_by = actor_user_id


def apply_rules(
    db: Session,
    *,
    limit: int = 5000,
    date_from: Optional[datetime] = None,
    date_to: Optional[datetime] = None,
    fields: Iterable[str] = RULE_FIELDS,
    dry_run: bool = False,
    actor_user_id: Optional[int] = None,
    revalidate: bool = False,
) -> Dict[str, Any]:
    """Fill empty annotation fields on existing transactions from the rules.

    Idempotent and safe to re-run: it writes only where a field is empty or was
    itself written by automation, so nothing a person entered is disturbed.
    Receipts inside a locked period are left alone, as `fill_defaults` leaves them.

    With `revalidate` the notes an earlier rule wrote are looked at again: one
    whose rule has changed is corrected, one whose rule is gone is taken back.
    A wrong match therefore lasts only until the confirmed receipts contradict it.
    """
    from sqlalchemy import or_

    fields = [f for f in fields if f in RULE_FIELDS]
    recheck_notes = revalidate and "details" in fields
    rules = get_rules(db)
    if not rules and not recheck_notes:
        return {
            "scanned": 0, "updated": 0, "changed_fields": 0, "skipped_locked": 0, "retracted": 0,
            "dry_run": dry_run,
        }

    query = db.query(Transaction).filter(Transaction.operator_raw.isnot(None))
    if date_from is not None:
        query = query.filter(Transaction.transaction_date >= date_from)
    if date_to is not None:
        query = query.filter(Transaction.transaction_date <= date_to)

    # Only rows still missing at least one of the requested fields, and with
    # `revalidate` the rows whose note a rule wrote.
    wanted = [
        getattr(Transaction, annotation_service.ANNOTATION_FIELDS[f][0]).is_(None)
        for f in fields
    ]
    if recheck_notes:
        wanted.append(Transaction.annotation_details_src == "rule")
    if wanted:
        query = query.filter(or_(*wanted))

    locks = _locked_ranges(db)
    scanned = updated = changed_fields = skipped_locked = retracted = 0
    # Newest first: with an arbitrary order the limit could leave out exactly the
    # receipts that just arrived.
    for txn in query.order_by(Transaction.id.desc()).limit(limit).all():
        scanned += 1
        when = txn.transaction_date
        if when is not None and any(start <= when.date() <= end for start, end in locks):
            skipped_locked += 1
            continue
        found = rule_patch(
            rules, operator_raw=txn.operator_raw,
            card_last_4=txn.card_last_4, transaction_type=txn.transaction_type,
        )
        patch = {field: found[field] for field in fields if field in found}
        changed = 0
        if patch:
            result = annotation_service.apply_annotation_patch(
                db, txn, patch, source="rule", actor_user_id=actor_user_id, validate=False,
            )
            changed = len(result["changed"])
        if recheck_notes and "details" not in found and txn.annotation_details_src == "rule":
            _retract_rule_note(db, txn, actor_user_id)
            retracted += 1
            changed += 1
        if changed:
            updated += 1
            changed_fields += changed

    if dry_run:
        db.rollback()
    else:
        db.commit()

    return {
        "scanned": scanned,
        "updated": updated,
        "changed_fields": changed_fields,
        "skipped_locked": skipped_locked,
        "retracted": retracted,
        "dry_run": dry_run,
    }


def details_report(db: Session) -> Dict[str, Any]:
    """What a pass over «Подробности» would do, in counts only.

    Reads a few columns and writes nothing, so it also runs through a read-only
    role. It decides from the confirmed receipts directly, exactly as
    `recompute` followed by `apply_rules(revalidate=True)` would.
    """
    histories = _details_histories(db)
    decisions = {key: details_decision(history) for key, history in histories.items()}
    pinned = {
        key: value
        for key, value in db.query(AnnotationRule.operator_key, AnnotationRule.value)
        .filter(AnnotationRule.field == "details")
        .filter(AnnotationRule.is_pinned.is_(True))
        .filter(AnnotationRule.is_enabled.is_(True))
    }
    locks = _locked_ranges(db)
    nothing = DetailsDecision(None, "none", "no_confirmed_similar", 0, 0)

    report: Dict[str, Any] = {
        "receipts": 0,
        "confirmed": 0,
        "keys": len(histories),
        "rules": sum(1 for decision in decisions.values() if decision.tier == "confident"),
        "empty": 0,
        "would_fill": 0,
        "locked": 0,
        "suggestions": Counter(),
        "unresolved": Counter(),
        "rule_written": 0,
        "rule_supported": 0,
        "rule_would_change": 0,
        "rule_would_retract": 0,
        "agent_written": 0,
        "agent_agrees": 0,
        "agent_differs": 0,
        "unknown_origin": 0,
    }
    rows = db.query(
        Transaction.operator_raw, Transaction.card_last_4, Transaction.transaction_type,
        Transaction.transaction_date, Transaction.annotation_details, Transaction.annotation_details_src,
    ).filter(Transaction.operator_raw.isnot(None))
    for operator_raw, card, kind, when, note, source in rows.yield_per(1000):
        report["receipts"] += 1
        note = (note or "").strip()
        if source in ("manual", "import"):
            report["confirmed"] += 1
            continue
        pinned_note = pinned.get(_normalized_key(operator_raw))
        if pinned_note:
            decision = DetailsDecision(pinned_note, "confident", "pinned_rule", 0, 0)
        else:
            decision = decisions.get(details_key(operator_raw, card, kind), nothing)
        locked = bool(when and any(start <= when.date() <= end for start, end in locks))
        agrees = decision.tier == "confident" and _loose(decision.value) == _loose(note)

        if not note:
            report["empty"] += 1
            if decision.tier == "confident":
                report["locked" if locked else "would_fill"] += 1
            elif decision.tier == "suggestion":
                report["suggestions"][decision.reason] += 1
            else:
                report["unresolved"][decision.reason] += 1
        elif source == "rule":
            report["rule_written"] += 1
            if agrees:
                report["rule_supported"] += 1
            elif locked:
                report["locked"] += 1
            elif decision.tier == "confident":
                report["rule_would_change"] += 1
            else:
                report["rule_would_retract"] += 1
        elif source == "agent":
            report["agent_written"] += 1
            if decision.tier == "confident":
                report["agent_agrees" if agrees else "agent_differs"] += 1
        else:
            report["unknown_origin"] += 1
    return report


def details_plan(db: Session) -> Dict[str, List[Dict[str, Any]]]:
    """Receipt by receipt, what a pass over «Подробности» would write or only suggest.

    The same decisions as `details_report`, kept with the receipt they belong
    to: `fills` is what `recompute` followed by `apply_rules` writes into empty
    notes, `suggestions` is what it reports and never writes. Reads only and
    writes nothing. The result names receipts and notes: it belongs in a
    protected change manifest, never in a log or a ticket.
    """
    histories = _details_histories(db)
    decisions = {key: details_decision(history) for key, history in histories.items()}
    pinned = {
        key: value
        for key, value in db.query(AnnotationRule.operator_key, AnnotationRule.value)
        .filter(AnnotationRule.field == "details")
        .filter(AnnotationRule.is_pinned.is_(True))
        .filter(AnnotationRule.is_enabled.is_(True))
    }
    locks = _locked_ranges(db)
    nothing = DetailsDecision(None, "none", "no_confirmed_similar", 0, 0)
    plan: Dict[str, List[Dict[str, Any]]] = {"fills": [], "suggestions": [], "locked": []}
    rows = db.query(
        Transaction.id, Transaction.operator_raw, Transaction.card_last_4, Transaction.transaction_type,
        Transaction.transaction_date, Transaction.annotation_details, Transaction.annotation_details_src,
    ).filter(Transaction.operator_raw.isnot(None)).order_by(Transaction.id)
    for tx_id, operator_raw, card, kind, when, note, source in rows.yield_per(1000):
        if source in ("manual", "import") or (note or "").strip():
            continue
        pinned_note = pinned.get(_normalized_key(operator_raw))
        if pinned_note:
            key = _normalized_key(operator_raw)
            decision = DetailsDecision(pinned_note, "confident", "pinned_rule", 0, 0)
        else:
            key = details_key(operator_raw, card, kind)
            decision = decisions.get(key, nothing)
        if decision.tier == "none":
            continue
        entry = {
            "id": int(tx_id),
            # The rule that speaks for this receipt: taking the note back means stopping that rule too.
            "key": key,
            "before": {"details": None, "src": source},
            "after": {"details": decision.value, "src": "rule"},
            "reason": decision.reason,
            "support": decision.support,
            "total": decision.total,
        }
        if decision.tier == "suggestion":
            plan["suggestions"].append(entry)
        elif when and any(start <= when.date() <= end for start, end in locks):
            plan["locked"].append(entry)
        else:
            plan["fills"].append(entry)
    return plan


def details_report_lines(report: Dict[str, Any]) -> List[str]:
    """`details_report` as text. Counts and reasons only: no note, operator or card."""
    lines = [
        f"receipts looked at:                    {report['receipts']}",
        f"  confirmed by a person or workbook:   {report['confirmed']}",
        f"  confirmed patterns (operator+ПК+type): {report['keys']}, of them rules: {report['rules']}",
        f"receipts without «Подробности»:        {report['empty']}",
        f"  would be filled:                     {report['would_fill']}",
        f"  skipped, locked period:              {report['locked']}",
        f"  suggestion only, not written:        {sum(report['suggestions'].values())}",
        f"  left empty:                          {sum(report['unresolved'].values())}",
    ]
    if report["suggestions"]:
        lines.append("\nsuggestions, by why they are not written:")
        lines.extend(f"  {count:6d}  {reason}" for reason, count in report["suggestions"].most_common())
    if report["unresolved"]:
        lines.append("\nleft empty, by reason:")
        lines.extend(f"  {count:6d}  {reason}" for reason, count in report["unresolved"].most_common())
    lines.append(f"\nnotes an earlier rule wrote:           {report['rule_written']}")
    lines.append(f"  still supported:                     {report['rule_supported']}")
    lines.append(f"  --revalidate would correct:          {report['rule_would_change']}")
    lines.append(f"  --revalidate would take back:        {report['rule_would_retract']}")
    lines.append(f"notes the agent wrote (never learned from, not changed): {report['agent_written']}")
    lines.append(f"  a confirmed pattern agrees / differs: {report['agent_agrees']} / {report['agent_differs']}")
    if report["unknown_origin"]:
        lines.append(f"notes of unknown origin (not changed): {report['unknown_origin']}")
    return lines


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
        if field == "details":
            # Notes are grouped by their own key and compared loosely, as the
            # rules are; "distinct_operators" then counts those keys.
            for key, history in _details_histories(db).items():
                for note in history:
                    counts[key][_loose(note)] += 1
        else:
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
