#!/usr/bin/env python3
"""Build the editable Excel synchronization client on Windows.

The default packages VBA source using the pinned EPPlus authoring dependency,
without running Excel or changing trust/licensing settings. --native explicitly
uses an already-authorized Microsoft Excel installation instead. The original
client template and the read-only Power Query builders are never changed.

  python backend/tools/build_sync_workbook.py --out backend/assets/excel_sync_template.xlsm
  python backend/tools/build_sync_workbook.py --probe

--prepare-only produces an isolated .xlsx shell for a normal VBE UI import on a
machine where programmatic VBA access is disabled. It is explicitly NOT a sync
client until the modules/document events have been imported and saved as .xlsm.
The master template contains no workbook ID, credentials, connection or data.
The first macro-enabled open creates a unique workbook ID before connection.
"""
from __future__ import annotations

import argparse
import json
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from copy import copy, deepcopy

BACKEND = Path(__file__).resolve().parents[1]
SOURCES = Path(__file__).with_name("excel_sync")
TEMPLATE = BACKEND / "services" / "excel_client_template.xlsx"
COMPACT_TEXT_COLUMNS = "S T U W AK AL AN AP AQ AR AS AT AU AV AX".split()
CORE_TO_CARD = {column: column for column in range(15, 54)}
CORE_TO_HUMO = {
    **{column: column for column in range(15, 26)},
    26: 32, 32: 33, 33: 40, 34: 41, 35: 42, 36: 43,
    37: 44, 38: 45, 39: 46, 40: 47,
    **{column: column + 7 for column in range(41, 54)},
}
EXTRA_HEADERS = {
    41: "Валюта", 42: "Получатель", 43: "Карта получателя", 44: "Исходный текст",
    45: "Источник", 46: "Чат", 47: "Сообщение", 48: "Парсер", 49: "Уверенность",
    50: "Состояние / действие", 51: "id", 52: "uuid", 53: "version",
}
VIEW_SHEETS = ("CardXabar", "HUMO Card", "Все транзакции")
LOCK_SHEET = "Доступ"  # ExcelSync.LockSheetName()
LOCK_TEXT = {
    "A1": "Чеки защищены",
    "A3": "Чеки открываются только на компьютере, активированном паролем владельца.",
    "A4": "Включите макросы и введите пароль. Чтобы ввести его снова, дважды щёлкните по этому листу.",
}


def _set_column_dimension(sheet, column: int, **attributes) -> None:
    """Override one column without losing a template's grouped dimensions."""
    from openpyxl.utils import get_column_letter

    letter = get_column_letter(column)
    for key, dimension in list(sheet.column_dimensions.items()):
        start = dimension.min or __import__("openpyxl").utils.column_index_from_string(key)
        end = dimension.max or start
        if start <= column <= end:
            del sheet.column_dimensions[key]
            for low, high in ((start, column - 1), (column, column), (column + 1, end)):
                if low <= high:
                    part = copy(dimension)
                    part.index = get_column_letter(low)
                    part.min, part.max = low, high
                    sheet.column_dimensions[part.index] = part
            break
    dimension = sheet.column_dimensions[letter]
    dimension.min = dimension.max = column
    for name, value in attributes.items():
        setattr(dimension, name, value)


def _add_validations(sheet, first_row: int, mapping: dict[int, int]) -> None:
    from openpyxl.utils import get_column_letter
    from openpyxl.worksheet.datavalidation import DataValidation

    for formula, column in (('"DEBIT,CREDIT,CONVERSION,REVERSAL"', 20),
                            ('"UZS,USD,EUR,RUB,KZT,GBP,CNY"', 41),
                            ('"0,1"', 35), ('"0,1"', 36), ('ExcelPersonCodes', 38), ('ExcelArticleCodes', 40)):
        validation = DataValidation(type="list", formula1=formula, allow_blank=True)
        validation.errorTitle = "Проверьте значение"
        validation.error = "Выберите значение из списка."
        validation.showErrorMessage = True
        # These cells display bank headings / code=label while the editor also
        # accepts their canonical list entries. VBA validates their round trip.
        if column in (20, 40):
            validation.showErrorMessage = False
        sheet.add_data_validation(validation)
        letter = get_column_letter(mapping[column])
        validation.add(f"{letter}{first_row}:{letter}1048576")


