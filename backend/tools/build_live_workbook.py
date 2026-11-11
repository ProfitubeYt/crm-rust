#!/usr/bin/env python3
"""
Build the customer's live workbook — their own sheet, refreshing itself.

The customer asked for a file that always shows the current receipts instead of
exporting a new one each time, and looking exactly like the export they already
have. So this is not a table with our own styling on it: it is the same pruned
copy of their workbook that `services/excel_client_layout.py` writes into
(`services/excel_client_template.xlsx`), with a Power Query landed in the
assembled block instead of rows.

Two things that make that work:

* **The query lands at their own letters.** `GET /export.csv?layout=client`
  emits the whole block — spacers included — so `Z` is Сумма and `AG` is Остаток
  in the live sheet exactly as in the export and in `Книга1.xlsx`. Field names
  are not written: their header row is already there, on row 2.
* **The body format is applied to the columns, not to the rows.** A refresh that
  brings more receipts than last time would otherwise land them on unformatted
  rows. The template's body row is copied down once, so any number of rows keeps
  the grid, the tints and the number formats.

The figures in their counts row become formulas, so the row count, the date of
the last operation and the gross total move with the data.

An earlier attempt hand-wrote the Power Query into the .xlsx zip: the M code
lives base64-encoded inside a DataMashup blob (MS-QDEFF), and a byte out of
place means Excel either offers to repair the file or silently drops the query.
This asks Excel to write it instead — which is why it needs Excel installed, and
why it is an operator tool run on Windows rather than something the server does.

    python build_live_workbook.py --url https://host/api/transactions/export.csv \
        --key THE_EXPORT_KEY --out receipts_live.xlsx [--interval 5]

The first refresh asks once for a connection method — Anonymous is correct: the
key travels in a header, not in the URL.
"""
from __future__ import annotations

import argparse
import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from services.excel_client_layout import (  # noqa: E402
    COMBINED_CAPTIONS,
    SHEET_TEMPLATES,
    TEMPLATE_PATH,
    block_columns,
)

QUERY_NAME = "Чеки"
SHEET_NAME = "Все транзакции"
TEMPLATE = SHEET_TEMPLATES["CardXabar"]

#: How far down the body formatting is carried before the refresh takes over.
#: Excel stores a style per cell, so 200 000 rows is a 23 MB file for 1 500
#: receipts; past this the QueryTable's own `PreserveFormatting` extends the
#: grid as rows arrive.
FORMAT_ROWS = 20_000

#: Keys whose values are whole numbers or dates.
NUMERIC_KEYS = (
    "row_number", "operation_number", "amount_value", "balance_value",
    "p2p", "p2p_confirmed",
)
DATETIME_KEYS = ("date_time", "date_time_dedup")
DATE_KEYS = ("period",)
#: Columns their formulas fill with either a word or a zero. Left as text, the
#: zero would be *visible*: the sheet view hides numeric zeros, not "0".
MIXED_KEYS = ("application", "balance_text")

XL_PASTE_FORMATS = -4122
XL_CMD_SQL = 2
XL_OPEN_XML_WORKBOOK = 51


def _csv_headers() -> list:
    """The CSV's own column names, matching `_iter_client_layout_csv`."""
    return ["id"] + [key or f"_{letter}" for letter, key in block_columns()]


