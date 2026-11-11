"""
The customer's own workbook, reproduced from their file rather than from notes about it.

The customer keeps their receipts in `Книга1.xlsx`: one sheet per bank, a hidden
group of import columns on the left, an assembled block from column O, a frozen
header, zeros hidden, an 8pt Calibri grid with hair rules between rows. They
judge our export by opening it next to that file, and a layout that is merely
similar reads as wrong. Twice the layout was transcribed into code and twice it
drifted: openpyxl reports a hidden group of four columns under its first letter
only, so three of them came out visible and made a blank band in the middle of
the sheet; the view that hides zeros was never carried over, so every empty
`Приложение` cell read `0`.

So this module does not describe their layout, it opens it. A pruned copy of
their workbook — the rows above the data plus two data rows per sheet, made by
`tools/build_excel_client_template.py` — is the template. Column dimensions,
sheet views, merged cells, conditional formats and the theme are copied from it
wholesale; the header rows are copied cell for cell with the counts filled in;
every data row takes its styles from the template's data rows. What remains in
code is what cannot come from a file: which column carries which value, and
how a receipt's fields are read.

Things the file taught us that are easy to lose:

* **Column letters matter.** Their assembled block starts at column O because
  columns A–N are the hidden import group, and the rest of their tooling
  addresses `Z` as the amount and `AG` as the balance. The export keeps the
  letters, and fills the import group with the receipt text the way their
  formulas would have — one row per line of the message.
* **Zeros are written, not shown.** Their lookups write `0` where there is no
  application and no p2p flag; the sheet view hides zeros, so the cells read
  blank. Writing blanks instead would change what their formulas see.
* **The bank's own words.** `Операция`, the decorated amount and the balance
  are the SMS lines themselves — `🔴 Platezh`, `➖ 5 000.00 UZS`,
  `💵 4 229 872.82 UZS` — not our normalised labels.
* **`(Без дубля)` is the timestamp made unique.** Two receipts in the same
  minute get `+8.64 s` and `+17.28 s` (a ten-thousandth of a day each), which
  is how their sheet tells duplicates apart without losing either.
* **The second numbering column runs across sheets.** Their HUMO sheet opens
  at `№ 1` but `1012`, continuing where CardXabar stopped.
* **HUMO splits every amount into digit groups** in hidden helper columns
  (`Длн.`, `мрд`, `млн`, `тыс`, `ед`). They are hidden, but the file is theirs,
  so they are filled.
"""
from __future__ import annotations

import logging
import os
import unicodedata
from copy import copy, deepcopy
from dataclasses import dataclass, field
from datetime import date, datetime, timedelta
from decimal import Decimal
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

from openpyxl import Workbook, load_workbook
from openpyxl.cell.cell import ERROR_CODES, ILLEGAL_CHARACTERS_RE, Cell
from openpyxl.styles.cell_style import StyleArray
from openpyxl.utils import column_index_from_string, get_column_letter
from openpyxl.worksheet.dimensions import ColumnDimension, RowDimension

logger = logging.getLogger(__name__)

#: The pruned copy of the customer's workbook; see tools/build_excel_client_template.py.
TEMPLATE_PATH = Path(__file__).with_name("excel_client_template.xlsx")

#: Excel's hard row limit; the import group is cut rather than break the file.
EXCEL_MAX_ROWS = 1_048_576

#: Excel's per-cell text limit. A longer string makes the workbook unopenable.
MAX_CELL_TEXT = 32_767

#: A tenth of a millisecond short of nothing in Excel's day units: the step their
#: `(Без дубля)` column adds per repeated timestamp.
DEDUP_STEP = timedelta(days=0.0001)

#: Lines of one receipt carried into the hidden import group. Bank notices are
#: six lines; an OCR dump is not what that group is for.
IMPORT_LINES_PER_MESSAGE = 12


@dataclass(frozen=True)
class SheetTemplate:
    """One sheet of the customer's workbook and where our values go in it."""

    #: Template sheet name; also the source whose sheet this is.
    sheet: str
    #: The source's name as their captions spell it (`Сборка HumoCard`).
    token: str
    #: Rows copied verbatim above the data.
    header_rows: int
    #: Template row whose styles the first data row takes.
    first_row: int
    #: Template row whose styles every other data row takes.
    body_row: int
    #: Column letter -> key in the row dict.
    columns: Dict[str, str]
    #: Cell -> figure written into the header rows.
    figures: Dict[str, str] = field(default_factory=dict)
    #: Header cells whose text names the source.
    captions: Tuple[str, ...] = ()
    #: What their `G`..`L` import columns hold, in this sheet's own order.
    import_order: Tuple[str, ...] = (
        "headline", "operator", "amount", "card", "when", "balance",
    )

    @property
    def wants_digit_groups(self) -> bool:
        return "amount_digits" in self.columns.values()