def _prepare_bank_view(book, name: str, first_row: int, mapping: dict[int, int], style_name: str) -> None:
    """Keep the bank's physical layout; the VBA adapter owns its row values."""
    from openpyxl.styles import Alignment
    from openpyxl.utils import column_index_from_string, get_column_letter

    sheet = book[name]
    last_original = mapping[40]
    last_column = mapping[53]
    last_row = max(first_row, sheet.max_row)
    white_header = copy(sheet.cell(2, mapping[38])._style)
    detail_style = copy(sheet.cell(first_row, mapping[37])._style)
    # Retain the template title, presentation column numbers and bank headings.
    # Import scaffolding, sample totals, dates, rows and old error values never
    # enter a distributable master. Row 3 keeps only the original bank labels.
    for row in sheet:
        for cell in row:
            if cell.__class__.__name__ == "MergedCell":
                continue
            keep = cell.column >= 15 and cell.column <= last_original and (
                cell.row in (1, 2) or (cell.row == 3 and isinstance(cell.value, str)
                                      and cell.data_type not in ("f", "e")))
            if not keep:
                cell.value = None
            cell.comment = None
            cell.hyperlink = None
    sheet.unmerge_cells(f"{get_column_letter(mapping[38])}2:{get_column_letter(mapping[39])}2")
    for core_column, label in ((38, "Лицо"), (39, "Период"), (40, "Статья")):
        cell = sheet.cell(2, mapping[core_column], label)
        cell._style = copy(white_header)
    # Discard the old available-SMS count formatting outside the bank table.
    for conditional_range in list(sheet.conditional_formatting):
        if all(area.min_col > last_original for area in conditional_range.sqref.ranges):
            del sheet.conditional_formatting[str(conditional_range.sqref)]
    for core_column, label in EXTRA_HEADERS.items():
        physical = mapping[core_column]
        header = sheet.cell(2, physical, label)
        header._style = copy(white_header)
        sheet.cell(first_row, physical)._style = copy(detail_style)
        _set_column_dimension(sheet, physical, width=32 if core_column in (44, 50) else 15,
                              hidden=core_column in (51, 52, 53))
    status = sheet.cell(1, mapping[50], "Подключение настроится автоматически")
    status._style = copy(white_header)
    status.number_format = "@"
    status.alignment = Alignment(horizontal="left", vertical="center", wrap_text=False, shrink_to_fit=False)
    sheet["P3"] = f'=COUNT(Q{first_row}:Q1048576)'
    sheet["T3"] = f'=IF(COUNT(Q{first_row}:Q1048576)=0,"",MAX(Q{first_row}:Q1048576))'
    sheet["T3"].number_format = sheet.cell(first_row, 17).number_format
    if not sheet["U3"].value:
        sheet["U3"] = "- дата крайней операции в HumoCard"
        sheet["U3"]._style = copy(sheet["Q3"]._style)
    clip_columns = {mapping[column_index_from_string(col)] for col in COMPACT_TEXT_COLUMNS}
    # The hidden raw amount and balance are also strings and must not spill.
    clip_columns.update((mapping[25], mapping[32]))
    text_columns = {mapping[column_index_from_string(col)] for col in
                    "S T U W X Y AF AH AK AL AN AO AP AQ AR AS AT AU AV AX AY AZ BA".split()}
    if name == "HUMO Card":
        # Bank digit helpers are original text fragments ("9.000", "000").
        # General silently converts them to 9/0 in Excel and causes an endless
        # rewrite on every render. Only the two length helpers stay numeric.
        text_columns.update((26, 28, 29, 30, 31, 34, 36, 37, 38, 39))
    for column in range(15, last_column + 1):
        cell = sheet.cell(first_row, column)
        alignment = copy(cell.alignment)
        alignment.wrap_text = column in clip_columns
        if column in clip_columns:
            alignment.vertical = "top"
            alignment.shrink_to_fit = False
        cell.alignment = alignment
        if column in text_columns:
            cell.number_format = "@"
    for column in (mapping[35], mapping[36]):
        sheet.cell(first_row, column).number_format = "0"
    sheet.cell(first_row, mapping[49]).number_format = "0.00"
    for row in range(first_row, last_row + 1):
        sheet.row_dimensions[row].height = 12
        for column in range(15, last_column + 1):
            sheet.cell(row, column)._style = copy(sheet.cell(first_row, column)._style)
    # Both bank snapshots use physical columns on row 5. HUMO's visible body
    # begins on row 4, but its adapter can still use the same style-row contract.
    if style_name not in book.sheetnames:
        style = book.create_sheet(style_name)
        style.row_dimensions[5].height = 12
        for column in range(1, last_column + 1):
            style.cell(5, column)._style = copy(sheet.cell(first_row, column)._style)
        style.sheet_state = "veryHidden"
    sheet.sheet_view.tabSelected = name == "CardXabar"
    _add_validations(sheet, first_row, mapping)


