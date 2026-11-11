"""Import bookkeeping annotations from the client's Excel workbook.

The client keeps a parallel ledger by hand. Each sheet is one source (a card or
an SMS feed), the header sits on row 2 and data starts on row 4. To the right of
the parsed columns they add four hand-typed ones: a free-text note, an
accountable person, a bookkeeping month and a chart-of-accounts article.

This module reads those four back onto our transactions. Two rules shape the
whole design:

* **It never creates a transaction.** A workbook row we cannot match is
  reported, not invented. Creating rows here would fabricate source identities
  (`uq_transactions_source_msg`), poison fingerprints and break reconciliation.
* **It never silently overwrites a human.** Field-level provenance decides, and
  a clash is surfaced as a conflict for a person to resolve.

Matching cannot use `compute_fingerprint_v2`: the workbook truncates the
operator (to 20 characters on the card sheet), so the operator string in the
file is a prefix of ours, not an equal. Matching therefore keys on the v1 triple
-- amount, minute, card -- and uses the operator only to break ties.
"""
from __future__ import annotations

import hashlib
import logging
import re
import unicodedata
from dataclasses import dataclass, field as dataclass_field
from datetime import date, datetime, timedelta
from decimal import Decimal, InvalidOperation
from typing import Any, Dict, List, Optional, Sequence, Tuple

from sqlalchemy.orm import Session

from database.models import Transaction
from parsers.operator_mapper import OperatorMapper
from services import annotation_service

logger = logging.getLogger(__name__)

HEADER_ROW = 2
FIRST_DATA_ROW = 4

# Header labels of the assembled block, matched case-insensitively on a prefix
# so small edits ("Сумма операции" vs "Сумма") keep working.
_HEADER_ALIASES = {
    "datetime": ("дата_время", "дата и время"),
    "operator": ("оператор/продавец", "оператор"),
    "amount": ("сумма операции", "сумма"),
    "card": ("пк",),
    "details": ("подробности",),
}

# Tolerance when the workbook's clock and ours disagree by rounding.
_TIME_TOLERANCE = timedelta(minutes=5)


@dataclass
class WorkbookRow:
    """One annotated row read out of the workbook."""
    sheet: str
    row_number: int
    transaction_date: Optional[datetime]
    operator: Optional[str]
    amount: Optional[Decimal]
    card_last4: Optional[str]
    details: Optional[str]
    person: Optional[str]
    period: Optional[date]
    article: Optional[str]

    def has_annotation(self) -> bool:
        return any((self.details, self.person, self.period, self.article))

    def patch(self) -> Dict[str, Any]:
        return {
            "details": self.details,
            "person": self.person,
            "period": self.period,
            "article": self.article,
        }

    def row_hash(self, transaction_id: int) -> str:
        parts = [
            str(transaction_id),
            self.details or "",
            self.person or "",
            self.period.isoformat() if self.period else "",
            self.article or "",
        ]
        return hashlib.sha256("|".join(parts).encode("utf-8")).hexdigest()

    def describe(self) -> Dict[str, Any]:
        return {
            "sheet": self.sheet,
            "row": self.row_number,
            "date": self.transaction_date.isoformat() if self.transaction_date else None,
            "operator": self.operator,
            "amount": str(self.amount) if self.amount is not None else None,
            "card": self.card_last4,
            "details": self.details,
            "person": self.person,
            "period": self.period.isoformat() if self.period else None,
            "article": self.article,
        }


@dataclass
class ImportReport:
    row_count: int = 0
    matched: int = 0
    applied: int = 0
    skipped_same: int = 0
    conflicts: List[Dict[str, Any]] = dataclass_field(default_factory=list)
    ambiguous: List[Dict[str, Any]] = dataclass_field(default_factory=list)
    unmatched: List[Dict[str, Any]] = dataclass_field(default_factory=list)
    forbidden: List[Dict[str, Any]] = dataclass_field(default_factory=list)
    # Codes the workbook uses that the dictionaries did not know. They are
    # registered as `is_auto` so an admin can label them, and reported here so
    # they stop disappearing silently.
    unknown_codes: List[Dict[str, Any]] = dataclass_field(default_factory=list)
    sheets: List[str] = dataclass_field(default_factory=list)

    def as_dict(self, *, sample: int = 200) -> Dict[str, Any]:
        return {
            "row_count": self.row_count,
            "matched": self.matched,
            "applied": self.applied,
            "skipped_same": self.skipped_same,
            "counts": {
                "conflicts": len(self.conflicts),
                "ambiguous": len(self.ambiguous),
                "unmatched": len(self.unmatched),
                "forbidden": len(self.forbidden),
                "unknown_codes": len(self.unknown_codes),
            },
            "sheets": self.sheets,
            "conflicts": self.conflicts[:sample],
            "ambiguous": self.ambiguous[:sample],
            "unmatched": self.unmatched[:sample],
            "forbidden": self.forbidden[:sample],
            "unknown_codes": self.unknown_codes,
        }


