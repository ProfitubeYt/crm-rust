"""The shared application dictionary: what may be added to it, and what still waits for a person.

The dictionary (`operator_reference`) grows in one way only: a person adds or
activates a line. Nothing learns into it by itself, and the optional AI only
ever leaves inactive suggestions. This module is the review side of that:

* `plan_import` says, line by line, what adding researched lines would do
  (`add`, `covered`, `exists`, or a conflict that is never overwritten), and
  writes nothing;
* `review_operators` lists the operator lines of receipts that have no
  application, with how often they occur, what the research file says about
  them and a review status, so an unknown operator is seen and decided by a
  person instead of being guessed.

The research file (`data/operator_dictionary/research_*.json`) holds public
organisation and service names with their sources, never receipt data. Only
its `verified` section is ever offered for import; `needs_owner_review` and
`not_an_application` are shown to the reviewer and go nowhere by themselves.
"""
from __future__ import annotations

import json
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

from sqlalchemy import func
from sqlalchemy.orm import Session

from database.models import OperatorReference, Transaction
from parsers.operator_mapper import OperatorForm, OperatorMapper, operator_form
from services.application_detection import (
    _PERSON_MARKERS,
    ApplicationDetector,
    ai_may_see,
    confirmed_applications,
    detector_for,
    has_provenance,
)

RESEARCH_DIR = Path(__file__).resolve().parents[1] / "data" / "operator_dictionary"

#: Legal forms that open or close a merchant's line: the line names a company, not an application.
_COMPANY_MARKERS = frozenset({
    "OOO", "MCHJ", "LLC", "XK", "SP", "AJ", "AO", "JSC", "OK", "OT", "YT", "KHK", "SAS", "QK", "FX",
})

#: Review statuses, in the order a reviewer wants them.
STATUSES = (
    "verified_pending_import",
    "needs_owner_review",
    "unreviewed",
    "not_an_application",
    "merchant_or_individual",
    "no_operator_or_cash",
)


def load_research(path: Optional[Path] = None) -> Dict[str, Any]:
    """The newest research file, or the one given."""
    if path is None:
        files = sorted(RESEARCH_DIR.glob("research_*.json"))
        if not files:
            return {"verified": [], "needs_owner_review": [], "not_an_application": []}
        path = files[-1]
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


@dataclass(frozen=True)
class DictionaryLine:
    """One researched line that may be added to the dictionary."""

    operator: str
    application: str
    is_p2p: bool
    entry_id: str = ""


def verified_lines(research: Dict[str, Any]) -> List[DictionaryLine]:
    lines: List[DictionaryLine] = []
    for entry in research.get("verified", []):
        for line in entry.get("lines", []):
            lines.append(DictionaryLine(
                operator=str(line["operator"]),
                application=str(entry["application"]),
                is_p2p=bool(line.get("is_p2p", False)),
                entry_id=str(entry.get("id", "")),
            ))
    return lines


@dataclass
class PlannedLine:
    line: DictionaryLine
    #: add | covered | exists | conflict_dictionary | conflict_detector | conflict_confirmed | duplicate | invalid
    action: str
    detail: str = ""
    #: Receipts of exactly this operator name (country code and city aside).
    receipts: int = 0
    #: Stored applications on those receipts that differ from the line: {application: receipts}.
    stored_differs: Dict[str, int] = field(default_factory=dict)
    #: Inactive dictionary lines with the same operator text: {application: lines}.
    inactive_twins: Dict[str, int] = field(default_factory=dict)

    @property
    def adds(self) -> bool:
        return self.action == "add"


def _same(left: Optional[str], right: Optional[str]) -> bool:
    return (left or "").strip().casefold() == (right or "").strip().casefold()