def prepare_shell(out: Path) -> dict:
    """Author an isolated OOXML shell when native Excel editing is unavailable.

    Used only for the explicitly requested new macro-client shell. This is not
    the existing Power Query exporter. The data sheet owns the status and row
    actions; macros manage automatic synchronization and the blank input row.
    """
    import openpyxl
    from openpyxl.styles import Alignment, Font, Protection
    from openpyxl.workbook.protection import WorkbookProtection
    from openpyxl.workbook.defined_name import DefinedName
    from openpyxl.worksheet.views import Pane, Selection, SheetView, SheetViewList

    book = openpyxl.load_workbook(TEMPLATE, keep_links=False)
    all_view = book.copy_worksheet(book["CardXabar"])
    all_view.title = "Все транзакции"
    all_view.views = deepcopy(book["CardXabar"].views)
    sheet = book.copy_worksheet(book["CardXabar"])
    template_row_height = sheet.row_dimensions[5].height
    if template_row_height is None:
        template_row_height = sheet.sheet_format.defaultRowHeight
    sheet.row_dimensions[5].height = template_row_height
    sheet.title = "_transactions"
    for row in sheet.iter_rows(min_row=5):
        for cell in row:
            if cell.__class__.__name__ != "MergedCell":
                cell.value = None
    # The reference carries sample counts, timestamps and zero-filled import
    # scaffolding above the body. Keep its formatting, not those stale values.
    for row in sheet.iter_rows(min_row=1, max_row=4):
        for cell in row:
            if cell.__class__.__name__ != "MergedCell" and (cell.column < 15 or cell.row != 2):
                cell.value = None
    sheet["B1"] = None
    sheet.unmerge_cells("AL2:AM2")
    for merged in ("O1:S1", "T1:AN1", "O4:AN4"):
        sheet.merge_cells(merged)
    sheet["O1"] = "Все транзакции"
    sheet["T1"] = "Подключение настроится автоматически"
    sheet["O4"] = "Новый чек: заполните строку внизу. Дважды нажмите статус строки для действий."
    for address in ("O1", "T1", "O4"):
        sheet[address].number_format = "@"
        sheet[address].alignment = Alignment(horizontal="left", vertical="center", wrap_text=False)
    sheet.row_dimensions[4].height = template_row_height
    sheet["P2"] = "№ опер."
    sheet["R2"] = "Дата"
    sheet["AL2"] = "Лицо"
    sheet["AM2"] = "Период"
    sheet["AN2"] = "Статья"
    for col in ("AL", "AM", "AN"):
        sheet[f"{col}2"]._style = copy(sheet["AK2"]._style)
        sheet[f"{col}2"].alignment = Alignment(horizontal="center", vertical="center", shrink_to_fit=True)
    sheet["P3"] = '=COUNT(Q5:Q1048576)'
    sheet["Q3"] = "Чеки"
    sheet["T3"] = '=IF(COUNT(Q5:Q1048576)=0,"",MAX(Q5:Q1048576))'
    sheet["T3"].number_format = "dd.mm.yyyy hh:mm"
    sheet["U3"] = "Последняя операция"
    sheet["Z3"] = '=SUMIF(AO5:AO1048576,"UZS",Z5:Z1048576)'
    sheet["W3"] = "Оборот UZS"
    sheet["AO3"] = '=SUMIF(AO5:AO1048576,"USD",Z5:Z1048576)'
    sheet["AP3"] = "Оборот USD"
    sheet["Z3"].number_format = '#,##0.00" UZS"'
    sheet["AO3"].number_format = '#,##0.00" USD"'
    extra = {"AO":"Валюта", "AP":"Получатель", "AQ":"Карта получателя", "AR":"Исходный текст",
             "AS":"Источник", "AT":"Чат", "AU":"Сообщение", "AV":"Парсер", "AW":"Уверенность",
             "AX":"Состояние / действие", "AY":"id", "AZ":"uuid", "BA":"version"}
    for col, label in extra.items():
        sheet[f"{col}2"] = label
        sheet[f"{col}2"]._style = copy(sheet["AK2"]._style)
        sheet[f"{col}5"]._style = copy(sheet["AK5"]._style)
        sheet.column_dimensions[col].width = 15 if col not in {"AR", "AX"} else 32
    text_cols = "S T U W X AH AK AL AN AO AP AQ AR AS AT AU AV AX AY AZ BA".split()
    for col in text_cols:
        sheet[f"{col}5"].number_format = "@"
        sheet.column_dimensions[col].number_format = "@"
    # Keep full transaction time in Q and the short date in its read-only R
    # companion. The reference's 12-character widths clip the longer format.
    for col, width, number_format in (("Q", 23, "dd.mm.yyyy hh:mm:ss"), ("R", 13, "dd.mm.yyyy")):
        sheet.column_dimensions[col].width = width
        sheet.column_dimensions[col].number_format = number_format
        sheet[f"{col}5"].number_format = number_format
    for col, width in (("Z", 20), ("AG", 20), ("AM", 12)):
        sheet.column_dimensions[col].width = width
    sheet["AM5"].number_format = "mmm yyyy"
    for col in ("Y", "Z", "AF", "AG"):
        sheet[f"{col}5"].number_format = "#,##0.00"
    for col in ("AI", "AJ"):
        sheet[f"{col}5"].number_format = "0"
    for col in range(15, 54):
        alignment = copy(sheet.cell(5, col).alignment)
        clip_text = openpyxl.utils.get_column_letter(col) in COMPACT_TEXT_COLUMNS
        alignment.wrap_text = clip_text
        if clip_text:
            alignment.vertical = "top"
            alignment.shrink_to_fit = False
        sheet.cell(5, col).alignment = alignment
    for col in range(1, 15):
        sheet.column_dimensions[openpyxl.utils.get_column_letter(col)].hidden = True
    for col in ("AY", "AZ", "BA"):
        sheet.column_dimensions[col].hidden = True
    # Replace the reference's horizontal split and all four pane selections.
    # O5 is the scroll origin, not a request to freeze hidden columns A:N.
    sheet.views = SheetViewList(sheetView=[SheetView(
        workbookViewId=0, topLeftCell="O1", showZeros=False, zoomScale=100, zoomScaleNormal=100,
        pane=Pane(ySplit=4, topLeftCell="O5", activePane="bottomLeft", state="frozen"),
        selection=[Selection(pane="bottomLeft", activeCell="O5", sqref="O5")],
    )])
    style = book.create_sheet("_style")
    style.row_dimensions[5].height = template_row_height
    for col in range(15, 54):
        style.cell(5, col)._style = copy(sheet.cell(5, col)._style)
        style.cell(5, col).protection = Protection(locked=True)
    style.sheet_state = "veryHidden"
    settings = book.create_sheet("_sync")
    configuration = (("workbook_id", None), ("protocol", "1"), ("data_sheet", sheet.title),
                     ("source_version", "1"), ("server_url", None), ("shared_token", None),
                     ("owner_id", None), ("shared_mode", "1"), ("status", None), ("active_view", None))
    for row, (label, value) in enumerate(configuration, 1):
        settings.cell(row, 1, label)
        settings.cell(row, 2, value).number_format = "@"
    settings.sheet_state = "veryHidden"
    refs = book.create_sheet("_refs")
    refs.append(["person_code", "person_label", "article_code", "article_label"])
    for col in "ABCD":
        refs.column_dimensions[col].number_format = "@"
    refs.sheet_state = "veryHidden"
    for name, column in (("ExcelPersonCodes", "A"), ("ExcelArticleCodes", "C")):
        book.defined_names.add(DefinedName(name, attr_text=f"'_refs'!${column}$2:INDEX('_refs'!${column}:${column},MAX(2,COUNTA('_refs'!${column}:${column})))"))
    _add_validations(sheet, 5, CORE_TO_CARD)
    _prepare_bank_view(book, "CardXabar", 5, CORE_TO_CARD, "_styleCard")
    _prepare_bank_view(book, "HUMO Card", 4, CORE_TO_HUMO, "_styleHumo")
    _prepare_bank_view(book, "Все транзакции", 5, CORE_TO_CARD, "_styleCard")
    all_view["O1"] = "Все транзакции"
    all_view["Q3"] = "- число операций"
    all_view["U3"] = "- дата последней операции"
    sheet.sheet_state = "veryHidden"
    # A saved file never shows receipts: macros reveal the views only on a
    # computer the owner's password has activated (see ExcelSync.ShowViews).
    lock = book.create_sheet(LOCK_SHEET, 0)
    lock.column_dimensions["A"].width = 110
    for address, text in LOCK_TEXT.items():
        lock[address] = text
    lock["A1"].font = Font(bold=True, size=16)
    lock["A6"].number_format = "@"
    for name in VIEW_SHEETS:
        book[name].sheet_state = "veryHidden"
    for target in book.worksheets:
        for view in target.views.sheetView:
            view.tabSelected = target is lock
    for target in book.worksheets:
        for row in target:
            for cell in row:
                if cell.__class__.__name__ != "MergedCell":
                    cell.protection = Protection(locked=True)
        target.protection.set_password("tbs-layout")
        target.protection.sheet = True
    book.security = WorkbookProtection(lockStructure=True)
    book.security.set_workbook_password("tbs-layout")
    book.active = 0
    book.calculation.fullCalcOnLoad = True
    out.parent.mkdir(parents=True, exist_ok=True)
    book.save(out)
    return {"path": str(out), "macro_client": False, "authoring": "OOXML shell",
            "workbook_id_blank": True, "queries": 0, "connections": 0, "data_sheet": sheet.title,
            "visible_sheets": [LOCK_SHEET], "view_sheets": list(VIEW_SHEETS)}