# --------------------------------------------------------------------------
# Parsing
# --------------------------------------------------------------------------

def _clean_header(value: Any) -> str:
    if value is None:
        return ""
    return unicodedata.normalize("NFKC", str(value)).strip().lower()


def _locate_columns(header_values: Sequence[Any]) -> Optional[Dict[str, int]]:
    """Map logical names to 0-based column indices using the header row.

    The three annotation columns after "Подробности" carry no header of their
    own in the client's file, so they are taken positionally -- verified against
    both sheets of the reference workbook.
    """
    headers: Dict[int, str] = {}
    for col, raw in enumerate(header_values):
        text = _clean_header(raw)
        if text:
            headers[col] = text

    details_col = next(
        (col for col, text in headers.items() if text.startswith("подробности")),
        None,
    )
    if details_col is None:
        return None

    found: Dict[str, int] = {
        "details": details_col,
        "person": details_col + 1,
        "period": details_col + 2,
        "article": details_col + 3,
    }

    # The assembled block lives to the left of "Подробности"; the raw import
    # block on the far left repeats some labels, so search right-to-left.
    for name in ("datetime", "operator", "amount", "card"):
        aliases = _HEADER_ALIASES[name]
        match = None
        for col in sorted((c for c in headers if c < details_col), reverse=True):
            text = headers[col]
            if any(text.startswith(alias) for alias in aliases):
                match = col
                break
        if match is not None:
            found[name] = match

    if "datetime" not in found:
        return None
    return found


_AMOUNT_ALLOWED = re.compile(r"[^0-9.,\s-]")
_DECIMAL_TAIL = re.compile(r"[.,](\d{2})$")


def parse_amount(value: Any) -> Optional[Decimal]:
    """Parse an amount cell, tolerating both decimal conventions in the file.

    The workbook mixes '➖ 40.000,00 UZS' (dot thousands) and '➖ 5 000.00 UZS'
    (dot decimal) across sheets, so the separator is decided by what trails the
    number rather than assumed.
    """
    if value is None or value == "":
        return None
    if isinstance(value, (int, float, Decimal)):
        try:
            return Decimal(str(value))
        except InvalidOperation:
            return None

    text = _AMOUNT_ALLOWED.sub("", str(value)).strip()
    if not text:
        return None

    tail = _DECIMAL_TAIL.search(text)
    if tail:
        head = text[: tail.start()]
        head = re.sub(r"[^\d-]", "", head)
        text = f"{head}.{tail.group(1)}"
    else:
        text = re.sub(r"[^\d-]", "", text)

    if not text or text in {"-", "."}:
        return None
    try:
        return Decimal(text)
    except InvalidOperation:
        return None


def parse_datetime(value: Any) -> Optional[datetime]:
    """Coerce an Excel cell into a naive local datetime.

    `transaction_date` is stored naive local, so the workbook's wall clock is
    taken at face value -- no timezone conversion is applied anywhere.
    """
    if value is None or value == "":
        return None
    if isinstance(value, datetime):
        return value.replace(tzinfo=None)
    if isinstance(value, date):
        return datetime(value.year, value.month, value.day)
    if isinstance(value, (int, float)):
        # Excel serial date (1900 date system, with the classic leap-year bug).
        try:
            return datetime(1899, 12, 30) + timedelta(days=float(value))
        except (OverflowError, ValueError):
            return None
    text = str(value).strip()
    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%d.%m.%Y %H:%M:%S",
                "%d.%m.%Y %H:%M", "%d.%m.%Y", "%Y-%m-%d"):
        try:
            return datetime.strptime(text, fmt)
        except ValueError:
            continue
    return None


def parse_card_last4(value: Any) -> Optional[str]:
    if value is None or value == "":
        return None
    digits = "".join(ch for ch in str(value) if ch.isdigit())
    return digits[-4:].zfill(4) if digits else None


def parse_article(value: Any) -> Optional[str]:
    """Take the code out of a 'р1=ФЗП,ФМП,МП' cell and canonicalize it."""
    if value is None or value == "":
        return None
    text = str(value).strip()
    if not text or text == "0":
        return None
    code = text.split("=", 1)[0].strip()
    return annotation_service.normalize_code(code) or None


def _clean_text(value: Any, *, limit: int = annotation_service.DETAILS_MAX_LENGTH) -> Optional[str]:
    if value is None or value == "":
        return None
    text = str(value).strip()
    if not text or text == "0":
        return None
    return text[:limit]