def plan_import(db: Session, lines: Sequence[DictionaryLine]) -> List[PlannedLine]:
    """What adding `lines` as active dictionary lines would do. Reads only.

    A line is added only when nothing the dictionary, the brand rules or a
    person already says speaks against it. An existing line and a value a
    person set are never overwritten: a disagreement is reported as a conflict
    and the line is left out.
    """
    mapper = OperatorMapper(db)
    mapper.refresh_cache()
    confirmed = confirmed_applications(db, force=True)
    detector = ApplicationDetector(mapper, confirmed=confirmed)

    stored_rows: Dict[str, List[Tuple[str, bool]]] = {}
    for row in db.query(OperatorReference).all():
        key = OperatorMapper.normalize_operator(row.operator_name or "")
        stored_rows.setdefault(key, []).append(((row.application_name or "").strip(), bool(row.is_active)))

    # Receipts per operator name, with what is stored on them.
    by_name: Dict[str, Counter] = {}
    manual_by_name: Dict[str, Counter] = {}
    source_column = Transaction.application_src if has_provenance(db) else None
    columns = [Transaction.operator_raw, Transaction.application_mapped]
    if source_column is not None:
        columns.append(source_column)
    for row in db.query(*columns).filter(Transaction.operator_raw.isnot(None)):
        key = operator_form(row[0]).key
        if not key:
            continue
        application = (row[1] or "").strip()
        by_name.setdefault(key, Counter())[application] += 1
        if source_column is not None and row[2] == "manual" and application:
            manual_by_name.setdefault(key, Counter())[application] += 1

    planned: List[PlannedLine] = []
    seen: Dict[str, str] = {}
    for line in lines:
        form = operator_form(line.operator)
        application = (line.application or "").strip()
        item = PlannedLine(line=line, action="add")
        planned.append(item)
        if not form.full or not application:
            item.action, item.detail = "invalid", "operator or application is empty"
            continue

        stored = by_name.get(form.key, Counter())
        item.receipts = sum(stored.values())
        item.stored_differs = {name: count for name, count in stored.items() if name and not _same(name, application)}
        twins = stored_rows.get(form.full, [])
        item.inactive_twins = dict(Counter(name for name, active in twins if not active))

        earlier = seen.get(form.full)
        if earlier is not None:
            if _same(earlier, application):
                item.action, item.detail = "duplicate", "the same line stands earlier in this import"
            else:
                item.action, item.detail = "conflict_dictionary", f"this import also files the line under {earlier!r}"
            continue
        seen[form.full] = application

        active = [name for name, is_active in twins if is_active]
        if any(_same(name, application) for name in active):
            item.action, item.detail = "exists", "an active line with this operator and application is already there"
            continue
        if active:
            item.action, item.detail = "conflict_dictionary", f"an active line files this operator under {active[0]!r}"
            continue

        manual = manual_by_name.get(form.key)
        if manual and any(not _same(name, application) for name in manual):
            other = sorted(name for name in manual if not _same(name, application))
            item.action, item.detail = "conflict_confirmed", f"people set {', '.join(other)} on receipts of this operator"
            continue

        detection = detector.detect(line.operator)
        if detection.found and _same(detection.application, application):
            item.action, item.detail = "covered", f"already answered ({detection.reason})"
        elif detection.found:
            kind = "conflict_dictionary" if detection.source == "dict" else "conflict_detector"
            item.action, item.detail = kind, f"today's evidence says {detection.application!r} ({detection.reason})"
        elif detection.candidates:
            item.action = "conflict_detector"
            item.detail = f"today's evidence is undecided between {', '.join(detection.candidates)}"
    return planned


def preview_detector(db: Session, lines: Iterable[DictionaryLine]) -> ApplicationDetector:
    """The detector as it would be once `lines` are in the dictionary. Stores nothing."""
    detector = detector_for(db, fresh=True)
    detector.mapper.preview_rows((line.operator, line.application, line.is_p2p) for line in lines)
    return detector