#: Their hidden import group, identical on both sheets: the message text line
#: by line, with the fields their formulas pull out of each header line.
IMPORT_COLUMNS: Dict[str, str] = {
    "A": "line",
    "B": "message_no",
    "C": "header_no",
    "D": "service",
    "E": "date",
    "F": "time",
    "G": "line_1",
    "H": "line_2",
    "I": "line_3",
    "J": "line_4",
    "K": "line_5",
    "L": "line_6",
    "M": "weekday",
}
IMPORT_LAST_COLUMN = column_index_from_string("N")

_CARDXABAR = SheetTemplate(
    sheet="CardXabar",
    token="CardXabar",
    header_rows=4,      # title, header, counts, and their row of zeros
    first_row=5,
    body_row=6,
    columns={
        "O": "row_number",
        "P": "operation_number",
        "Q": "date_time",
        "R": "date_time_dedup",
        "S": "source",
        "T": "operation",
        "U": "operator",
        "W": "application",
        "X": "sign",
        "Y": "amount_text",
        "Z": "amount_value",
        "AF": "balance_text",
        "AG": "balance_value",
        "AH": "card",
        "AI": "p2p",
        "AJ": "p2p_confirmed",
        "AK": "details",
        "AL": "person",
        "AM": "period",
        "AN": "article",
    },
    figures={
        "B1": "count",
        "P3": "count",
        "T3": "last_operation_at",
        "Z3": "total_amount",
        "AU3": "count",
    },
    captions=("O1", "Q3", "U3"),
)

_HUMO = SheetTemplate(
    sheet="HUMO Card",
    token="HumoCard",
    header_rows=3,      # title, header, counts
    first_row=4,
    body_row=5,
    columns={
        "O": "row_number",
        "P": "operation_number",
        "Q": "date_time",
        "R": "date_time_dedup",
        "S": "source",
        "T": "operation",
        "U": "operator",
        "W": "application",
        "X": "sign",
        "Y": "amount_text",
        "Z": "amount_digits",
        "AA": "amount_length",
        "AB": "amount_billions",
        "AC": "amount_millions",
        "AD": "amount_thousands",
        "AE": "amount_units",
        "AF": "amount_value",
        "AG": "balance_text",
        "AH": "balance_digits",
        "AI": "balance_length",
        "AJ": "balance_billions",
        "AK": "balance_millions",
        "AL": "balance_thousands",
        "AM": "balance_units",
        "AN": "balance_value",
        "AO": "card",
        "AP": "p2p",
        "AQ": "p2p_confirmed",
        "AR": "details",
        "AS": "person",
        "AT": "period",
        "AU": "article",
    },
    figures={
        "B1": "count",
        "P3": "count",
    },
    captions=("O1", "Q3"),
    # Their HUMO sheet heads these columns Операция / Сумма / Оператор, where
    # CardXabar heads them Операция / Оператор / Сумма.
    import_order=("headline", "amount", "operator", "card", "when", "balance"),
)

#: In the order their workbook has them.
SHEET_TEMPLATES: Dict[str, SheetTemplate] = {
    _CARDXABAR.sheet: _CARDXABAR,
    _HUMO.sheet: _HUMO,
}

#: The sheet the workbook opens on: every receipt of the export, in the
#: CardXabar layout, before the one-sheet-per-bank split their file has.
COMBINED_SHEET_NAME = "Все транзакции"
#: The title strip is three narrow columns wide; `Сборка: все транзакции` is cut
#: off at the column-number strip, the bare name is not.
COMBINED_CAPTIONS: Dict[str, str] = {
    "O1": "Все транзакции",
    "Q3": "- число операций, все сервисы",
    "U3": "- дата крайней операции",
}


def block_columns(template: SheetTemplate = _CARDXABAR) -> List[Tuple[str, Optional[str]]]:
    """
    Their assembled block, column by column, in their order.

    The live sheet feeds on this: its rows land at the same letters as the
    export's, spacers included, so `Z` is still Сумма and `AG` still Остаток for
    anything the customer has pointed a formula at.
    """
    letters = [column_index_from_string(letter) for letter in template.columns]
    return [
        (get_column_letter(index), template.columns.get(get_column_letter(index)))
        for index in range(min(letters), max(letters) + 1)
    ]


def template_for_source(source_display: str) -> SheetTemplate:
    """HUMO gets its own sheet; every other bank is laid out the CardXabar way."""
    if "HUMO" in (source_display or "").upper():
        return _HUMO
    return _CARDXABAR


def is_native_source(template: SheetTemplate, source_display: str) -> bool:
    """Whether this is the very source the template sheet was built for."""
    return _normalise(source_display) == _normalise(template.sheet)


