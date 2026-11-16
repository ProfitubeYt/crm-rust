#!/usr/bin/env python3
"""
Cut the customer's workbook down to the template the export writes into.

The export reproduces the customer's own file — `Книга1.xlsx`, one sheet per bank
— and it does so by opening a pruned copy of that file and taking every style
from it: column widths, the hidden import group, tints, fonts, number formats,
borders, the frozen panes and the sheet view that hides zeros. Transcribing those
into code was tried twice and drifted twice; a hidden four-column group came out
as three visible blank columns because openpyxl reports a grouped range under
its first letter only. Keeping their file as the template removes that class
of mistake altogether.

This tool makes the copy. For each sheet it keeps the rows above the data plus
two data rows — the first carries the thin top border their block opens with,
the second is the style every following row inherits — and deletes everything
below. Their data rows are not uniform (a cell restyled by hand here and there,
one row given a height), so the two kept rows are settled first: each column
takes the style most of their rows have. It then reloads the result and checks
every kept cell, column dimension, view, merge and conditional format against
the source, so a template that lost something is never committed by accident.

    python tools/build_excel_client_template.py "~/Downloads/Telegram Desktop/Книга1.xlsx"

Run it again whenever the customer sends a new version of their workbook.
"""
from __future__ import annotations

import argparse
import sys
from collections import Counter
from copy import copy
from pathlib import Path
from typing import List

BACKEND_ROOT = Path(__file__).resolve().parents[1]
if str(BACKEND_ROOT) not in sys.path:
    sys.path.insert(0, str(BACKEND_ROOT))

import openpyxl  # noqa: E402
from openpyxl.styles import Border  # noqa: E402
from openpyxl.utils import get_column_letter  # noqa: E402

from services.excel_client_layout import SHEET_TEMPLATES, TEMPLATE_PATH, SheetTemplate  # noqa: E402

#: The column that says whether a row of their block is a receipt: `Сервис`.
SOURCE_COLUMN = 19


def _color(value):
    if value is None:
        return None
    key = value.rgb if value.type == "rgb" else value.theme if value.type == "theme" else value.indexed
    return (value.type, key, round(value.tint or 0, 3))


def _cell_signature(cell):
    font, fill, border, align = cell.font, cell.fill, cell.border, cell.alignment
    return (
        font.name, font.sz, font.b, font.i, font.u, _color(font.color),
        fill.fill_type,
        _color(fill.fgColor) if fill.fill_type else None,
        _color(fill.bgColor) if fill.fill_type else None,
        tuple((getattr(border, side).style, _color(getattr(border, side).color))
              for side in ("left", "right", "top", "bottom")),
        cell.number_format,
        align.horizontal, align.vertical, align.wrap_text, align.indent,
        align.shrink_to_fit, align.text_rotation,
        cell.protection.locked, cell.protection.hidden,
    )


def _without_borders(signature):
    return signature[:9] + signature[10:]


def _column_signatures(worksheet):
    """Per-column dimension attributes, with grouped ranges expanded."""
    out = {}
    for dim in worksheet.column_dimensions.values():
        signature = (
            dim.min, dim.max, dim.width, dim.hidden, dim.outlineLevel, dim.bestFit, dim.collapsed,
            dim.font.name, dim.font.sz, dim.fill.fill_type,
            _color(dim.fill.fgColor) if dim.fill.fill_type else None,
            dim.number_format, dim.alignment.horizontal, dim.alignment.vertical,
            tuple(getattr(dim.border, side).style for side in ("left", "right", "top", "bottom")),
        )
        for index in range(dim.min or 0, min(dim.max or 0, 64) + 1):
            out[index] = signature
    return out


def _conditional_formats(worksheet):
    out = []
    for cf in worksheet.conditional_formatting:
        for rule in cf.rules:
            fill = rule.dxf.fill.bgColor.rgb if rule.dxf is not None and rule.dxf.fill is not None else None
            out.append((str(cf.sqref), rule.type, tuple(rule.formula or ()), rule.priority, str(fill)))
    return sorted(out)


