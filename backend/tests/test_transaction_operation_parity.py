"""«Операция» parity: what the transaction API sends for it, and the export's sort by it.

The desktop client derives the cell from a row the way the Excel workbook does
(desktop/crates/tbs-core/src/operation.rs, ExcelSync.bas OperationDisplay). The
workbook reads the stored type and the bank assigned to the row; the API used
to send neither. The server export had no order for the column at all.
"""
from __future__ import annotations

from io import BytesIO

import openpyxl
import pytest

from api.routes.transactions import TransactionResponse
from services.excel_sync_service import serialize_record
from tests.test_transactions_export import (  # noqa: F401  (client is a fixture)
    _seed_monitored_chat,
    _seed_tx,
    client,
)

CARDXABAR_CHAT, HUMO_CHAT = -700, -701


def seed(db_session, *, idx, day, raw, chat=CARDXABAR_CHAT, **values):
    tx = _seed_tx(db_session, idx=idx, day=day, amount="-100.00", operator=f"OP-{idx}", source_chat_id=chat)
    tx.raw_message = raw
    for key, value in values.items():
        setattr(tx, key, value)
    db_session.commit()
    return tx


# --- the two inputs in every transaction the API sends -------------------------

def test_the_api_sends_the_stored_type_and_the_assigned_bank_beside_the_inferred_type(client, db_session):
    # Stored as money out; the words of the receipt make the API call it a top-up.
    differing = seed(db_session, idx=1, day=2, raw="Пополнение баланса телефона", excel_bank="HUMO Card")
    ordinary = seed(db_session, idx=2, day=3, raw="🔴 E-Com oplata\n➖ 100.00 UZS")
    listed = client.get("/api/transactions/", params={"page_size": 50})
    assert listed.status_code == 200, listed.text
    items = {item["id"]: item for item in listed.json()["items"]}

    one = items[differing.id]
    # What clients already read is what it was.
    assert (one["transaction_type"], one["transaction_type_display"]) == ("CREDIT", "Пополнение")
    assert (one["stored_transaction_type"], one["excel_bank"]) == ("DEBIT", "HUMO Card")
    other = items[ordinary.id]
    assert (other["transaction_type"], other["stored_transaction_type"], other["excel_bank"]) == ("DEBIT", "DEBIT", None)
    # The same two values the Excel snapshot gives the workbook for these rows.
    for tx in (differing, ordinary):
        fields = serialize_record(tx)["fields"]
        assert items[tx.id]["stored_transaction_type"] == fields["transaction_type"]
        assert items[tx.id]["excel_bank"] == fields["excel_bank"]

    detail = client.get(f"/api/transactions/{differing.id}")
    assert detail.status_code == 200, detail.text
    assert {key: detail.json()[key] for key in ("transaction_type", "stored_transaction_type", "excel_bank")} == {
        "transaction_type": "CREDIT", "stored_transaction_type": "DEBIT", "excel_bank": "HUMO Card"}


def test_the_new_fields_are_optional_additions_to_the_reply():
    fields = TransactionResponse.model_fields
    for name in ("stored_transaction_type", "excel_bank"):
        assert not fields[name].is_required() and fields[name].default is None
    # Nothing a client already reads was renamed or made optional.
    assert fields["transaction_type"].is_required() and fields["transaction_type_display"].is_required()


# --- GET /api/transactions/export?sort_by=operation -----------------------------

ROWS = [  # idx, day, chat, receipt, the cell the export writes
    (1, 1, CARDXABAR_CHAT, "🔴 Platezh\n➖ 1 000.00 UZS", "🔴 Platezh"),
    (2, 2, CARDXABAR_CHAT, "🔴 E-Com oplata\n➖ 5 000.00 UZS", "🔴 E-Com oplata"),
    (3, 3, HUMO_CHAT, "💸 Оплата\n➖ 40.000,00 UZS", "💸 Оплата"),
    (4, 4, HUMO_CHAT, "[Photo]\nПеревод через Payme", "💸 Списание"),
    (5, 5, CARDXABAR_CHAT, "Перевод", "🔴 Списание"),
    (6, 6, CARDXABAR_CHAT, "🔴 e-com OPLATA\n➖ 7 000.00 UZS", "🔴 e-com OPLATA"),
    (7, 7, HUMO_CHAT, "ℹ️ Счет по карте изменен\n➕ 2.257.419,87 UZS", "ℹ️ Счет по карте изменен"),
]
# Lower-cased and by code point: ℹ (U+2139) before 💸 (U+1F4B8) before 🔴 (U+1F534).
# Rows 2 and 6 differ only in case, so they keep the order the desktop client
# receives rows in (by date, then id) in both directions: the older one first.
ASCENDING = [7, 3, 4, 2, 6, 1, 5]
DESCENDING = [5, 1, 2, 6, 4, 3, 7]