def _normalise(text: str) -> str:
    return "".join((text or "").split()).lower()


def order_sources(sources: Iterable[str]) -> List[str]:
    """Their sheet order: the banks their workbook already has, then the rest as seen."""
    seen = list(dict.fromkeys(sources))
    ordered: List[str] = []
    for template in SHEET_TEMPLATES.values():
        for source in seen:
            if source not in ordered and is_native_source(template, source):
                ordered.append(source)
    ordered.extend(source for source in seen if source not in ordered)
    return ordered


# --- Reading the bank's own words --------------------------------------------
# Their Операция, Сумма and Остаток cells are the SMS lines themselves. Taking
# them verbatim is both more faithful and less to maintain than modelling each
# bank's emoji vocabulary and decimal separators — CardXabar writes
# `🔴 Platezh` / `💵 7 574 438.62 UZS`, HUMO writes `🏧 Снятие наличных` /
# `💰 54.989,00 UZS`, and neither needs a rule here.

_AMOUNT_MARKERS = ("➖", "➕")
_BALANCE_MARKERS = ("💵", "💰")
_OPERATOR_MARKERS = ("📍", "🏪")
_CARD_MARKERS = ("💳",)
_CLOCK_MARKERS = ("🕓", "🕘", "🕒", "📅")
#: Every marker that names a field rather than the operation.
_FIELD_MARKERS = (
    _AMOUNT_MARKERS + _BALANCE_MARKERS + _OPERATOR_MARKERS + _CARD_MARKERS + _CLOCK_MARKERS
)
_MAX_OPERATION_LEN = 60


def _lines(raw_message: str) -> List[str]:
    return [line.strip() for line in (raw_message or "").splitlines() if line.strip()]


#: `ℹ️` is U+2139 plus this selector. On its own U+2139 is a *letter* to
#: `unicodedata` (category Ll, it lives among the letterlike symbols), so a
#: category test alone threw away the balance-change notices — 11 of them on
#: production — and put our own wording in the customer's Операция column.
_EMOJI_PRESENTATION = "️"


def operation_from_raw(raw_message: str) -> Optional[str]:
    """
    The first line of the message, when it is one of the bank's headline lines.

    A headline opens with a symbol (`🔴`, `💸`) or with a character the bank asked
    to be drawn as one (`ℹ️`). A receipt that opens with `[Photo]`, `Перевод
    через…` or an OCR dump does not, and falls back to the label we derived.
    """
    lines = _lines(raw_message)
    if not lines:
        return None
    first = lines[0]
    if len(first) > _MAX_OPERATION_LEN:
        return None
    if not unicodedata.category(first[0]).startswith("S") and first[1:2] != _EMOJI_PRESENTATION:
        return None
    if first.startswith(_FIELD_MARKERS):
        # `📍 PLUM OPLATA, UZ` opens with a symbol too. A receipt that arrives
        # without its headline must not have its merchant read as the operation.
        return None
    return first


def _line_starting_with(raw_message: str, markers: tuple) -> Optional[str]:
    for line in _lines(raw_message):
        if line.startswith(markers):
            return line
    return None


def amount_text_from_raw(raw_message: str) -> Optional[str]:
    """`➖ 5 000.00 UZS`, exactly as the bank sent it."""
    return _line_starting_with(raw_message, _AMOUNT_MARKERS)


def balance_text_from_raw(raw_message: str) -> Optional[str]:
    """`💵 4 229 872.82 UZS` — or `💰 …` on the banks that write it that way."""
    return _line_starting_with(raw_message, _BALANCE_MARKERS)


# --- Per-source presentation, for the receipts that carry no headline ---------
_TYPE_EMOJI = {
    "DEBIT": "💸",
    "CREDIT": "🎉",
    "CONVERSION": "💱",
    "REVERSAL": "↩️",
}
_WITHDRAWAL_HINTS = ("снятие", "обнал", "atm", "банкомат", "bxo")
#: A balance-change notice is not a payment; their sheet marks it `ℹ️`.
_NOTICE_HINTS = ("изменен", "изменение", "счет по карте")


class SourceStyle:
    """How one source's decorated cells are written when we have to write them."""

    __slots__ = ("uniform_emoji", "thousands", "decimal", "balance_emoji")

    def __init__(
        self,
        uniform_emoji: Optional[str],
        thousands: str,
        decimal: str,
        balance_emoji: str = "💵",
    ) -> None:
        self.uniform_emoji = uniform_emoji
        self.thousands = thousands
        self.decimal = decimal
        self.balance_emoji = balance_emoji