def plan_lines(planned: Sequence[PlannedLine]) -> List[str]:
    """`plan_import` as text. Dictionary lines are public gateway names, safe to print."""
    counts = Counter(item.action for item in planned)
    out = ["dictionary import, nothing written"]
    out.append("  " + ", ".join(f"{action}: {count}" for action, count in sorted(counts.items())))
    for item in planned:
        extra = []
        if item.receipts:
            extra.append(f"{item.receipts} receipts")
        if item.stored_differs:
            extra.append("stored differently: " + ", ".join(f"{name} ×{count}" for name, count in sorted(item.stored_differs.items())))
        if item.inactive_twins:
            extra.append("inactive twin: " + ", ".join(sorted(item.inactive_twins)))
        if item.detail:
            extra.append(item.detail)
        p2p = " p2p" if item.line.is_p2p else ""
        out.append(f"  {item.action:20s} {item.line.operator!r} -> {item.line.application!r}{p2p}" + (f"  [{'; '.join(extra)}]" if extra else ""))
    return out


# --- Review of operators without an application -------------------------------

def _starts_with(tokens: Sequence[str], prefix: Sequence[str]) -> bool:
    return len(prefix) <= len(tokens) and tuple(tokens[:len(prefix)]) == tuple(prefix)


class _Research:
    """The research file indexed for lookup by operator name."""

    def __init__(self, research: Dict[str, Any]):
        self.by_name: Dict[str, Tuple[str, Dict[str, Any]]] = {}
        self.prefixes: List[Tuple[Tuple[str, ...], str, Dict[str, Any]]] = []
        for section in ("needs_owner_review", "not_an_application"):
            for entry in research.get(section, []):
                for line in entry.get("lines", []):
                    self.by_name[operator_form(line["operator"]).key] = (section, entry)
                for name in entry.get("exact", []):
                    self.by_name[OperatorMapper.normalize_operator(name)] = (section, entry)
                for prefix in entry.get("prefixes", []):
                    tokens = tuple(OperatorMapper.normalize_operator(prefix).split())
                    if tokens:
                        self.prefixes.append((tokens, section, entry))
        # The longest prefix speaks first: `HUMO TO VISA` before a bare `HUMO`.
        self.prefixes.sort(key=lambda item: -len(item[0]))

    def find(self, form: OperatorForm) -> Optional[Tuple[str, Dict[str, Any]]]:
        found = self.by_name.get(form.key)
        if found:
            return found
        for tokens, section, entry in self.prefixes:
            if _starts_with(form.tokens, tokens):
                return section, entry
        return None


def _is_merchant_line(operator_raw: str, form: OperatorForm) -> bool:
    """A line with an address, a person's name or a company's legal form: a merchant."""
    if not ai_may_see(operator_raw):
        return True
    tokens = set(form.tokens)
    return bool(tokens & _PERSON_MARKERS or tokens & _COMPANY_MARKERS)