def build_m_query(url: str, key: str) -> str:
    """
    The Power Query that pulls the live table.

    The URL is a literal rather than assembled from parts: a computed URL makes
    Power Query treat the source as dynamic, which trips the privacy-level
    prompt on every refresh. The key rides in a header, so it never lands in a
    browser history, a proxy log or the workbook's visible connection string.

    One locale is enough here, unlike the flat feed this replaces: the CSV emits
    ISO timestamps and unformatted integers on purpose, so `en-US` reads both.
    A single mixed-dialect pass used to empty the Сумма column silently.
    """
    safe_url = url.replace('"', '""')
    safe_key = key.replace('"', '""')

    typed = []
    for header in _csv_headers():
        if header == "id" or header in MIXED_KEYS:
            continue                                     # handled separately
        if header.startswith("_") or header in NUMERIC_KEYS:
            # The spacers are always empty, and typing them as a number turns
            # `""` into a real empty cell rather than an empty string.
            typed.append(f'{{"{header}", Int64.Type}}')
        elif header in DATETIME_KEYS:
            typed.append(f'{{"{header}", type datetime}}')
        elif header in DATE_KEYS:
            typed.append(f'{{"{header}", type date}}')
        else:
            typed.append(f'{{"{header}", type text}}')
    types = ",\n            ".join(typed)

    # Their `Приложение` holds a name or a zero. Left as text the zero would show
    # as "0" — the sheet view hides numeric zeros, not text ones — so it is put
    # back to a number where it is one.
    mixed = ",\n            ".join(
        f'{{"{key}", each try Number.From(_) otherwise _}}' for key in MIXED_KEYS
    )
    # Exactly their block, spacers included and in their order, so the rows land
    # across O..AN with Сумма on `Z` and Остаток on `AG`. `id` has no column in
    # their layout: dropping it is what keeps everything from shifting right.
    kept = ", ".join(f'"{key or "_" + letter}"' for letter, key in block_columns())

    # An absent value is an empty field in the CSV, and `type text` would land
    # it as an empty string — a cell that is not empty, only invisible, and that
    # a COUNTA or a filter still sees.
    text_columns = ", ".join(
        f'"{header}"' for header in _csv_headers()
        if header != "id"
        and not header.startswith("_")
        and header not in NUMERIC_KEYS
        and header not in DATETIME_KEYS
        and header not in DATE_KEYS
    )

    return f'''let
    Источник = Csv.Document(
        Web.Contents("{safe_url}", [Headers=[#"X-Export-Key"="{safe_key}"]]),
        [Delimiter=",", Encoding=65001, QuoteStyle=QuoteStyle.Csv]
    ),
    Заголовки = Table.PromoteHeaders(Источник, [PromoteAllScalars=true]),
    Типы = Table.TransformColumnTypes(
        Заголовки,
        {{
            {types}
        }},
        "en-US"
    ),
    Смешанные = Table.TransformColumns(
        Типы,
        {{
            {mixed}
        }}
    ),
    Пустые = Table.ReplaceValue(
        Смешанные, "", null, Replacer.ReplaceValue, {{{text_columns}}}
    ),
    Порядок = Table.SelectColumns(Пустые, {{{kept}}})
in
    Порядок'''


def build(url: str, key: str, out_path: str, interval_minutes: int) -> str:
    import win32com.client as com

    out_path = os.path.abspath(out_path)
    # Excel edits in place, and the template must survive untouched.
    working = os.path.join(tempfile.mkdtemp(prefix="live-workbook-"), "live.xlsx")
    shutil.copyfile(TEMPLATE_PATH, working)

    excel = com.DispatchEx("Excel.Application")
    excel.Visible = False
    excel.DisplayAlerts = False
    excel.AskToUpdateLinks = False

    try:
        workbook = excel.Workbooks.Open(working, UpdateLinks=0)
        sheet = workbook.Worksheets(TEMPLATE.sheet)
        for other in list(workbook.Worksheets):
            if other.Name != TEMPLATE.sheet:
                other.Delete()
        sheet.Name = SHEET_NAME

        _prepare_sheet(sheet)
        _write_captions(sheet)

        workbook.Queries.Add(Name=QUERY_NAME, Formula=build_m_query(url, key))

        # QueryTables.Add rather than ListObjects.Add: the latter rejects a
        # Mashup connection string with E_INVALIDARG, which is a dead end and not
        # an argument worth having with Excel.
        sheet.Activate()
        first_cell = f"{_first_letter()}{TEMPLATE.first_row}"
        connection_string = (
            "OLEDB;Provider=Microsoft.Mashup.OleDb.1;Data Source=$Workbook$;"
            f"Location={QUERY_NAME}"
        )
        query_table = sheet.QueryTables.Add(
            Connection=connection_string,
            Destination=sheet.Range(first_cell),
        )
        query_table.CommandType = XL_CMD_SQL
        query_table.CommandText = f"SELECT * FROM [{QUERY_NAME}]"
        # Their header is already on row 2; a second one would push the data down.
        query_table.FieldNames = False
        query_table.BackgroundQuery = True
        query_table.RefreshOnFileOpen = True
        query_table.PreserveFormatting = True
        query_table.AdjustColumnWidth = False
        query_table.Refresh(BackgroundQuery=False)

        connection = workbook.Connections(1)
        connection.RefreshWithRefreshAll = True
        if interval_minutes > 0:
            connection.OLEDBConnection.RefreshPeriod = int(interval_minutes)
        connection.OLEDBConnection.RefreshOnFileOpen = True

        _write_figures(sheet)

        workbook.SaveAs(out_path, FileFormat=XL_OPEN_XML_WORKBOOK)
        workbook.Close(SaveChanges=False)
        return out_path
    finally:
        excel.Quit()
        shutil.rmtree(os.path.dirname(working), ignore_errors=True)