# CardXabar tags every operation with the same red dot and uses `1 000.00`;
# HUMO Card uses a per-type emoji, the European `1.000,00` and `💰` for balance.
_STYLE_CARDXABAR = SourceStyle(uniform_emoji="🔴", thousands=" ", decimal=".")
_STYLE_HUMO = SourceStyle(uniform_emoji=None, thousands=".", decimal=",", balance_emoji="💰")
_STYLE_DEFAULT = _STYLE_CARDXABAR


def style_for_source(source_display: str) -> SourceStyle:
    haystack = (source_display or "").upper()
    if "HUMO" in haystack:
        return _STYLE_HUMO
    return _STYLE_DEFAULT


def _group(amount: Decimal, style: SourceStyle) -> str:
    whole, _, frac = f"{amount:.2f}".partition(".")
    grouped = ""
    for offset, digit in enumerate(reversed(whole)):
        if offset and offset % 3 == 0:
            grouped = style.thousands + grouped
        grouped = digit + grouped
    return f"{grouped}{style.decimal}{frac}"


def format_money(value: Optional[Decimal], currency: str, style: SourceStyle) -> str:
    """`➖ 5 000.00 UZS` — their decorated amount, in the source's own convention."""
    if value is None:
        return ""
    try:
        amount = Decimal(str(value))
    except Exception:  # noqa: BLE001
        return ""
    sign = "➖" if amount < 0 else "➕"
    return f"{sign} {_group(abs(amount), style)} {currency or 'UZS'}"


def format_balance(value: Optional[Decimal], currency: str, style: SourceStyle) -> str:
    """`💵 4 229 872.82 UZS` — the balance is never signed in their sheet."""
    if value is None:
        return ""
    try:
        amount = abs(Decimal(str(value)))
    except Exception:  # noqa: BLE001
        return ""
    return f"{style.balance_emoji} {_group(amount, style)} {currency or 'UZS'}"


def operation_label(
    transaction_type: str,
    display_label: str,
    haystack: str,
    style: SourceStyle,
) -> str:
    """The Операция cell for receipts that arrived without a headline line."""
    if not display_label:
        return ""
    if style.uniform_emoji:
        return f"{style.uniform_emoji} {display_label}"
    label_lowered = display_label.lower()
    if any(hint in label_lowered for hint in _NOTICE_HINTS):
        return f"ℹ️ {display_label}"
    lowered = (haystack or "").lower()
    if transaction_type == "DEBIT" and any(hint in lowered for hint in _WITHDRAWAL_HINTS):
        return f"🏧 {display_label}"
    return f"{_TYPE_EMOJI.get(transaction_type, 'ℹ️')} {display_label}"


def operation_text(
    *,
    source_display: str,
    transaction_type: str,
    type_display: str,
    operator_raw: str,
    raw_message: str,
) -> str:
    """The Операция cell: the bank's own headline first; our label only when there isn't one."""
    return operation_from_raw(raw_message) or operation_label(
        transaction_type, type_display, f"{operator_raw} {raw_message}", style_for_source(source_display),
    )


def direction_sign(transaction_type: str) -> str:
    """Their sign column: `-` for money out, `+` for money in."""
    if transaction_type in {"CREDIT", "REVERSAL"}:
        return "+"
    if transaction_type in {"DEBIT", "CONVERSION"}:
        return "-"
    return ""


def whole_amount(value: Optional[Decimal]) -> Optional[int]:
    """
    Their number columns hold an unsigned whole number: `4 229 872.82` is `4229872`.

    The sign is carried by the column beside it, which is why their own total is a
    sum of magnitudes rather than a net.
    """
    if value is None:
        return None
    try:
        return int(abs(Decimal(str(value))))
    except Exception:  # noqa: BLE001
        return None


def digit_groups(value: Optional[int], prefix: str, *, leading_space: bool) -> Dict[str, Any]:
    """
    HUMO's hidden helpers: `40000` becomes ` 40.000`, its length, and the
    thousand groups spread over `мрд`/`млн`/`тыс`/`ед`.

    Their formula leaves the leading group a number and the rest text, so a
    `000` group keeps its zeros; a missing balance is a row of zeros.
    """
    names = ("units", "thousands", "millions", "billions")
    if not value:
        out = {f"{prefix}_digits": 0, f"{prefix}_length": 0}
        out.update({f"{prefix}_{name}": 0 for name in names})
        return out
    grouped = f"{int(value):,}".replace(",", ".")
    text = (" " if leading_space else "") + grouped
    groups = grouped.split(".")
    out: Dict[str, Any] = {f"{prefix}_digits": text, f"{prefix}_length": len(text)}
    out.update({f"{prefix}_{name}": 0 for name in names})
    for position, group in enumerate(reversed(groups)):
        if position >= len(names):
            # Their sheet stops at мрд. A number past a trillion is a bad parse,
            # and one of them must not abort the whole export.
            logger.warning("Digit groups beyond мрд in %s: %s", prefix, value)
            break
        leading = position == len(groups) - 1
        out[f"{prefix}_{names[position]}"] = int(group) if leading else group
    return out