def parse_workbook(content: bytes) -> Tuple[List[WorkbookRow], List[str]]:
    """Read every annotated row out of the workbook.

    Streams each sheet in a single pass: the client's sheets carry tens of
    thousands of template rows, and random cell access on a read-only sheet
    rescans for every lookup.

    Returns the rows plus the names of sheets that were read.
    """
    import openpyxl  # imported lazily: only the import path needs it

    from io import BytesIO

    workbook = openpyxl.load_workbook(BytesIO(content), data_only=True, read_only=True)
    rows: List[WorkbookRow] = []
    sheets: List[str] = []
    try:
        for sheet in workbook.worksheets:
            columns: Optional[Dict[str, int]] = None

            for row_number, values in enumerate(
                sheet.iter_rows(values_only=True), start=1
            ):
                if row_number == HEADER_ROW:
                    columns = _locate_columns(values)
                    if not columns:
                        logger.info(
                            "annotation import: sheet %r has no annotation block", sheet.title
                        )
                        break
                    sheets.append(sheet.title)
                    continue
                if row_number < FIRST_DATA_ROW or columns is None:
                    continue

                def cell(name: str) -> Any:
                    col = columns.get(name)
                    if col is None or col >= len(values):
                        return None
                    return values[col]

                period_raw = cell("period")
                parsed = WorkbookRow(
                    sheet=sheet.title,
                    row_number=row_number,
                    transaction_date=parse_datetime(cell("datetime")),
                    operator=_clean_text(cell("operator"), limit=500),
                    amount=parse_amount(cell("amount")),
                    card_last4=parse_card_last4(cell("card")),
                    details=_clean_text(cell("details")),
                    person=annotation_service.normalize_code(
                        _clean_text(cell("person"), limit=32) or ""
                    ) or None,
                    period=annotation_service.coerce_period(period_raw) if period_raw else None,
                    article=parse_article(cell("article")),
                )
                # Template rows below the data carry zeros and no date.
                if parsed.transaction_date is None or not parsed.has_annotation():
                    continue
                rows.append(parsed)
    finally:
        workbook.close()

    return rows, sheets


# --------------------------------------------------------------------------
# Matching
# --------------------------------------------------------------------------

def _amount_key(amount: Optional[Decimal]) -> Optional[int]:
    """Absolute amount in minor units -- debits are stored negative for us."""
    if amount is None:
        return None
    return int((abs(amount) * 100).to_integral_value())


def _candidate_window(rows: Sequence[WorkbookRow]) -> Tuple[Optional[datetime], Optional[datetime]]:
    dates = [row.transaction_date for row in rows if row.transaction_date]
    if not dates:
        return None, None
    return min(dates) - timedelta(days=1), max(dates) + timedelta(days=1)


class _CandidateIndex:
    """In-memory index of our transactions over the workbook's date window.

    One query beats a lookup per row: the workbook covers months, but that is
    still only thousands of rows.
    """

    def __init__(self, transactions: Sequence[Transaction]):
        self.by_minute_card: Dict[Tuple[str, int, str], List[Transaction]] = {}
        self.by_day_card: Dict[Tuple[str, int, str], List[Transaction]] = {}
        self.by_day: Dict[Tuple[str, int], List[Transaction]] = {}

        for txn in transactions:
            when = txn.transaction_date
            amount = _amount_key(txn.amount)
            if when is None or amount is None:
                continue
            card = (txn.card_last_4 or "").strip() or "----"
            day = when.strftime("%Y-%m-%d")
            minute = when.strftime("%Y-%m-%d %H:%M")
            self.by_minute_card.setdefault((minute, amount, card), []).append(txn)
            self.by_day_card.setdefault((day, amount, card), []).append(txn)
            self.by_day.setdefault((day, amount), []).append(txn)

    def lookup(self, row: WorkbookRow) -> Tuple[List[Transaction], str]:
        when = row.transaction_date
        amount = _amount_key(row.amount)
        if when is None or amount is None:
            return [], "no_key"

        card = (row.card_last4 or "").strip() or "----"
        day = when.strftime("%Y-%m-%d")
        minute = when.strftime("%Y-%m-%d %H:%M")

        exact = self.by_minute_card.get((minute, amount, card), [])
        if exact:
            return exact, "minute_card"

        same_day_card = self.by_day_card.get((day, amount, card), [])
        near = [
            txn for txn in same_day_card
            if txn.transaction_date and abs(txn.transaction_date - when) <= _TIME_TOLERANCE
        ]
        if near:
            return near, "day_card_near"
        if same_day_card:
            return same_day_card, "day_card"

        return self.by_day.get((day, amount), []), "day_amount"


