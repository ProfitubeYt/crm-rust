"""Fill the application of old receipts from the operator dictionary.

The parser stamps ``application_mapped`` only at parse time, so receipts parsed
before their operator reached ``operator_reference`` stay empty forever. This
runs the same matcher over them after the fact.

Only empty applications are filled: a value someone typed is never replaced.
Receipts inside an active locked period are left alone, as the automation
routes do. ``is_p2p`` changes only when asked, because it has no provenance
and a hand-set flag cannot be told apart from a parsed one.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field
from datetime import date
from typing import List, Optional

from sqlalchemy import func, or_
from sqlalchemy.orm import Session

from database.models import Transaction
from parsers.operator_mapper import OperatorMapper
from services.period_lock_service import period_lock_service


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
) -> BackfillReport:
    """Match every receipt without an application; write only when ``apply``."""
    mapper = OperatorMapper(db)
    mapper.refresh_cache()
    locks = period_lock_service.get_active_locks(db)

    query = (
        db.query(Transaction)
        .filter(or_(Transaction.application_mapped.is_(None), func.trim(Transaction.application_mapped) == ""))
        .filter(Transaction.operator_raw.isnot(None))
        .order_by(Transaction.id)
    )
    if limit:
        query = query.limit(limit)

    report = BackfillReport(applied=apply)
    for tx in query.all():
        report.candidates += 1
        match = mapper.map_operator_details(tx.operator_raw or "")
        if not match or not match.get("application_name"):
            report.unmatched_operators[(tx.operator_raw or "").strip()] += 1
            continue
        if tx.transaction_date and _locked(tx.transaction_date.date(), locks):
            report.locked += 1
            continue

        report.matched += 1
        report.by_application[match["application_name"]] += 1
        report.ids.append(int(tx.id))
        p2p = match.get("is_p2p")
        p2p_differs = set_p2p and p2p is not None and bool(tx.is_p2p) != bool(p2p)
        if p2p_differs:
            report.p2p_changed += 1
        if apply:
            tx.application_mapped = match["application_name"]
            if p2p_differs:
                tx.is_p2p = bool(p2p)

    if apply:
        db.commit()
    else:
        db.rollback()
    return report