def _receipt_rows(worksheet, template: SheetTemplate) -> List[int]:
    """The rows of their block that hold a receipt, not a formula's leftover zero."""
    rows = []
    for row in range(template.first_row, worksheet.max_row + 1):
        value = worksheet.cell(row, SOURCE_COLUMN).value
        if isinstance(value, str) and value.strip():
            rows.append(row)
    return rows


def settle_data_rows(worksheet, template: SheetTemplate) -> None:
    """
    Give the two kept data rows the styles the export should spread over every row.

    Their receipts are not styled uniformly — a cell restyled by hand here and
    there — so the body row takes, column by column, the style most of their
    receipts have. The first row keeps its own style when it differs from that
    only by its borders (the top edge of the block is a thin rule); when it
    differs in more than that, it takes the majority style but keeps its own
    top border, which is the one part of it that is design.
    """
    rows = _receipt_rows(worksheet, template)
    body_rows = [row for row in rows if row != template.first_row] or rows
    for column in range(1, worksheet.max_column + 1):
        signatures = {row: _cell_signature(worksheet.cell(row, column)) for row in body_rows}
        majority, _ = Counter(signatures.values()).most_common(1)[0]
        donor = worksheet.cell(next(row for row in body_rows if signatures[row] == majority), column)
        worksheet.cell(template.body_row, column)._style = copy(donor._style)

        first = worksheet.cell(template.first_row, column)
        if _without_borders(_cell_signature(first)) == _without_borders(majority):
            continue
        top = copy(first.border.top)
        first._style = copy(donor._style)
        first.border = Border(left=donor.border.left, right=donor.border.right, bottom=donor.border.bottom, top=top)


def build(source_path: Path, template_path: Path) -> None:
    workbook = openpyxl.load_workbook(source_path)
    for worksheet in workbook.worksheets:
        template = SHEET_TEMPLATES.get(worksheet.title)
        if template is None:
            raise SystemExit(
                f"Sheet {worksheet.title!r} has no entry in SHEET_TEMPLATES; "
                "describe it in services/excel_client_layout.py first."
            )
        settle_data_rows(worksheet, template)
        # A height set by hand on one of their data rows is not the design.
        for row in (template.first_row, template.body_row):
            if row in worksheet.row_dimensions:
                del worksheet.row_dimensions[row]

        keep = template.body_row
        worksheet.delete_rows(keep + 1, max(worksheet.max_row - keep, 1) + 1)
        for row in list(worksheet.row_dimensions.keys()):
            if row > keep:
                del worksheet.row_dimensions[row]
        # Their notes on header cells explain their formulas ("works for the date
        # format in column A…"); an export has no formulas, and ExcelJS cannot
        # open a workbook that carries them.
        for row in worksheet.iter_rows(min_row=1, max_row=keep):
            for cell in row:
                if cell.comment is not None:  # merged cells cannot even be asked
                    cell.comment = None
        # openpyxl writes <sheetFormatPr> before it has looked at the columns,
        # so the outline depth that shows the group button is lost unless set.
        worksheet.column_dimensions.max_outline = max(
            (dim.outlineLevel or 0) for dim in worksheet.column_dimensions.values()
        ) or None
    # A values-only workbook still remembers the workbook its formulas once read.
    workbook._external_links = []
    workbook.save(template_path)


