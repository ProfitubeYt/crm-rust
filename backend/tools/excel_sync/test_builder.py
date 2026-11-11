"""Isolated shell verification. Run directly; no backend DB fixtures are loaded."""
from __future__ import annotations

import hashlib
import io
import importlib.util
import re
from pathlib import Path
import tempfile
import unittest
import zipfile
from copy import copy

import openpyxl

SPEC = importlib.util.spec_from_file_location("sync_builder", Path(__file__).parents[1] / "build_sync_workbook.py")
builder = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(builder)


class WorkbookShellTests(unittest.TestCase):
    def test_bank_views_preserve_reference_and_physical_field_positions(self):
        def dimension(sheet, col):
            for item in sheet.column_dimensions.values():
                if item.min <= col <= item.max:
                    return item.width, item.hidden, item.outlineLevel
            return None, False, 0

        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "banks.xlsx"
            builder.prepare_shell(target)
            book = openpyxl.load_workbook(io.BytesIO(target.read_bytes()))  # never lock the temp file
            original = openpyxl.load_workbook(builder.TEMPLATE)
            for name, first, mapping, style_name in (
                    ("CardXabar", 5, builder.CORE_TO_CARD, "_styleCard"),
                    ("HUMO Card", 4, builder.CORE_TO_HUMO, "_styleHumo"),
                    ("Все транзакции", 5, builder.CORE_TO_CARD, "_styleCard")):
                sheet, ref, proto = book[name], original["CardXabar" if name == "Все транзакции" else name], book[style_name]
                # Saved views are hidden; only an activated computer reveals them.
                self.assertEqual(sheet.sheet_state, "veryHidden")
                self.assertEqual(sheet["P2"].value, 1)
                self.assertEqual(sheet["R2"].value, "(Без дубля)")
                for col in range(15, mapping[40] + 1):
                    self.assertEqual(dimension(sheet, col), dimension(ref, col), (name, col))
                for core_col in (17, 18, 26, 33, 39):
                    physical = mapping[core_col]
                    self.assertEqual(sheet.cell(first, physical).number_format, ref.cell(first, physical).number_format)
                for core_col in (38, 39, 40):
                    self.assertEqual(copy(sheet.cell(2, mapping[core_col]).fill), copy(ref.cell(2, mapping[38]).fill))
                self.assertEqual(sheet.cell(2, mapping[50]).value, "Состояние / действие")
                for col in range(15, mapping[53] + 1):
                    cell = sheet.cell(first, col)
                    self.assertIsNone(cell.value, (name, cell.coordinate))
                    self.assertTrue(cell.protection.locked)
                    self.assertEqual(cell._style, proto.cell(5, col)._style)
                    reference_column = col if col <= mapping[40] else mapping[37]
                    self.assertEqual(cell.font.sz, ref.cell(first, reference_column).font.sz, (name, col))
                self.assertEqual(sheet.row_dimensions[first].height, 12)
                self.assertEqual(proto.row_dimensions[5].height, 12)
                self.assertEqual(sheet["P3"].value, f'=COUNT(Q{first}:Q1048576)')
                for row in sheet:
                    for cell in row:
                        self.assertNotEqual(cell.data_type, 'e')
                        if cell.row >= first: self.assertIsNone(cell.value)
                for core_col in (51, 52, 53):
                    self.assertTrue(dimension(sheet, mapping[core_col])[1])
                self.assertEqual(sheet.sheet_view.showZeros, ref.sheet_view.showZeros)
                self.assertEqual(sheet.sheet_view.pane, ref.sheet_view.pane)
            self.assertEqual(builder.CORE_TO_HUMO[26], 32)
            self.assertEqual(builder.CORE_TO_HUMO[33], 40)
            self.assertEqual(builder.CORE_TO_HUMO[36], 43)
            self.assertEqual(builder.CORE_TO_HUMO[52], 59)
            for col in (26, 28, 29, 30, 31, 34, 36, 37, 38, 39):
                self.assertEqual(book['HUMO Card'].cell(4, col).number_format, '@')
                self.assertEqual(book['_styleHumo'].cell(5, col).number_format, '@')
            book.close()
            original.close()

    def test_new_shell_preserves_layout_and_cannot_edit_without_macros(self):
        before = hashlib.sha256(builder.TEMPLATE.read_bytes()).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "shell.xlsx"
            builder.prepare_shell(target)
            book = openpyxl.load_workbook(io.BytesIO(target.read_bytes()))  # never lock the temp file
            original = openpyxl.load_workbook(builder.TEMPLATE)
            sheet = book["_transactions"]
            self.assertEqual(book.sheetnames, ["Доступ", "CardXabar", "HUMO Card", "Все транзакции", "_transactions", "_style", "_sync", "_refs", "_styleCard", "_styleHumo"])
            self.assertEqual([s.title for s in book if s.sheet_state == "visible"], ["Доступ"])
            self.assertEqual(book.active.title, "Доступ")
            lock = book["Доступ"]
            self.assertEqual(lock["A1"].value, "Чеки защищены")
            self.assertTrue(lock["A1"].font.b)
            self.assertIn("паролем владельца", lock["A3"].value)
            self.assertIn("дважды щёлкните", lock["A4"].value)
            self.assertIsNone(lock["A6"].value)
            self.assertTrue(lock.protection.sheet)
            self.assertEqual([s.title for s in book if s.sheet_view.tabSelected], ["Доступ"])
            for name in ("CardXabar", "HUMO Card", "Все транзакции"):
                self.assertEqual(book[name].sheet_state, "veryHidden")
            vba = (builder.SOURCES / "ExcelSync.bas").read_text(encoding="ascii")
            escaped = "".join(r"\u%04x" % ord(c) for c in builder.LOCK_SHEET)
            self.assertIn(f'LockSheetName = Ui("{escaped}")', vba)
            self.assertEqual(sheet.sheet_state, "veryHidden")
            for name in ("_style", "_sync", "_refs", "_styleCard", "_styleHumo"):
                self.assertEqual(book[name].sheet_state, "veryHidden")
                self.assertTrue(book[name].protection.sheet)
            self.assertTrue(book.security.lockStructure)
            self.assertEqual({str(merged) for merged in sheet.merged_cells.ranges},
                             {"B2:C2", "AI2:AJ2", "O1:S1", "T1:AN1", "O4:AN4"})
            self.assertEqual(sheet["O1"].value, "Все транзакции")
            self.assertEqual(sheet["T1"].value, "Подключение настроится автоматически")
            self.assertEqual(sheet["O4"].value,
                             "Новый чек: заполните строку внизу. Дважды нажмите статус строки для действий.")
            for cell in ("O1", "T1", "O4"):
                self.assertFalse(sheet[cell].alignment.wrap_text)
                self.assertEqual(sheet[cell].number_format, "@")
                self.assertTrue(sheet[cell].protection.locked)
            self.assertEqual(sheet.row_dimensions[1].height, original["CardXabar"].row_dimensions[1].height)
            self.assertEqual(sheet.row_dimensions[4].height, 12)
            self.assertEqual(sheet.row_dimensions[5].height, 12)
            self.assertEqual(book["_style"].row_dimensions[5].height, 12)
            self.assertEqual(sheet["AX2"].value, "Состояние / действие")
            self.assertEqual(sheet.column_dimensions["AX"].width, 32)
            self.assertEqual(sheet["Q2"].value, original["CardXabar"]["Q2"].value)
            self.assertEqual(sheet["AK2"].value, original["CardXabar"]["AK2"].value)
            for col in ("O", "U", "AH", "AK", "AN"):
                self.assertEqual(sheet.column_dimensions[col].width, original["CardXabar"].column_dimensions[col].width)
            for col, width, number_format in (("Z", 20, "#,##0.00"), ("AG", 20, "#,##0.00"), ("AM", 12, "mmm yyyy")):
                self.assertEqual(sheet.column_dimensions[col].width, width)
                self.assertEqual(sheet[f"{col}5"].number_format, number_format)
                self.assertEqual(book["_style"][f"{col}5"].number_format, number_format)
            for col, minimum_width, number_format in (("Q", 23, "dd.mm.yyyy hh:mm:ss"), ("R", 13, "dd.mm.yyyy")):
                self.assertGreaterEqual(sheet.column_dimensions[col].width, minimum_width)
                self.assertEqual(sheet[f"{col}5"].number_format, number_format)
                # RenderGrid copies this row's styles for every fetched record.
                self.assertEqual(book["_style"][f"{col}5"].number_format, number_format)
            self.assertEqual(len(sheet.views.sheetView), 1)
            view = sheet.sheet_view
            self.assertEqual((view.topLeftCell, view.zoomScale), ("O1", 100))
            self.assertIsNone(view.pane.xSplit)
            self.assertEqual((view.pane.ySplit, view.pane.topLeftCell, view.pane.state), (4, "O5", "frozen"))
            self.assertEqual([(selection.pane, selection.activeCell, selection.sqref) for selection in view.selection],
                             [("bottomLeft", "O5", "O5")])
            self.assertTrue(all(sheet.column_dimensions[openpyxl.utils.get_column_letter(col)].hidden for col in range(1, 15)))
            self.assertEqual(sheet["Z5"].number_format, "#,##0.00")
            self.assertTrue(sheet.protection.sheet)
            for cell in ("AK5", "AY5", "AZ5", "BA5"):
                self.assertTrue(sheet[cell].protection.locked)
            for col in ("AY", "AZ", "BA"):
                self.assertTrue(sheet.column_dimensions[col].hidden)
            for cell in ("U5", "AK5", "AR5", "AY5"):
                self.assertEqual(book["_style"][cell].number_format, "@")
            compact_columns = {"S", "T", "U", "W", "AK", "AL", "AN", "AP", "AQ", "AR", "AS", "AT", "AU", "AV", "AX"}
            self.assertEqual(set(builder.COMPACT_TEXT_COLUMNS), compact_columns)
            for layout_sheet in (sheet, book["_style"]):
                for cell in layout_sheet[5][14:53]:
                    clip_text = cell.column_letter in compact_columns
                    self.assertEqual(bool(cell.alignment.wrap_text), clip_text, cell.coordinate)
                    if clip_text:
                        self.assertEqual(cell.alignment.vertical, "top", cell.coordinate)
                        self.assertFalse(cell.alignment.shrink_to_fit, cell.coordinate)
                    reference = original["CardXabar"].cell(5, cell.column if cell.column < 41 else 37)
                    self.assertEqual(cell.font.sz, reference.font.sz, cell.coordinate)
                    self.assertEqual(cell.font.name, reference.font.name, cell.coordinate)
            for address in ("Q5", "R5"):
                self.assertEqual(sheet[address].alignment.indent, original["CardXabar"][address].alignment.indent)
            vba = (builder.SOURCES / "ExcelSync.bas").read_text(encoding="utf-8-sig")
            predicate = vba.split("Private Function IsCompactTextColumn", 1)[1].split("End Function", 1)[0]
            numeric_columns = set(map(int, re.search(r"Case ([\d, ]+)", predicate).group(1).split(",")))
            self.assertEqual(numeric_columns, {openpyxl.utils.column_index_from_string(col) for col in compact_columns})
            native = (builder.SOURCES / "build_native.ps1").read_text(encoding="utf-8-sig")
            self.assertIn("BankViews.bas", native)
            self.assertNotIn("ColumnWidth", native)  # Both paths consume the same prepared layout.
            self.assertIsNone(book["_sync"]["B1"].value)
            self.assertEqual([(book["_sync"].cell(row, 1).value, book["_sync"].cell(row, 2).value)
                              for row in range(1, 11)],
                             [("workbook_id", None), ("protocol", "1"), ("data_sheet", sheet.title),
                              ("source_version", "1"), ("server_url", None), ("shared_token", None),
                              ("owner_id", None), ("shared_mode", "1"), ("status", None), ("active_view", None)])
            for row in range(1, 11):
                self.assertEqual(book["_sync"].cell(row, 2).number_format, "@")
            self.assertIsNone(sheet["Q5"].value)
            self.assertTrue(all(cell.value is None for row in sheet.iter_rows(min_row=5, min_col=15, max_col=53) for cell in row))
            self.assertEqual([sheet[c].value for c in ("AL2", "AM2", "AN2")], ["Лицо", "Период", "Статья"])
            self.assertIsNone(sheet["AU3"].value)
            self.assertIsNone(sheet["AV3"].value)
            self.assertTrue(all(cell.value is None for row in sheet.iter_rows(min_row=1, max_row=4, max_col=14) for cell in row))
            self.assertTrue(all(cell.value is None for cell in sheet[4] if cell.coordinate != "O4"))
            self.assertEqual(sheet["R2"].value, "Дата")
            self.assertEqual(sheet["W3"].value, "Оборот UZS")
            self.assertEqual(sheet["Z3"].value, '=SUMIF(AO5:AO1048576,"UZS",Z5:Z1048576)')
            self.assertEqual(sheet["Z3"].number_format, '#,##0.00" UZS"')
            self.assertEqual(sheet["AO3"].number_format, '#,##0.00" USD"')
            self.assertEqual({cell.coordinate: cell.value for row in sheet for cell in row if cell.data_type == "f"},
                             {"P3": '=COUNT(Q5:Q1048576)',
                              "T3": '=IF(COUNT(Q5:Q1048576)=0,"",MAX(Q5:Q1048576))',
                              "Z3": '=SUMIF(AO5:AO1048576,"UZS",Z5:Z1048576)',
                              "AO3": '=SUMIF(AO5:AO1048576,"USD",Z5:Z1048576)'})
            with zipfile.ZipFile(target) as package:
                names = package.namelist()
                self.assertFalse(any("queryTable" in name or "connection" in name or "externalLink" in name for name in names))
                xml = "\n".join(package.read(name).decode("utf-8") for name in names if name.endswith(".xml"))
                self.assertNotIn("Bearer ", xml)
                self.assertNotIn("X-Export-Key", xml)
            book.close()
            original.close()
        self.assertEqual(hashlib.sha256(builder.TEMPLATE.read_bytes()).hexdigest(), before)


if __name__ == "__main__":
    unittest.main()
