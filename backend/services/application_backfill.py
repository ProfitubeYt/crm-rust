"""Fill the application of receipts from the evidence in their operator line.

The parser decides the application once, at parse time, so a receipt parsed
before its operator was known stays empty forever. This runs the same detector
(`services.application_detection`) over the receipts after the fact.

What it will and will not touch:

* It fills an empty application. A value a person set is never replaced, an
  application a person emptied is not filled back in, and a value of unknown
  origin is left alone; where such a value differs from what the evidence says
  the difference is counted for review, nothing more.
* Receipts inside an active locked period are left alone, as the automation
  routes do.
* With ``revalidate`` it also re-derives the values this module wrote earlier
  (`dict`, `rule`, `similar`): one that the evidence no longer supports is
  corrected or cleared, so a wrong dictionary line does not outlive its fix.
  An AI guess is replaced when the evidence names another application, and
  kept when the evidence is merely silent.
* ``is_p2p`` changes only when asked, because it has no provenance and a
  hand-set flag cannot be told apart from a parsed one.

Nothing is written unless ``apply`` is set, and a second run changes nothing.
The dry run reads a handful of columns only, so it also works through a
read-only role that may not see the whole table.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field
from datetime import date
from typing import Dict, List, Optional, Tuple

from sqlalchemy import func, literal, or_
from sqlalchemy.orm import Session

from database.models import Transaction
from services.application_detection import (
    DETECTED_SOURCES,
    Detection,
    detector_for,
    has_provenance,
    stamp_application,
)
from services.period_lock_service import period_lock_service

_WRITE_BATCH = 500


@dataclass
class BackfillReport:
    candidates: int = 0
    matched: int = 0
    locked: int = 0
    p2p_changed: int = 0
    applied: bool = False
    by_application: Counter = field(default_factory=Counter)
    unmatched_operators: Counter = field(default_factory=Counter)
    ids: List[int] = field(default_factory=list)
    #: Proposed fills by evidence tier: dictionary_exact, brand_in_operator, …
    by_reason: Counter = field(default_factory=Counter)
    #: Receipts left empty, by why: no_evidence, ambiguous, cash_or_terminal, …
    unresolved: Counter = field(default_factory=Counter)
    #: Candidate sets of the ambiguous receipts, e.g. "Paynet | Xazna".
    ambiguous: Counter = field(default_factory=Counter)
    #: Stored value of another origin vs what the evidence says: (stored, detected).
    disagreements: Counter = field(default_factory=Counter)
    #: Values this module wrote that the evidence no longer supports.
    stale: int = 0
    #: Of those, how many `revalidate` corrected or cleared.
    revalidated: int = 0
    #: Receipts whose application a person set or emptied; never touched.
    manual: int = 0
    #: Receipts with any application at all, before this run.
    already_set: int = 0
    total: int = 0

    @property
    def unmatched(self) -> int:
        return self.candidates - self.matched - self.locked


def _locked(day: date, locks) -> bool:
    return any(lock.date_from <= day <= lock.date_to for lock in locks)


def backfill_applications(
    db: Session,
    *,
    apply: bool = False,
    set_p2p: bool = False,
    limit: Optional[int] = None,
    revalidate: bool = False,
) -> BackfillReport:
    """Run the detector over every receipt; write only when ``apply``."""
    detector = detector_for(db, fresh=True)
    locks = period_lock_service.get_active_locks(db)
    provenance = has_provenance(db)

    source_column = Transaction.application_src if provenance else literal(None)
    query = (
        db.query(
            Transaction.id,
            Transaction.operator_raw,
            Transaction.application_mapped,
            source_column,
            Transaction.is_p2p,
            Transaction.transaction_date,
        )
        .filter(Transaction.operator_raw.isnot(None))
        .order_by(Transaction.id)
    )
    if limit:
        # The limit counts receipts without an application, as it always did.
        empty = (
            db.query(Transaction.id)
            .filter(or_(Transaction.application_mapped.is_(None), func.trim(Transaction.application_mapped) == ""))
            .filter(Transaction.operator_raw.isnot(None))
            .order_by(Transaction.id)
            .limit(limit)
            .subquery()
        )
        query = query.filter(Transaction.id.in_(db.query(empty.c.id)))

    report = BackfillReport(applied=apply)
    detections: Dict[str, Detection] = {}
    # id -> (application or None, source, confidence, is_p2p to set or None)
    writes: Dict[int, Tuple[Optional[str], Optional[str], Optional[float], Optional[bool]]] = {}

    for tx_id, operator_raw, stored, source, is_p2p, when in query.all():
        report.total += 1
        stored = (stored or "").strip()
        operator = (operator_raw or "").strip()
        detection = detections.get(operator)
        if detection is None:
            detection = detections[operator] = detector.detect(operator)
        locked = bool(when and _locked(when.date(), locks))

        if source == "manual":
            report.manual += 1
            if stored:
                report.already_set += 1
            continue

        if stored:
            report.already_set += 1
            if source in DETECTED_SOURCES:
                if detection.application == stored:
                    continue
                report.stale += 1
                if revalidate and not locked:
                    report.revalidated += 1
                    writes[int(tx_id)] = (detection.application, detection.source, detection.confidence, None)
            elif detection.found and detection.application != stored:
                if source == "ai":
                    # An AI guess gives way to evidence, never to its absence.
                    report.stale += 1
                    if revalidate and not locked:
                        report.revalidated += 1
                        writes[int(tx_id)] = (detection.application, detection.source, detection.confidence, None)
                else:
                    report.disagreements[(stored, detection.application)] += 1
            continue

        report.candidates += 1
        if not detection.found:
            report.unmatched_operators[operator] += 1
            report.unresolved[detection.reason] += 1
            if detection.candidates:
                report.ambiguous[" | ".join(sorted(detection.candidates))] += 1
            continue
        if locked:
            report.locked += 1
            continue

        report.matched += 1
        report.by_application[detection.application] += 1
        report.by_reason[detection.reason] += 1
        report.ids.append(int(tx_id))
        p2p = detection.is_p2p if set_p2p else None
        if p2p is not None and bool(is_p2p) == bool(p2p):
            p2p = None
        if p2p is not None:
            report.p2p_changed += 1
        writes[int(tx_id)] = (detection.application, detection.source, detection.confidence, p2p)

    if apply and writes:
        ids = sorted(writes)
        for start in range(0, len(ids), _WRITE_BATCH):
            page = ids[start:start + _WRITE_BATCH]
            for tx in db.query(Transaction).filter(Transaction.id.in_(page)):
                application, source, confidence, p2p = writes[int(tx.id)]
                stamp_application(tx, application, source=source, confidence=confidence)
                if p2p is not None:
                    tx.is_p2p = bool(p2p)
        db.commit()
    else:
        db.rollback()
    return report


def report_lines(report: BackfillReport, *, top: int = 0, set_p2p: bool = False) -> List[str]:
    """The report as text. Operator lines are printed only when ``top`` asks for them:
    a few of them carry a person's name, and the default output must be safe to paste."""
    lines = ["APPLIED" if report.applied else "DRY RUN, nothing written"]
    lines.append(f"receipts looked at:            {report.total}")
    lines.append(f"  already with an application: {report.already_set}")
    lines.append(f"  set or emptied by a person:  {report.manual}")
    lines.append(f"receipts without application:  {report.candidates}")
    lines.append(f"filled from evidence:          {report.matched}")
    lines.append(f"skipped, locked period:        {report.locked}")
    lines.append(f"left empty:                    {report.unmatched}")
    if set_p2p:
        lines.append(f"is_p2p changed:                {report.p2p_changed}")
    if report.by_reason:
        lines.append("\nfilled, by evidence:")
        lines.extend(f"  {count:6d}  {reason}" for reason, count in report.by_reason.most_common())
    if report.by_application:
        lines.append("\nfilled, by application:")
        lines.extend(f"  {count:6d}  {app}" for app, count in report.by_application.most_common())
    if report.unresolved:
        lines.append("\nleft empty, by reason:")
        lines.extend(f"  {count:6d}  {reason}" for reason, count in report.unresolved.most_common())
    if report.ambiguous:
        lines.append("\nambiguous, by candidates (for review):")
        lines.extend(f"  {count:6d}  {names}" for names, count in report.ambiguous.most_common())
    if report.disagreements:
        lines.append("\nstored value differs from the evidence (not changed, for review):")
        lines.extend(
            f"  {count:6d}  stored {stored!r}, evidence says {detected!r}"
            for (stored, detected), count in report.disagreements.most_common()
        )
    if report.stale:
        lines.append(
            f"\nearlier automatic values the evidence no longer supports: {report.stale}"
            f" (corrected: {report.revalidated})"
        )
    if top and report.unmatched_operators:
        lines.append("\ntop operators left empty:")
        lines.extend(f"  {count:6d}  {op}" for op, count in report.unmatched_operators.most_common(top))
    return lines