@pytest.fixture
def receipts(db_session):
    _seed_monitored_chat(db_session, chat_id=CARDXABAR_CHAT, title="CardXabar")
    _seed_monitored_chat(db_session, chat_id=HUMO_CHAT, title="HUMO Card")
    for idx, day, chat, raw, _cell in ROWS:
        seed(db_session, idx=idx, day=day, raw=raw, chat=chat)
    return {f"OP-{idx}": cell for idx, _day, _chat, _raw, cell in ROWS}


def exported(client, **params) -> openpyxl.Workbook:
    response = client.get("/api/transactions/export", params=params)
    assert response.status_code == 200, response.text
    return openpyxl.load_workbook(BytesIO(response.content))


def block(sheet, first_row: int) -> list[tuple[str, str]]:
    """(Операция, Оператор) of the assembled rows: columns T and U of the customer's layout."""
    rows = []
    for row in range(first_row, sheet.max_row + 1):
        if sheet.cell(row, 21).value:
            rows.append((sheet.cell(row, 20).value, sheet.cell(row, 21).value))
    return rows


@pytest.mark.parametrize("direction,order", [("asc", ASCENDING), ("desc", DESCENDING)])
def test_the_client_layout_is_ordered_by_its_operation_cell(client, receipts, direction, order):
    book = exported(client, sort_by="operation", sort_dir=direction, layout="client")
    combined = block(book["Все транзакции"], 5)
    assert [operator for _cell, operator in combined] == [f"OP-{idx}" for idx in order]
    # The order is the order of the cells the file itself shows.
    assert all(cell == receipts[operator] for cell, operator in combined)
    keys = [cell.lower() for cell, _operator in combined]
    assert keys == sorted(keys, reverse=direction == "desc")
    # Each bank's own sheet follows the same order.
    assert [operator for _cell, operator in block(book["CardXabar"], 5)] == [
        f"OP-{idx}" for idx in order if idx in (1, 2, 5, 6)]
    assert [operator for _cell, operator in block(book["HUMO Card"], 4)] == [
        f"OP-{idx}" for idx in order if idx in (3, 4, 7)]


@pytest.mark.parametrize("direction,order", [("asc", ASCENDING), ("desc", DESCENDING)])
def test_the_plain_layout_follows_the_same_order(client, receipts, direction, order):
    sheet = exported(client, sort_by="operation", sort_dir=direction, layout="plain",
                     columns="operator_raw").worksheets[0]
    assert [sheet.cell(row, 1).value for row in range(2, sheet.max_row + 1)] == [f"OP-{idx}" for idx in order]


def test_filters_still_decide_which_rows_are_ordered(client, receipts):
    sheet = exported(client, sort_by="operation", sort_dir="asc", layout="plain", columns="operator_raw",
                     operators="OP-1,OP-3,OP-5").worksheets[0]
    assert [sheet.cell(row, 1).value for row in range(2, sheet.max_row + 1)] == ["OP-3", "OP-1", "OP-5"]


def test_a_row_is_ordered_by_what_its_cell_says_when_the_types_differ(client, db_session, receipts):
    # Stored DEBIT, read as a top-up from its words: the export writes the
    # inferred wording (as before), and that text is what it sorts by.
    seed(db_session, idx=8, day=8, raw="Пополнение баланса телефона")
    book = exported(client, sort_by="operation", sort_dir="asc", layout="client")
    combined = block(book["Все транзакции"], 5)
    assert ("🔴 Пополнение", "OP-8") in combined
    assert [operator for _cell, operator in combined] == [f"OP-{idx}" for idx in (7, 3, 4, 2, 6, 1, 8, 5)]


def test_other_sort_keys_and_an_empty_result_are_as_before(client, db_session, receipts):
    by_date = exported(client, sort_by="no_such_column", layout="plain", columns="operator_raw").worksheets[0]
    assert [by_date.cell(row, 1).value for row in range(2, by_date.max_row + 1)] == [f"OP-{idx}" for idx in range(1, 8)]
    nothing = exported(client, sort_by="operation", layout="plain", columns="operator_raw",
                       operators="NO-SUCH-OPERATOR").worksheets[0]
    assert nothing.max_row == 1
    assert exported(client, sort_by="operation", operators="NO-SUCH-OPERATOR").sheetnames