def _first_letter() -> str:
    return block_columns()[0][0]


def _last_letter() -> str:
    return block_columns()[-1][0]


def _prepare_sheet(sheet) -> None:
    """
    Empty the template's sample rows and carry their look down the sheet.

    The template holds two data rows so the writer can take a first-row and a
    body style from it. Here they are the seed for every row the query will
    bring: the body row's format is copied down, so a refresh that returns more
    receipts than the last one still lands them on their own grid.
    """
    first, body = TEMPLATE.first_row, TEMPLATE.body_row
    last_column = _last_letter()

    sheet.Range(f"A{body}:{last_column}{body}").Copy()
    sheet.Range(f"A{body + 1}:{last_column}{FORMAT_ROWS}").PasteSpecial(Paste=XL_PASTE_FORMATS)
    sheet.Application.CutCopyMode = False

    sheet.Range(f"A{first}:{last_column}{FORMAT_ROWS}").ClearContents()


def _write_captions(sheet) -> None:
    for coordinate, text in COMBINED_CAPTIONS.items():
        sheet.Range(coordinate).Value = text


def _write_figures(sheet) -> None:
    """
    Their counts row, as formulas rather than a snapshot.

    The export writes the figures it computed; a live sheet has to recompute
    them, or the row would still describe whatever was in the file the day it
    was built.
    """
    first = TEMPLATE.first_row
    letters = {key: letter for letter, key in block_columns() if key}
    numbers = f"${letters['row_number']}${first}:${letters['row_number']}${FORMAT_ROWS}"
    stamps = f"${letters['date_time']}${first}:${letters['date_time']}${FORMAT_ROWS}"
    amounts = f"${letters['amount_value']}${first}:${letters['amount_value']}${FORMAT_ROWS}"

    formulas = {
        "count": f"=COUNT({numbers})",
        "last_operation_at": f"=MAX({stamps})",
        "total_amount": f"=SUM({amounts})",
    }
    for coordinate, figure in TEMPLATE.figures.items():
        formula = formulas.get(figure)
        if formula:
            sheet.Range(coordinate).Formula = formula


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True, help="Full URL of /api/transactions/export.csv")
    parser.add_argument("--key", required=True, help="Value for the X-Export-Key header")
    parser.add_argument("--out", default="receipts_live.xlsx")
    parser.add_argument("--interval", type=int, default=5, help="Background refresh, minutes (0 = off)")
    args = parser.parse_args()

    url = args.url
    if "layout=" not in url:
        url = f"{url}{'&' if '?' in url else '?'}layout=client"

    try:
        path = build(url, args.key, args.out, args.interval)
    except Exception as exc:  # noqa: BLE001
        print(f"Не удалось собрать книгу: {exc}", file=sys.stderr)
        return 1

    print(f"Книга собрана: {path}")
    print("При первом обновлении Excel спросит способ подключения — выберите «Анонимно».")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