def verify(source_path: Path, template_path: Path) -> int:
    """Compare the kept part of the template with the source; return the number of differences."""
    source = openpyxl.load_workbook(source_path)
    template = openpyxl.load_workbook(template_path)
    problems = 0

    def report(message):
        nonlocal problems
        problems += 1
        print("  " + message)

    if source.loaded_theme != template.loaded_theme:
        report("theme differs")
    if source.sheetnames != template.sheetnames:
        report(f"sheets: {source.sheetnames} -> {template.sheetnames}")
    if template._external_links:
        report("template still links to an external workbook")
    for worksheet in template.worksheets:
        noted = [cell.coordinate for row in worksheet.iter_rows() for cell in row if cell.comment is not None]
        if noted:
            report(f"{worksheet.title} still carries notes on {noted}")

    for name in source.sheetnames:
        ws_source, ws_template = source[name], template[name]
        sheet = SHEET_TEMPLATES[name]
        print(f"{name}: kept rows 1..{sheet.body_row}")
        for row in range(1, sheet.header_rows + 1):
            for column in range(1, ws_source.max_column + 1):
                a, b = ws_source.cell(row, column), ws_template.cell(row, column)
                if a.value != b.value:
                    report(f"value {a.coordinate}: {a.value!r} -> {b.value!r}")
                if _cell_signature(a) != _cell_signature(b):
                    report(f"style {a.coordinate}: {_cell_signature(a)} -> {_cell_signature(b)}")
        # Settle the source the same way and the two data rows must match it.
        settle_data_rows(ws_source, sheet)
        for row in (sheet.first_row, sheet.body_row):
            for column in range(1, ws_source.max_column + 1):
                a, b = ws_source.cell(row, column), ws_template.cell(row, column)
                if _cell_signature(a) != _cell_signature(b):
                    report(f"style {b.coordinate}: {_cell_signature(a)} -> {_cell_signature(b)}")
        cols_source, cols_template = _column_signatures(ws_source), _column_signatures(ws_template)
        for index in sorted(set(cols_source) | set(cols_template)):
            if cols_source.get(index) != cols_template.get(index):
                report(f"column {get_column_letter(index)}: {cols_source.get(index)} -> {cols_template.get(index)}")
        for row in range(1, sheet.header_rows + 1):
            a, b = ws_source.row_dimensions.get(row), ws_template.row_dimensions.get(row)
            if (a and (a.height, a.hidden)) != (b and (b.height, b.hidden)):
                report(f"row {row} dimension differs")
        for row in (sheet.first_row, sheet.body_row):
            dim = ws_template.row_dimensions.get(row)
            if dim is not None and dim.height:
                report(f"row {row} kept a height of its own")
        if ws_template.max_row > sheet.body_row:
            report(f"template still has {ws_template.max_row} rows")

        view_source, view_template = ws_source.sheet_view, ws_template.sheet_view
        for attr in ("showZeros", "topLeftCell", "zoomScale", "showGridLines"):
            if getattr(view_source, attr) != getattr(view_template, attr):
                report(f"view {attr}: {getattr(view_source, attr)} -> {getattr(view_template, attr)}")
        if ws_source.freeze_panes != ws_template.freeze_panes:
            report(f"freeze: {ws_source.freeze_panes} -> {ws_template.freeze_panes}")
        if ws_source.sheet_format.__dict__ != ws_template.sheet_format.__dict__:
            report(f"sheet format: {ws_source.sheet_format.__dict__} -> {ws_template.sheet_format.__dict__}")
        if ws_source.sheet_properties.outlinePr.__dict__ != ws_template.sheet_properties.outlinePr.__dict__:
            report("outline properties differ")
        if set(map(str, ws_source.merged_cells.ranges)) != set(map(str, ws_template.merged_cells.ranges)):
            report(f"merged: {ws_source.merged_cells.ranges} -> {ws_template.merged_cells.ranges}")
        if _conditional_formats(ws_source) != _conditional_formats(ws_template):
            report(f"conditional formats: {_conditional_formats(ws_source)} -> {_conditional_formats(ws_template)}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", help="the customer's workbook (Книга1.xlsx)")
    parser.add_argument("--out", default=str(TEMPLATE_PATH), help=f"where to write the template (default: {TEMPLATE_PATH})")
    args = parser.parse_args()

    source_path = Path(args.source).expanduser()
    template_path = Path(args.out)
    build(source_path, template_path)
    print(f"wrote {template_path} ({template_path.stat().st_size} bytes)")
    problems = verify(source_path, template_path)
    if problems:
        print(f"{problems} difference(s) between the template and the source")
        return 1
    print("template matches the source on every kept cell, column, view, merge and conditional format")
    return 0


if __name__ == "__main__":
    sys.exit(main())