# --- Sheet naming ------------------------------------------------------------
_FORBIDDEN_SHEET_CHARS = set(':\\/?*[]')


def sanitize_sheet_name(raw: str, taken: set) -> str:
    """
    Excel sheet names: at most 31 chars, none of `: \\ / ? * [ ]`, unique.

    Russian chat titles run long, so truncation alone collides — and a duplicate
    name raises at save time, in front of the user. The counter suffix prevents it.
    """
    cleaned = "".join(" " if ch in _FORBIDDEN_SHEET_CHARS else ch for ch in (raw or ""))
    name = " ".join(cleaned.split()) or "Лист"
    name = name[:31]
    # Excel's other sheet-name rule, which openpyxl does not enforce: a name may
    # not begin or end with an apostrophe. A chat titled `'CardXabar'` produced a
    # workbook Excel refused to open at all, with no error from us.
    name = name.strip("'") or "Лист"

    if name not in taken:
        taken.add(name)
        return name
    for counter in range(2, 1000):
        suffix = f" ({counter})"
        candidate = name[: 31 - len(suffix)] + suffix
        if candidate not in taken:
            taken.add(candidate)
            return candidate
    fallback = f"{name[:26]}{len(taken):04d}"
    taken.add(fallback)
    return fallback


# --- One receipt as a row ----------------------------------------------------
def build_row_values(
    *,
    transaction_date: Optional[datetime],
    source_display: str,
    transaction_type: str,
    type_display: str,
    operator_raw: str,
    application_mapped: str,
    amount: Optional[Decimal],
    balance_after: Optional[Decimal],
    currency: str,
    card_last_4: str,
    is_p2p: bool,
    annotation_details: str,
    annotation_person_code: str,
    annotation_period: Optional[date],
    annotation_article_code: str,
    article_labels: Optional[Dict[str, str]] = None,
    raw_message: str = "",
) -> Dict[str, Any]:
    """
    Render one transaction into the values of the customer's columns.

    Numbering, the de-duplicated timestamp and HUMO's digit helpers depend on the
    sheet the row lands on, so the writer adds those.
    """
    style = style_for_source(source_display)

    article = (annotation_article_code or "").strip()
    if article and article_labels:
        label = article_labels.get(article)
        if label:
            article = f"{article}={label}"

    return {
        "date_time": transaction_date,
        "source": source_display,
        "operation": operation_text(
            source_display=source_display, transaction_type=transaction_type, type_display=type_display,
            operator_raw=operator_raw, raw_message=raw_message,
        ),
        "operator": operator_raw,
        # Their lookup writes a zero rather than leaving the cell empty.
        "application": application_mapped or 0,
        "sign": direction_sign(transaction_type),
        "amount_text": amount_text_from_raw(raw_message)
        or format_money(amount, currency, style),
        "amount_value": whole_amount(amount),
        # A receipt without a balance line leaves their formulas at zero, in
        # both the decorated and the plain column.
        "balance_text": balance_text_from_raw(raw_message)
        or format_balance(balance_after, currency, style)
        or 0,
        "balance_value": whole_amount(balance_after) if balance_after is not None else 0,
        "card": card_last_4,
        "p2p": 1 if is_p2p else 0,
        "p2p_confirmed": 1 if is_p2p else None,
        "details": annotation_details,
        "person": annotation_person_code,
        "period": annotation_period,
        "article": article,
        "raw_message": raw_message or "",
    }


#: The marker each import column is filled from. The banks do not agree on line
#: order — CardXabar sends the card before the merchant, HUMO the other way, and
#: NBU varies between messages — so the column is chosen by what the line starts
#: with, the way their formulas pick it out, rather than by where it appeared.
_IMPORT_ROLE_MARKERS: Dict[str, Tuple[str, ...]] = {
    "amount": _AMOUNT_MARKERS,
    "operator": _OPERATOR_MARKERS,
    "card": _CARD_MARKERS,
    "balance": _BALANCE_MARKERS,
}