def review_operators(db: Session, research: Optional[Dict[str, Any]] = None) -> List[Dict[str, Any]]:
    """Operator lines of receipts without an application, most frequent first.

    Each item says how many receipts carry the line, how many of them have no
    application, when it was last seen, a review `status` and, where the
    research file has one, the `candidate` with its confidence and sources.
    `shareable` is false for a line with an address or a person's name: it is
    counted, and shown only to somebody who may see the receipts themselves.
    """
    research = research if research is not None else load_research()
    index = _Research(research)
    verified = verified_lines(research)
    entries = {entry.get("id", ""): entry for entry in research.get("verified", [])}
    today = detector_for(db, fresh=True)
    after = preview_detector(db, verified)
    entry_of_application: Dict[str, str] = {line.application.casefold(): line.entry_id for line in verified}
    inactive: Dict[str, List[str]] = {}
    for row in db.query(OperatorReference).filter(OperatorReference.is_active.is_(False)):
        inactive.setdefault(OperatorMapper.normalize_operator(row.operator_name or ""), []).append(row.application_name)

    groups: Dict[str, Dict[str, Any]] = {}
    rows = db.query(
        Transaction.operator_raw, Transaction.application_mapped, func.count(Transaction.id), func.max(Transaction.transaction_date),
    ).filter(Transaction.operator_raw.isnot(None)).group_by(Transaction.operator_raw, Transaction.application_mapped)
    for operator_raw, application, count, last in rows:
        operator = (operator_raw or "").strip()
        group = groups.setdefault(operator, {"receipts": 0, "without_application": 0, "last_seen": None})
        group["receipts"] += int(count)
        if not (application or "").strip():
            group["without_application"] += int(count)
        if last is not None and (group["last_seen"] is None or last > group["last_seen"]):
            group["last_seen"] = last

    items: List[Dict[str, Any]] = []
    for operator, group in groups.items():
        if not group["without_application"]:
            continue
        form = operator_form(operator)
        detection = today.detect(operator)
        if detection.found:
            # Answered by today's dictionary: the historical pass fills it, nothing to review.
            continue
        item: Dict[str, Any] = {
            "operator": operator,
            "receipts": group["receipts"],
            "without_application": group["without_application"],
            "last_seen": group["last_seen"].date().isoformat() if group["last_seen"] else None,
            "status": "unreviewed",
            "candidate": None,
            "confidence": None,
            "reason": detection.reason,
            "sources": [],
            "inactive_suggestions": sorted(set(inactive.get(form.full, []))),
            "shareable": ai_may_see(operator),
        }
        items.append(item)

        if detection.reason in ("no_operator_in_receipt", "cash_or_terminal"):
            item["status"] = "no_operator_or_cash"
            continue
        proposed = after.detect(operator)
        if proposed.found:
            entry = entries.get(entry_of_application.get(proposed.application.casefold(), ""), {})
            item.update(
                status="verified_pending_import",
                candidate=proposed.application,
                confidence=entry.get("confidence"),
                sources=[source["url"] for source in entry.get("sources", [])],
            )
            continue
        found = index.find(form)
        # A line with an address is a merchant whatever its first words are.
        if found and (found[0] != "not_an_application" or item["shareable"]):
            section, entry = found
            item.update(
                status=section,
                candidate=entry.get("candidate"),
                confidence=entry.get("confidence"),
                reason=entry.get("why_not_verified") or entry.get("finding") or item["reason"],
                sources=[source["url"] for source in entry.get("sources", [])],
            )
            continue
        if detection.candidates or item["inactive_suggestions"]:
            item["status"] = "needs_owner_review"
            item["candidate"] = " | ".join(detection.candidates) or " | ".join(item["inactive_suggestions"])
            item["reason"] = detection.reason if detection.candidates else "inactive_dictionary_suggestion"
        elif _is_merchant_line(operator, form):
            item["status"] = "merchant_or_individual"
    items.sort(key=lambda entry: (STATUSES.index(entry["status"]), -entry["without_application"], entry["operator"]))
    return items


def review_summary(items: Sequence[Dict[str, Any]]) -> Dict[str, Dict[str, int]]:
    """Operator lines and receipts per status: counts only."""
    summary: Dict[str, Dict[str, int]] = {status: {"operators": 0, "receipts": 0} for status in STATUSES}
    for item in items:
        bucket = summary[item["status"]]
        bucket["operators"] += 1
        bucket["receipts"] += item["without_application"]
    return summary


def review_lines(items: Sequence[Dict[str, Any]], *, names: bool = False, top: int = 0) -> List[str]:
    """The review as text. Merchant lines are printed only when ``names`` asks for them."""
    out = ["operators without an application, by review status (operator lines / receipts):"]
    summary = review_summary(items)
    for status in STATUSES:
        out.append(f"  {status:26s} {summary[status]['operators']:5d} {summary[status]['receipts']:6d}")
    shown = 0
    for item in items:
        if item["status"] in ("merchant_or_individual", "no_operator_or_cash") and not names:
            continue
        if not item["shareable"] and not names:
            continue
        if top and shown >= top:
            break
        shown += 1
        candidate = f" -> {item['candidate']}" if item["candidate"] else ""
        confidence = f" ({item['confidence']})" if item["confidence"] else ""
        out.append(f"  {item['without_application']:5d}  {item['status']:24s} {item['operator']!r}{candidate}{confidence}")
    return out