def build(out: Path, *, prepare_only: bool = False, probe: bool = False, native: bool = False) -> dict:
    if os.name != "nt":
        raise RuntimeError("This builder requires Windows PowerShell and .NET Framework.")
    out = out.resolve()
    expected = ".xlsx" if prepare_only else ".xlsm"
    if not probe and out.suffix.lower() != expected:
        raise ValueError(f"Output must end with {expected}")
    if out == TEMPLATE.resolve():
        raise ValueError("The source template must not be overwritten.")
    if not TEMPLATE.is_file():
        raise FileNotFoundError(TEMPLATE)
    if prepare_only and not probe:
        return prepare_shell(out)
    if not native and not probe:
        package_path = SOURCES / "vba_package.py"
        if not package_path.is_file():
            raise RuntimeError("The VBA packaging helper is missing.")
        spec = importlib.util.spec_from_file_location("excel_sync_vba_package", package_path)
        package = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(package)
        with tempfile.TemporaryDirectory(prefix="tbs-excel-shell-") as directory:
            shell_path = Path(directory) / "shell.xlsx"
            prepare_shell(shell_path)
            package.package_workbook(shell_path, out, SOURCES)
        return {"path": str(out), "macro_client": True, "workbook_id_blank": True,
                "authoring": "EPPlus VBA source packaging", "runtime_tested": False}
    shell = shutil.which("powershell.exe")
    if not shell:
        raise RuntimeError("Windows PowerShell is unavailable.")
    out.parent.mkdir(parents=True, exist_ok=True)
    # No secrets ever enter this build manifest, command line or workbook.
    with tempfile.TemporaryDirectory(prefix="tbs-excel-build-") as directory:
        settings = Path(directory) / "build.json"
        native_script = Path(directory) / "native.ps1"
        result = Path(directory) / "result.json"
        working = Path(directory) / "working.xlsx"
        # Both authoring paths consume exactly the same prepared bank layout.
        prepare_shell(working)
        # Windows PowerShell 5 reads a BOM-less .ps1 using the ANSI code page.
        native_script.write_text((SOURCES / "build_native.ps1").read_text(encoding="utf-8-sig"), encoding="utf-8-sig")
        settings.write_text(json.dumps({
            "working": str(working), "out": str(out), "sources": str(SOURCES),
            "prepare_only": prepare_only, "probe": probe, "result": str(result),
        }), encoding="utf-8")
        run = subprocess.run(
            [shell, "-NoProfile", "-NonInteractive", "-File", str(native_script),
             "-Settings", str(settings)],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=180,
        )
        if run.returncode:
            raise RuntimeError((run.stderr or run.stdout).strip())
        if not result.is_file():
            raise RuntimeError("Excel did not return a build report.")
        return json.loads(result.read_text(encoding="utf-8-sig"))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=BACKEND / "assets" / "excel_sync_template.xlsm")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--probe", action="store_true")
    parser.add_argument("--native", action="store_true", help="Use authorized Excel COM rather than portable source packaging")
    args = parser.parse_args()
    print(json.dumps(build(args.out, prepare_only=args.prepare_only, probe=args.probe, native=args.native), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