def _import_roles(
    lines: Sequence[str],
    when: Optional[datetime],
    order: Sequence[str],
) -> Dict[str, Any]:
    """
    One receipt's lines sorted into the roles their import columns name.

    A line the markers do not claim still has to go somewhere — an OCR dump or a
    photo caption has no `📍` at all — so whatever is left fills the unclaimed
    roles left to right, in the sheet's own column order, and nothing is
    silently dropped.
    """
    roles: Dict[str, Any] = {"headline": lines[0] if lines else 0, "when": when or 0}
    claimed = {0} if lines else set()
    for role, markers in _IMPORT_ROLE_MARKERS.items():
        for index, line in enumerate(lines):
            if index not in claimed and line.startswith(markers):
                roles[role] = line
                claimed.add(index)
                break

    if len(claimed) > (1 if lines else 0):
        # A marked receipt: a column with no marker of its own stays empty rather
        # than taking whatever is left. The clock line belongs to no column, so
        # filling from the leftovers would put it under `Сумма остатка`.
        for role in order:
            roles.setdefault(role, 0)
        return roles

    # Nothing was marked — an OCR dump, a photo caption, a transfer description.
    # Its lines fill the columns left to right so that nothing is dropped.
    spare = (line for index, line in enumerate(lines) if index not in claimed)
    for role in order:
        if role not in roles:
            roles[role] = next(spare, 0)
    return roles


def import_lines(
    row: Dict[str, Any],
    message_no: int,
    template: SheetTemplate = _CARDXABAR,
) -> List[Dict[str, Any]]:
    """
    What their import group holds for one receipt: the Telegram export line by
    line — the `CardXabar, [01.01.2026 7:17]` header, the message, a blank — with
    the fields their formulas pull out of it beside the header line.
    """
    when: Optional[datetime] = row.get("date_time")
    source = row.get("source") or ""
    lines = _lines(row.get("raw_message") or "")[:IMPORT_LINES_PER_MESSAGE]
    roles = _import_roles(lines, when, template.import_order)
    stamp = f"{when:%d.%m.%Y} {when.hour}:{when:%M}" if when else ""
    header: Dict[str, Any] = {
        "line": f"{source}, [{stamp}]",
        "message_no": message_no,
        "header_no": message_no,
        "service": source,
        "date": datetime(when.year, when.month, when.day) if when else 0,
        "time": when.time() if when else 0,
        "weekday": when.isoweekday() if when else 0,
    }
    for position, role in enumerate(template.import_order, start=1):
        header[f"line_{position}"] = roles.get(role, 0)
    rest = {key: 0 for key in IMPORT_COLUMNS.values()}
    rest["message_no"] = message_no
    body = [dict(rest, line=line) for line in lines]
    blank = dict(rest, line=None)
    return [header, *body, blank]


# --- Writing -----------------------------------------------------------------
class _StyleBank:
    """
    Template styles re-registered in the output workbook.

    A cell's style is an index into its own workbook's tables, so a template
    style cannot be assigned across workbooks. Each distinct template style is
    rebuilt once through a probe cell and the resulting index array shared by
    every cell that uses it — a few dozen arrays for a million cells.
    """

    def __init__(self, workbook: Workbook) -> None:
        self._probe_sheet = workbook.create_sheet("_style_probe")
        self._cache: Dict[Tuple[int, ...], StyleArray] = {}

    def transplant(self, source) -> Optional[StyleArray]:
        array = getattr(source, "_style", None)
        if array is None or not any(array):
            return None
        key = tuple(array)
        found = self._cache.get(key)
        if found is None:
            probe = Cell(self._probe_sheet, row=1, column=1)
            probe.font = copy(source.font)
            probe.fill = copy(source.fill)
            probe.border = copy(source.border)
            probe.alignment = copy(source.alignment)
            probe.protection = copy(source.protection)
            probe.number_format = source.number_format
            found = probe._style
            self._cache[key] = found
        return found

    def close(self, workbook: Workbook) -> None:
        workbook.remove(self._probe_sheet)


def _text_safe(value: Any) -> Any:
    """Raw message text as a cell value: never a formula, never an illegal character."""
    if not isinstance(value, str):
        return value
    if ILLEGAL_CHARACTERS_RE.search(value):
        value = ILLEGAL_CHARACTERS_RE.sub("", value)
    return value


def _make_cell(worksheet, value: Any, style: Optional[StyleArray]) -> Optional[Cell]:
    if value is None and style is None:
        return None
    if isinstance(value, str):
        value = _text_safe(value)
        if len(value) > MAX_CELL_TEXT:
            # Excel's per-cell limit. openpyxl enforces it through the value
            # setter, which the raw assignment below skips.
            value = value[:MAX_CELL_TEXT]
        if value.startswith("=") or value in ERROR_CODES:
            # A line of dashes, an OCR artefact or a literal `#N/A` must not
            # become a formula or an error value in the customer's sheet.
            cell = Cell(worksheet, row=1, column=1, style_array=style)
            cell._value = value
            cell.data_type = "s"
            return cell
    return Cell(worksheet, row=1, column=1, value=value, style_array=style)


def import_block_enabled() -> bool:
    return os.getenv("EXPORT_CLIENT_IMPORT_BLOCK", "1").strip().lower() not in {"0", "false", "no", "off"}