def _disambiguate(row: WorkbookRow, candidates: List[Transaction]) -> List[Transaction]:
    """Narrow several candidates using the operator, honouring truncation.

    The workbook truncates the operator, so ours must *start with* theirs once
    both are normalized -- an equality test would reject every card-sheet row.
    """
    if len(candidates) <= 1 or not row.operator:
        return candidates

    needle = OperatorMapper.normalize_operator(row.operator)
    if not needle:
        return candidates

    prefixed = [
        txn for txn in candidates
        if OperatorMapper.normalize_operator(txn.operator_raw or "").startswith(needle)
    ]
    if len(prefixed) == 1:
        return prefixed
    return prefixed or candidates


def match_rows(
    db: Session,
    rows: Sequence[WorkbookRow],
) -> Tuple[List[Tuple[WorkbookRow, Transaction]], List[WorkbookRow], List[Tuple[WorkbookRow, List[Transaction]]]]:
    """Resolve workbook rows to transactions.

    Returns `(matched, unmatched, ambiguous)`; ambiguity is reported, never
    guessed at.
    """
    start, end = _candidate_window(rows)
    if start is None:
        return [], list(rows), []

    transactions = (
        db.query(Transaction)
        .filter(Transaction.transaction_date >= start)
        .filter(Transaction.transaction_date <= end)
        .all()
    )
    index = _CandidateIndex(transactions)

    matched: List[Tuple[WorkbookRow, Transaction]] = []
    unmatched: List[WorkbookRow] = []
    ambiguous: List[Tuple[WorkbookRow, List[Transaction]]] = []

    for row in rows:
        candidates, _stage = index.lookup(row)
        if not candidates:
            unmatched.append(row)
            continue
        candidates = _disambiguate(row, candidates)
        if len(candidates) == 1:
            matched.append((row, candidates[0]))
        else:
            ambiguous.append((row, candidates))

    return matched, unmatched, ambiguous


# --------------------------------------------------------------------------
# Applying
# --------------------------------------------------------------------------

def file_digest(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def run_import(
    db: Session,
    content: bytes,
    *,
    dry_run: bool = True,
    overwrite_manual: bool = False,
    actor_user_id: Optional[int] = None,
    import_batch_id: Optional[Any] = None,
    authorize: Optional[Any] = None,
) -> ImportReport:
    """Parse, match and (unless `dry_run`) apply the workbook's annotations.

    `authorize` is an optional callable `(transaction) -> None` that raises when
    the caller may not touch that row; it is checked per row so one forbidden
    receipt does not fail the whole import.
    """
    rows, sheets = parse_workbook(content)
    report = ImportReport(row_count=len(rows), sheets=sheets)

    matched, unmatched, ambiguous = match_rows(db, rows)
    report.matched = len(matched)
    report.unmatched = [row.describe() for row in unmatched]
    report.ambiguous = [
        {**row.describe(), "candidate_ids": [int(t.id) for t in candidates[:10]]}
        for row, candidates in ambiguous
    ]

    reference_codes = annotation_service.load_reference_codes(db)

    # Register codes the workbook uses that the dictionaries have never seen,
    # before applying anything. Otherwise they are written onto transactions and
    # then match nothing: the grid renders a bare code and no report mentions it.
    unknown_articles = {
        code for code in (
            annotation_service.normalize_code(row.article) for row, _ in matched
        ) if code and code not in reference_codes["article"]
    }
    unknown_persons = {
        code for code in (
            annotation_service.normalize_code(row.person) for row, _ in matched
        ) if code and code not in reference_codes["person"]
    }
    if unknown_articles or unknown_persons:
        report.unknown_codes = annotation_service.register_auto_codes(
            db, articles=unknown_articles, persons=unknown_persons,
        )
        reference_codes = annotation_service.load_reference_codes(db)

    for row, txn in matched:
        if authorize is not None:
            try:
                authorize(txn)
            except Exception as exc:  # noqa: BLE001 - reported, not raised
                report.forbidden.append({**row.describe(), "transaction_id": int(txn.id), "reason": str(exc)})
                continue

        digest = row.row_hash(int(txn.id))
        if txn.annotation_import_hash == digest:
            # Nothing changed since the last import of this row. Skipping keeps
            # updated_at untouched, so mirrors are not forced to re-sync.
            report.skipped_same += 1
            continue

        result = annotation_service.apply_annotation_patch(
            db,
            txn,
            row.patch(),
            source="import",
            actor_user_id=actor_user_id,
            import_batch_id=import_batch_id,
            overwrite_manual=overwrite_manual,
            reference_codes=reference_codes,
            validate=False,  # unknown codes are reported below, not rejected
        )

        if result["skipped"]:
            report.conflicts.append({
                **row.describe(),
                "transaction_id": int(txn.id),
                "fields": result["skipped"],
            })

        if result["changed"]:
            report.applied += 1
            txn.annotation_import_hash = digest

    if dry_run:
        db.rollback()
    return report