class ClientWorkbookWriter:
    """
    Writes the customer's workbook: a sheet with every receipt, then one sheet
    per source laid out by the template sheet for that bank, streamed row by row.

        writer = ClientWorkbookWriter()
        writer.add_combined_sheet(all_rows)
        writer.add_sheet("CardXabar", rows)
        writer.add_sheet("HUMO Card", rows)
        writer.save(stream)
    """

    def __init__(
        self,
        *,
        descending: bool = False,
        template_path: Path = TEMPLATE_PATH,
        with_import_block: Optional[bool] = None,
    ) -> None:
        self.template = load_workbook(template_path)
        self.workbook = Workbook(write_only=True)
        # Their theme, so the tinted fills resolve to the same colours.
        self.workbook.loaded_theme = self.template.loaded_theme
        self.descending = descending
        self.with_import_block = import_block_enabled() if with_import_block is None else with_import_block
        self._styles = _StyleBank(self.workbook)
        self._taken_names: set = set()
        self._operation_number = 0
        self._sheet_count = 0

    # -- public ---------------------------------------------------------------
    def add_combined_sheet(self, rows: Sequence[Dict[str, Any]]) -> None:
        """Every receipt on one sheet, numbered on its own; the per-bank numbering is untouched."""
        self.add_sheet("", rows, combined=True)

    def add_sheet(
        self,
        source_display: str,
        rows: Sequence[Dict[str, Any]],
        *,
        sheet_name: Optional[str] = None,
        combined: bool = False,
    ) -> None:
        template = _CARDXABAR if combined else template_for_source(source_display)
        source_ws = self.template[template.sheet]
        default_name = COMBINED_SHEET_NAME if combined else (source_display or template.sheet)
        title = sanitize_sheet_name(sheet_name or default_name, self._taken_names)
        target_ws = self.workbook.create_sheet(title)
        active = self._sheet_count == 0
        self._sheet_count += 1

        self._copy_sheet_shape(source_ws, target_ws, active=active)
        max_column = source_ws.max_column

        figures = self._figures(rows)
        captions = self._captions(source_ws, template, source_display, combined)
        for row_index in range(1, template.header_rows + 1):
            target_ws.append(self._header_row(
                source_ws, target_ws, row_index, max_column, template, figures, captions,
            ))

        first_styles = self._row_styles(source_ws, template.first_row, max_column)
        body_styles = self._row_styles(source_ws, template.body_row, max_column)
        keys = self._column_keys(template, max_column)
        import_columns = set(range(1, IMPORT_LAST_COLUMN + 1))

        # The second numbering column continues across the per-bank sheets, the
        # way their HUMO sheet opens at 1012; the combined sheet counts on its
        # own. When the export is newest-first the numbers run downward so the
        # oldest row keeps 1.
        if combined:
            start = 1
        else:
            start = self._operation_number + 1
            self._operation_number += len(rows)
        numbers = range(start, start + len(rows))
        if self.descending:
            numbers = range(start + len(rows) - 1, start - 1, -1)

        lines: Iterable[Dict[str, Any]] = ()
        if self.with_import_block:
            lines = (
                line
                for message_no, row in enumerate(rows, start=1)
                for line in import_lines(row, message_no, template)
            )
        line_iter = iter(lines)

        seen_stamps: Dict[Any, int] = {}
        row_limit = EXCEL_MAX_ROWS - template.header_rows
        written = 0
        for position, row in enumerate(rows, start=1):
            values = dict(row)
            values["row_number"] = position
            values["operation_number"] = numbers[position - 1]
            stamp = values.get("date_time")
            if stamp is not None:
                repeat = seen_stamps.get(stamp, 0)
                seen_stamps[stamp] = repeat + 1
                values["date_time_dedup"] = stamp + DEDUP_STEP * repeat if repeat else stamp
            if template.wants_digit_groups:
                values.update(digit_groups(values.get("amount_value"), "amount", leading_space=True))
                values.update(digit_groups(values.get("balance_value"), "balance", leading_space=False))
            line = next(line_iter, None)
            if line:
                values.update(line)
            styles = first_styles if position == 1 else body_styles
            target_ws.append(self._data_row(target_ws, values, keys, styles, max_column, None))
            written += 1

        # The import group outlives the assembled block: a receipt is several lines.
        for line in line_iter:
            if written >= row_limit:
                logger.warning("Client-layout export: import group cut at Excel's row limit on %s", title)
                break
            target_ws.append(self._data_row(target_ws, line, keys, body_styles, max_column, import_columns))
            written += 1

    def save(self, stream) -> None:
        if self._sheet_count == 0:
            self.add_sheet(_CARDXABAR.sheet, [])
        self._styles.close(self.workbook)
        self.workbook.save(stream)

    # -- internals ------------------------------------------------------------
    def _copy_sheet_shape(self, source_ws, target_ws, *, active: bool) -> None:
        """Everything about the sheet that is not a cell: widths, groups, views, merges, rules."""
        for key, dim in source_ws.column_dimensions.items():
            target_ws.column_dimensions[key] = ColumnDimension(
                target_ws,
                index=dim.index,
                width=dim.width,
                bestFit=dim.bestFit,
                hidden=dim.hidden,
                outlineLevel=dim.outlineLevel,
                collapsed=dim.collapsed,
                style=self._styles.transplant(dim),
                min=dim.min,
                max=dim.max,
            )
        # openpyxl writes <sheetFormatPr> before it has looked at the columns, so
        # the outline depth that puts the group button above the sheet is lost
        # unless it is set by hand.
        target_ws.column_dimensions.max_outline = max(
            (dim.outlineLevel or 0) for dim in source_ws.column_dimensions.values()
        ) or None
        for index, dim in source_ws.row_dimensions.items():
            if dim.height or dim.hidden or dim.outlineLevel:
                target_ws.row_dimensions[index] = RowDimension(
                    target_ws, index=index, ht=dim.height, hidden=dim.hidden,
                    outlineLevel=dim.outlineLevel, collapsed=dim.collapsed,
                )
        target_ws.sheet_format = copy(source_ws.sheet_format)
        target_ws.sheet_properties = deepcopy(source_ws.sheet_properties)
        target_ws.views = deepcopy(source_ws.views)
        for view in target_ws.views.sheetView:
            # Two selected tabs would open the workbook with the sheets grouped.
            view.tabSelected = active
        target_ws.page_margins = copy(source_ws.page_margins)
        target_ws.page_setup = copy(source_ws.page_setup)
        target_ws.print_options = copy(source_ws.print_options)
        for merged in source_ws.merged_cells.ranges:
            target_ws.merged_cells.add(str(merged))
        for cf in source_ws.conditional_formatting:
            for rule in cf.rules:
                target_ws.conditional_formatting.add(str(cf.sqref), copy(rule))

    @staticmethod
    def _figures(rows: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
        """The counts their header rows carry: how many, the latest, the gross total."""
        stamps = [row.get("date_time") for row in rows if row.get("date_time")]
        return {
            "count": len(rows),
            "last_operation_at": max(stamps) if stamps else None,
            "total_amount": sum(row.get("amount_value") or 0 for row in rows) if rows else None,
        }

    @staticmethod
    def _captions(source_ws, template: SheetTemplate, source_display: str, combined: bool) -> Dict[str, str]:
        """
        What the header cells that name the source should say on this sheet.

        Their own banks keep the template's wording; another bank takes its
        place in the same sentence; the combined sheet has sentences of its own.
        """
        if combined:
            return dict(COMBINED_CAPTIONS)
        label = (source_display or "").strip()
        if not label or is_native_source(template, label):
            return {}
        captions: Dict[str, str] = {}
        for coordinate in template.captions:
            text = source_ws[coordinate].value
            if isinstance(text, str):
                captions[coordinate] = text.replace(template.token, label)
        return captions

    def _header_row(
        self, source_ws, target_ws, row_index: int, max_column: int, template: SheetTemplate,
        figures: Dict[str, Any], captions: Dict[str, str],
    ) -> List[Optional[Cell]]:
        cells: List[Optional[Cell]] = []
        for column in range(1, max_column + 1):
            source = source_ws.cell(row_index, column)
            coordinate = source.coordinate
            value = source.value
            if coordinate in template.figures:
                value = figures.get(template.figures[coordinate])
            elif coordinate in captions:
                value = captions[coordinate]
            cells.append(_make_cell(target_ws, value, self._styles.transplant(source)))
        return cells

    def _row_styles(self, source_ws, row_index: int, max_column: int) -> List[Optional[StyleArray]]:
        return [self._styles.transplant(source_ws.cell(row_index, column)) for column in range(1, max_column + 1)]

    @staticmethod
    def _column_keys(template: SheetTemplate, max_column: int) -> List[Optional[str]]:
        keys: List[Optional[str]] = []
        for column in range(1, max_column + 1):
            letter = get_column_letter(column)
            keys.append(IMPORT_COLUMNS.get(letter) or template.columns.get(letter))
        return keys

    @staticmethod
    def _data_row(
        target_ws, values: Dict[str, Any], keys: List[Optional[str]], styles: List[Optional[StyleArray]],
        max_column: int, only_columns: Optional[set],
    ) -> List[Optional[Cell]]:
        cells: List[Optional[Cell]] = []
        for column in range(1, max_column + 1):
            if only_columns is not None and column not in only_columns:
                cells.append(None)
                continue
            key = keys[column - 1]
            value = values.get(key) if key else None
            if value == "":
                value = None
            cells.append(_make_cell(target_ws, value, styles[column - 1]))
        return cells
