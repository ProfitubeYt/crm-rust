"""
The customer's workbook, asserted against the copy of their file the export writes into.

The export is judged by opening it next to `Книга1.xlsx`. What these tests hold
on to is exactly what a side-by-side look would catch: the same columns at the
same letters, the same hidden groups, their header rows verbatim with our counts
in them, zeros hidden, the timestamp made unique for duplicates, the second
numbering column running across sheets, and the bank's own words in the cells.
"""
# ruff: noqa: F811  (the `client` fixture is imported, then named as a test argument)
from datetime import date, datetime, timedelta
import csv
from decimal import Decimal
from io import BytesIO

import openpyxl
import pytest

# The authenticated TestClient and the seed helpers already exist next door; the
# endpoint tests below reuse them rather than standing up a second rig.
from tests.test_transactions_export import (  # noqa: F401  (client is a fixture)
    _seed_monitored_chat,
    _seed_tx,
    client,
)

from services.excel_client_layout import (
    DEDUP_STEP,
    block_columns,
    SHEET_TEMPLATES,
    TEMPLATE_PATH,
    ClientWorkbookWriter,
    amount_text_from_raw,
    balance_text_from_raw,
    build_row_values,
    digit_groups,
    direction_sign,
    format_balance,
    format_money,
    import_lines,
    operation_from_raw,
    operation_label,
    order_sources,
    sanitize_sheet_name,
    style_for_source,
    template_for_source,
    whole_amount,
)

CARDXABAR = SHEET_TEMPLATES["CardXabar"]
HUMO = SHEET_TEMPLATES["HUMO Card"]

#: Careful: this is the order the customer's *sheet* shows, which is already
#: sorted by their formulas. What the bot actually sends puts the amount second
#: and the merchant fourth — see CARDXABAR_SMS_AS_SENT.
CARDXABAR_SMS = (
    "🔴 E-Com oplata\n📍 PLUM OPLATA, UZ\n➖ 5 000.00 UZS\n💳 ***4483\n"
    "🕓 01.01.26 07:16\n💵 4 229 872.82 UZS"
)
#: The shape 975 of the 1089 CardXabar receipts on production actually have.
CARDXABAR_SMS_AS_SENT = (
    "🔴 E-Com oplata\n➖ 5 000.00 UZS\n💳 ***4483\n📍 PLUM OPLATA, UZ\n"
    "🕓 01.01.26 07:16\n💵 4 229 872.82 UZS"
)
HUMO_SMS = (
    "💸 Оплата\n➖ 40.000,00 UZS\n📍 HUSNORABONU GOLD>Urt\n💳 HUMOCARD *4428\n"
    "🕓 04.01.2026 11:24\n💰 14.914,00 UZS"
)


def _row(**overrides):
    values = dict(
        transaction_date=datetime(2026, 1, 1, 7, 16),
        source_display="CardXabar",
        transaction_type="DEBIT",
        type_display="Списание",
        operator_raw="PLUM OPLATA, UZ",
        application_mapped="",
        amount=Decimal("-5000.00"),
        balance_after=Decimal("4229872.82"),
        currency="UZS",
        card_last_4="4483",
        is_p2p=False,
        annotation_details="услуги мониторинга",
        annotation_person_code="СТ",
        annotation_period=date(2025, 12, 1),
        annotation_article_code="р1",
        article_labels={"р1": "ФЗП,ФМП,МП"},
        raw_message=CARDXABAR_SMS,
    )
    values.update(overrides)
    return build_row_values(**values)


def _workbook(sheets, **writer_kwargs):
    """Write the given (source, rows) pairs and read the result back."""
    writer = ClientWorkbookWriter(**writer_kwargs)
    for source, rows in sheets:
        writer.add_sheet(source, rows)
    stream = BytesIO()
    writer.save(stream)
    stream.seek(0)
    return openpyxl.load_workbook(stream)


def _style(cell):
    fill = cell.fill
    return (
        cell.font.name, cell.font.sz, cell.font.b, cell.font.i,
        fill.fill_type, (fill.fgColor.rgb, fill.fgColor.theme, round(fill.fgColor.tint or 0, 2)) if fill.fill_type else None,
        tuple(getattr(cell.border, side).style for side in ("left", "right", "top", "bottom")),
        cell.number_format, cell.alignment.horizontal, cell.alignment.indent,
    )


def _hidden_columns(sheet):
    hidden = set()
    for dim in sheet.column_dimensions.values():
        if dim.hidden:
            hidden.update(range(dim.min, dim.max + 1))
    return {openpyxl.utils.get_column_letter(index) for index in hidden}


@pytest.fixture(scope="module")
def template():
    return openpyxl.load_workbook(TEMPLATE_PATH)


class TestTheTemplateIsTheirFile:
    def test_it_ships_with_both_of_their_sheets(self, template):
        assert template.sheetnames == ["CardXabar", "HUMO Card"]

    def test_their_import_group_and_helpers_are_hidden(self, template):
        hidden = _hidden_columns(template["CardXabar"])
        assert {"A", "N", "Y", "AA", "AB", "AC", "AD", "AF"} <= hidden
        assert not {"O", "Z", "AE", "AG", "AK"} & hidden

    def test_humo_hides_its_digit_helpers(self, template):
        hidden = _hidden_columns(template["HUMO Card"])
        assert {"Y", "Z", "AA", "AE", "AG", "AM"} <= hidden
        assert not {"AF", "AN", "AO", "AR"} & hidden

    def test_zeros_are_hidden_by_the_view(self, template):
        assert template["CardXabar"].sheet_view.showZeros is False
        assert template["HUMO Card"].sheet_view.showZeros is False

    def test_the_template_holds_only_the_rows_the_writer_reads(self, template):
        assert template["CardXabar"].max_row == CARDXABAR.body_row
        assert template["HUMO Card"].max_row == HUMO.body_row


class TestSheetShape:
    """One CardXabar sheet, compared with the template it was written from."""

    @pytest.fixture(scope="class")
    def book(self):
        rows = [
            _row(),
            _row(transaction_date=datetime(2026, 1, 1, 9, 50), amount=Decimal("-45000.00"),
                 balance_after=Decimal("975512.77"), operator_raw="VIA PAYMENT, UZ"),
            _row(transaction_date=datetime(2026, 1, 2, 0, 57), amount=Decimal("-600000.00"),
                 application_mapped="Chakanapay", is_p2p=True),
        ]
        return _workbook([("CardXabar", rows)])

    def test_the_sheet_is_named_after_the_source(self, book):
        assert book.sheetnames == ["CardXabar"]

    def test_column_dimensions_are_theirs(self, book, template):
        ours, theirs = book["CardXabar"], template["CardXabar"]
        for letter, dim in theirs.column_dimensions.items():
            mine = ours.column_dimensions[letter]
            assert (mine.width, mine.hidden, mine.outlineLevel, mine.min, mine.max) == (
                dim.width, dim.hidden, dim.outlineLevel, dim.min, dim.max
            ), letter
        assert ours.sheet_format.outlineLevelCol == 1
        assert ours.sheet_format.defaultRowHeight == 12.0

    def test_the_view_is_theirs(self, book):
        sheet = book["CardXabar"]
        assert sheet.freeze_panes == "Q5"
        assert sheet.sheet_view.showZeros is False
        assert sheet.sheet_view.topLeftCell == "O1"
        assert sheet.row_dimensions[1].height == 15.75

    def test_merged_cells_and_conditional_formats_come_along(self, book):
        sheet = book["CardXabar"]
        assert {"B2:C2", "AI2:AJ2", "AL2:AM2"} == set(map(str, sheet.merged_cells.ranges))
        rules = [(str(cf.sqref), rule.type) for cf in sheet.conditional_formatting for rule in cf.rules]
        assert rules == [("AU3", "expression")]

    def test_the_header_rows_are_copied_verbatim(self, book, template):
        ours, theirs = book["CardXabar"], template["CardXabar"]
        for row in range(1, CARDXABAR.header_rows + 1):
            for column in range(1, theirs.max_column + 1):
                a, b = theirs.cell(row, column), ours.cell(row, column)
                if a.coordinate in CARDXABAR.figures:
                    continue
                assert b.value == a.value, a.coordinate
                assert _style(b) == _style(a), a.coordinate

    def test_the_figures_are_ours(self, book):
        sheet = book["CardXabar"]
        assert sheet["P3"].value == 3
        assert sheet["B1"].value == 3
        assert sheet["AU3"].value == 3
        assert sheet["T3"].value == datetime(2026, 1, 2, 0, 57)
        assert sheet["Z3"].value == 5000 + 45000 + 600000

    def test_their_row_of_zeros_stays_and_data_starts_below_it(self, book):
        sheet = book["CardXabar"]
        assert sheet["O4"].value == 0
        assert sheet["O5"].value == 1
        assert sheet["O7"].value == 3

    def test_data_rows_take_the_template_styles(self, book, template):
        ours, theirs = book["CardXabar"], template["CardXabar"]
        for column in range(1, theirs.max_column + 1):
            assert _style(ours.cell(5, column)) == _style(theirs.cell(CARDXABAR.first_row, column)), column
            assert _style(ours.cell(6, column)) == _style(theirs.cell(CARDXABAR.body_row, column)), column
            assert _style(ours.cell(7, column)) == _style(theirs.cell(CARDXABAR.body_row, column)), column

    def test_values_land_in_their_columns(self, book):
        sheet = book["CardXabar"]
        assert sheet["Q5"].value == datetime(2026, 1, 1, 7, 16)
        assert sheet["R5"].value == datetime(2026, 1, 1, 7, 16)
        assert sheet["S5"].value == "CardXabar"
        assert sheet["T5"].value == "🔴 E-Com oplata"
        assert sheet["U5"].value == "PLUM OPLATA, UZ"
        assert sheet["W5"].value == 0
        assert sheet["X5"].value == "-"
        assert sheet["Y5"].value == "➖ 5 000.00 UZS"
        assert sheet["Z5"].value == 5000
        assert sheet["AF5"].value == "💵 4 229 872.82 UZS"
        assert sheet["AG5"].value == 4229872
        assert sheet["AH5"].value == "4483"
        assert sheet["AI5"].value == 0
        assert sheet["AJ5"].value is None
        assert sheet["AK5"].value == "услуги мониторинга"
        assert sheet["AL5"].value == "СТ"
        assert sheet["AM5"].value == datetime(2025, 12, 1)
        assert sheet["AN5"].value == "р1=ФЗП,ФМП,МП"

    def test_p2p_fills_both_of_their_columns(self, book):
        sheet = book["CardXabar"]
        assert sheet["AI7"].value == 1
        assert sheet["AJ7"].value == 1
        assert sheet["W7"].value == "Chakanapay"

    def test_only_the_first_sheet_is_selected(self):
        book = _workbook([("CardXabar", [_row()]), ("HUMO Card", [_row(source_display="HUMO Card")])])
        assert [sheet.sheet_view.tabSelected for sheet in book.worksheets] == [True, False]


class TestHumoSheet:
    @pytest.fixture(scope="class")
    def book(self):
        rows = [
            _row(source_display="HUMO Card", transaction_date=datetime(2026, 1, 4, 11, 24),
                 amount=Decimal("-40000.00"), balance_after=Decimal("14914.00"), card_last_4="4428",
                 operator_raw="HUSNORABONU GOLD>Urt", raw_message=HUMO_SMS),
            _row(source_display="HUMO Card", transaction_date=datetime(2026, 1, 5),
                 transaction_type="CREDIT", amount=Decimal("2257419.87"), balance_after=None,
                 raw_message="ℹ️ Счет по карте изменен\n➕ 2.257.419,87 UZS"),
        ]
        return _workbook([("HUMO Card", rows)])

    def test_it_uses_their_humo_sheet(self, book):
        sheet = book["HUMO Card"]
        assert sheet["O1"].value == "Сборка HumoCard"
        assert sheet["Q3"].value == "- число операций HumoCard"
        assert sheet["P3"].value == 2
        assert sheet.freeze_panes == "Q4"
        assert sheet["AR2"].value == "Подробности"
        assert sheet["Y2"].value == "Сумма операции"

    def test_data_starts_right_under_the_counts_row(self, book):
        sheet = book["HUMO Card"]
        assert sheet["O4"].value == 1
        assert sheet["T4"].value == "💸 Оплата"
        assert sheet["Y4"].value == "➖ 40.000,00 UZS"
        assert sheet["AF4"].value == 40000
        assert sheet["AG4"].value == "💰 14.914,00 UZS"
        assert sheet["AN4"].value == 14914
        assert sheet["AO4"].value == "4428"

    def test_their_digit_helpers_are_filled(self, book):
        sheet = book["HUMO Card"]
        assert [sheet.cell(4, c).value for c in range(26, 32)] == [" 40.000", 7, 0, 0, 40, "000"]
        assert [sheet.cell(4, c).value for c in range(34, 40)] == ["14.914", 6, 0, 0, 14, "914"]
        assert [sheet.cell(5, c).value for c in range(26, 32)] == [" 2.257.419", 10, 0, 2, "257", "419"]

    def test_a_missing_balance_is_a_row_of_zeros_as_their_formula_leaves_it(self, book):
        sheet = book["HUMO Card"]
        assert [sheet.cell(5, c).value for c in range(33, 41)] == [0] * 8

    def test_data_rows_take_the_template_styles(self, book, template):
        ours, theirs = book["HUMO Card"], template["HUMO Card"]
        for column in range(1, theirs.max_column + 1):
            assert _style(ours.cell(4, column)) == _style(theirs.cell(HUMO.first_row, column)), column
            assert _style(ours.cell(5, column)) == _style(theirs.cell(HUMO.body_row, column)), column


class TestOtherSources:
    def test_an_unknown_bank_gets_the_cardxabar_layout_under_its_own_name(self):
        book = _workbook([("Kapitalbank", [_row(source_display="Kapitalbank")])])
        sheet = book["Kapitalbank"]
        assert sheet["O1"].value == "Сборка Kapitalbank"
        assert sheet["Q3"].value == "- число операций Kapitalbank"
        assert sheet["U3"].value == "- дата крайней операции в Kapitalbank"
        assert sheet["S5"].value == "Kapitalbank"

    def test_humo_is_recognised_however_the_chat_is_titled(self):
        assert template_for_source("Humo card bot") is HUMO
        assert template_for_source("CardXabar") is CARDXABAR
        assert template_for_source("") is CARDXABAR

    def test_their_sheets_come_first_and_in_their_order(self):
        assert order_sources(["Kapital", "HUMO Card", "CardXabar", "Kapital"]) == ["CardXabar", "HUMO Card", "Kapital"]

    def test_a_workbook_without_rows_still_opens_on_their_sheet(self):
        book = _workbook([])
        assert book.sheetnames == ["CardXabar"]
        assert book["CardXabar"]["P3"].value == 0
        assert book["CardXabar"]["T3"].value is None


class TestTheCombinedSheet:
    """Every receipt on the first sheet, before the split by bank."""

    @pytest.fixture(scope="class")
    def book(self):
        card = [_row(), _row(transaction_date=datetime(2026, 1, 3, 9, 0))]
        humo = [_row(source_display="HUMO Card", transaction_date=datetime(2026, 1, 2, 12, 0), raw_message=HUMO_SMS)]
        writer = ClientWorkbookWriter()
        writer.add_combined_sheet([card[0], humo[0], card[1]])
        writer.add_sheet("CardXabar", card)
        writer.add_sheet("HUMO Card", humo)
        stream = BytesIO()
        writer.save(stream)
        stream.seek(0)
        return openpyxl.load_workbook(stream)

    def test_it_opens_the_workbook_and_the_banks_follow_in_their_order(self, book):
        assert book.sheetnames == ["Все транзакции", "CardXabar", "HUMO Card"]
        assert [sheet.sheet_view.tabSelected for sheet in book.worksheets] == [True, False, False]

    def test_it_is_laid_out_like_cardxabar_with_captions_of_its_own(self, book):
        sheet = book["Все транзакции"]
        assert sheet["O1"].value == "Все транзакции"
        assert sheet["Q3"].value == "- число операций, все сервисы"
        assert sheet["U3"].value == "- дата крайней операции"
        assert sheet["AK2"].value == "Подробности"
        assert sheet.freeze_panes == "Q5"

    def test_every_receipt_keeps_its_own_source(self, book):
        sheet = book["Все транзакции"]
        assert [sheet.cell(r, 19).value for r in (5, 6, 7)] == ["CardXabar", "HUMO Card", "CardXabar"]
        assert sheet["T6"].value == "💸 Оплата"
        assert sheet["P3"].value == 3
        assert sheet["T3"].value == datetime(2026, 1, 3, 9, 0)

    def test_its_numbering_does_not_shift_the_banks(self, book):
        assert [book["Все транзакции"].cell(r, 16).value for r in (5, 6, 7)] == [1, 2, 3]
        assert [book["CardXabar"].cell(r, 16).value for r in (5, 6)] == [1, 2]
        assert book["HUMO Card"]["P4"].value == 3


class TestNumbering:
    def test_the_second_column_continues_across_sheets(self):
        book = _workbook([
            ("CardXabar", [_row(), _row(), _row()]),
            ("HUMO Card", [_row(source_display="HUMO Card"), _row(source_display="HUMO Card")]),
        ])
        assert [book["CardXabar"].cell(r, 16).value for r in (5, 6, 7)] == [1, 2, 3]
        assert [book["HUMO Card"].cell(r, 16).value for r in (4, 5)] == [4, 5]
        assert [book["HUMO Card"].cell(r, 15).value for r in (4, 5)] == [1, 2]

    def test_newest_first_keeps_one_on_the_oldest_row(self):
        book = _workbook([("CardXabar", [_row(), _row(), _row()])], descending=True)
        assert [book["CardXabar"].cell(r, 16).value for r in (5, 6, 7)] == [3, 2, 1]


class TestBezDublya:
    def test_repeated_timestamps_are_nudged_apart(self):
        stamp = datetime(2026, 1, 7, 18, 55)
        book = _workbook([("CardXabar", [_row(transaction_date=stamp)] * 3 + [_row(transaction_date=stamp + timedelta(hours=1))])])
        sheet = book["CardXabar"]
        assert [sheet.cell(r, 17).value for r in (5, 6, 7)] == [stamp, stamp, stamp]
        assert sheet["R5"].value == stamp
        assert sheet["R6"].value == stamp + DEDUP_STEP
        assert sheet["R7"].value == stamp + 2 * DEDUP_STEP
        assert sheet["R8"].value == stamp + timedelta(hours=1)

    def test_the_step_is_their_ten_thousandth_of_a_day(self):
        assert DEDUP_STEP == timedelta(seconds=8.64)


class TestImportGroup:
    def test_the_message_is_laid_out_line_by_line_the_way_their_import_reads_it(self):
        lines = import_lines(_row(), 7)
        assert lines[0]["line"] == "CardXabar, [01.01.2026 7:16]"
        assert lines[0]["message_no"] == 7
        assert lines[0]["header_no"] == 7
        assert lines[0]["service"] == "CardXabar"
        assert lines[0]["date"] == datetime(2026, 1, 1)
        assert lines[0]["time"] == datetime(2026, 1, 1, 7, 16).time()
        assert lines[0]["line_1"] == "🔴 E-Com oplata"
        assert lines[0]["line_2"] == "📍 PLUM OPLATA, UZ"
        assert lines[0]["line_5"] == datetime(2026, 1, 1, 7, 16)
        assert lines[0]["line_6"] == "💵 4 229 872.82 UZS"
        assert lines[0]["weekday"] == 4
        assert [line["line"] for line in lines[1:]] == CARDXABAR_SMS.split("\n") + [None]
        assert all(line["message_no"] == 7 and line["header_no"] == 0 for line in lines[1:])

    def test_the_group_sits_beside_the_block_and_outlives_it(self):
        book = _workbook([("CardXabar", [_row(), _row(transaction_date=datetime(2026, 1, 1, 9, 50))])])
        sheet = book["CardXabar"]
        assert sheet["A5"].value == "CardXabar, [01.01.2026 7:16]"
        assert sheet["A6"].value == "🔴 E-Com oplata"
        assert sheet["O6"].value == 2
        # two receipts of eight lines each, next to two receipts of the block
        assert sheet.max_row == CARDXABAR.header_rows + 16
        assert sheet["A13"].value == "CardXabar, [01.01.2026 9:50]"
        assert sheet["B13"].value == 2
        assert sheet["O13"].value is None

    def test_the_group_can_be_left_empty(self):
        book = _workbook([("CardXabar", [_row(), _row()])], with_import_block=False)
        sheet = book["CardXabar"]
        assert sheet["A5"].value is None
        assert sheet.max_row == CARDXABAR.header_rows + 2

    def test_the_columns_follow_the_marker_not_the_line_order(self):
        """
        The banks do not agree on line order: CardXabar sends the card before the
        merchant, HUMO the merchant before the card, and NBU varies between its
        own messages. Filling their `G`..`L` by position put the amount under
        `Оператор/Продавец` on every real CardXabar receipt — and passed, because
        the fixture above was copied from their already-sorted sheet.
        """
        header = import_lines(_row(raw_message=CARDXABAR_SMS_AS_SENT), 1)[0]
        assert header["line_1"] == "🔴 E-Com oplata"
        assert header["line_2"] == "📍 PLUM OPLATA, UZ"      # Оператор/Продавец
        assert header["line_3"] == "➖ 5 000.00 UZS"          # Сумма
        assert header["line_4"] == "💳 ***4483"               # ПК
        assert header["line_6"] == "💵 4 229 872.82 UZS"      # Сумма остатка

    def test_their_humo_sheet_orders_those_columns_differently(self):
        """`Операция / Сумма / Оператор` there, `Операция / Оператор / Сумма` on
        CardXabar — so the role order belongs to the sheet, not to the message."""
        header = import_lines(
            _row(source_display="HUMO Card", raw_message=HUMO_SMS), 1, HUMO,
        )[0]
        assert header["line_2"] == "➖ 40.000,00 UZS"
        assert header["line_3"] == "📍 HUSNORABONU GOLD>Urt"

    def test_a_receipt_with_no_markers_keeps_its_lines_in_order(self):
        """An OCR dump has no `📍`; nothing may be silently dropped."""
        header = import_lines(_row(raw_message="Перевод\nОт кого\nКому\nСумма"), 1)[0]
        assert header["line_1"] == "Перевод"
        assert [header["line_2"], header["line_3"], header["line_4"]] == [
            "От кого", "Кому", "Сумма",
        ]

    def test_a_marked_receipt_leaves_an_unmatched_column_empty(self):
        """
        The clock line belongs to no column of their import group. Filling the
        gaps from whatever is left over put it under `Сумма остатка` on every
        receipt that carried no balance.
        """
        header = import_lines(_row(
            raw_message="🔴 Platezh\n➖ 5 000.00 UZS\n💳 ***4483\n"
                        "📍 PLUM OPLATA, UZ\n🕓 01.01.26 07:16",
        ), 1)[0]
        assert header["line_6"] == 0                      # Сумма остатка
        assert header["line_2"] == "📍 PLUM OPLATA, UZ"
        assert header["line_3"] == "➖ 5 000.00 UZS"

    def test_a_missing_role_is_a_zero_like_the_rest_of_their_empty_cells(self):
        header = import_lines(_row(raw_message="🔴 Platezh\n➖ 5 000.00 UZS"), 1)[0]
        assert header["line_3"] == "➖ 5 000.00 UZS"
        assert header["line_2"] == 0
        assert header["line_4"] == 0

    def test_a_line_over_excels_cell_limit_is_cut_rather_than_breaking_the_file(self):
        """The raw-value path for `=` skips openpyxl's own length check."""
        book = _workbook([("CardXabar", [_row(raw_message="=" + "x" * 40000)])])
        assert len(book["CardXabar"]["A6"].value) == 32767

    def test_a_line_that_is_an_excel_error_code_stays_text(self):
        book = _workbook([("CardXabar", [_row(raw_message="🔴 Platezh\n#N/A")])])
        cell = book["CardXabar"]["A7"]
        assert cell.value == "#N/A"
        assert cell.data_type == "s"

    def test_a_line_that_looks_like_a_formula_stays_text(self):
        book = _workbook([("CardXabar", [_row(raw_message="=====\n➖ 5 000.00 UZS")])])
        sheet = book["CardXabar"]
        assert sheet["A6"].value == "====="
        assert sheet["A6"].data_type == "s"


class TestDirectionSign:
    """Their unlabelled column between Приложение and Сумма."""

    def test_debit_is_minus(self):
        assert direction_sign("DEBIT") == "-"

    def test_credit_is_plus(self):
        assert direction_sign("CREDIT") == "+"

    def test_unknown_type_is_blank_not_a_guess(self):
        assert direction_sign("SOMETHING") == ""


class TestMoneyFormatting:
    def test_cardxabar_convention(self):
        style = style_for_source("CardXabar")
        assert format_money(Decimal("-5000.00"), "UZS", style) == "➖ 5 000.00 UZS"
        assert format_money(Decimal("1000000.5"), "UZS", style) == "➕ 1 000 000.50 UZS"

    def test_humo_uses_its_own_separators(self):
        style = style_for_source("HUMO Card")
        assert format_money(Decimal("-40000"), "UZS", style) == "➖ 40.000,00 UZS"
        assert format_balance(Decimal("14914"), "UZS", style) == "💰 14.914,00 UZS"

    def test_balance_carries_the_note_emoji_and_no_sign(self):
        style = style_for_source("CardXabar")
        assert format_balance(Decimal("-4229872.82"), "UZS", style) == "💵 4 229 872.82 UZS"

    def test_missing_values_render_empty_not_zero(self):
        style = style_for_source("CardXabar")
        assert format_money(None, "UZS", style) == ""
        assert format_balance(None, "UZS", style) == ""


class TestWholeAmounts:
    def test_the_number_column_is_unsigned_because_the_sign_column_carries_it(self):
        assert whole_amount(Decimal("-5000.00")) == 5000

    def test_the_fraction_is_dropped_the_way_their_formula_drops_it(self):
        assert whole_amount(Decimal("4229872.82")) == 4229872

    def test_nothing_stays_nothing(self):
        assert whole_amount(None) is None


class TestDigitGroups:
    def test_the_leading_group_is_a_number_and_the_rest_keep_their_zeros(self):
        assert digit_groups(40000, "amount", leading_space=True) == {
            "amount_digits": " 40.000", "amount_length": 7,
            "amount_billions": 0, "amount_millions": 0, "amount_thousands": 40, "amount_units": "000",
        }

    def test_billions_have_a_column_too(self):
        groups = digit_groups(2297918329, "amount", leading_space=True)
        assert (groups["amount_billions"], groups["amount_millions"], groups["amount_thousands"], groups["amount_units"]) == (2, "297", "918", "329")

    def test_nothing_is_a_row_of_zeros(self):
        groups = digit_groups(None, "balance", leading_space=False)
        assert set(groups.values()) == {0}


class TestDigitGroupsAtTheEdges:
    def test_a_number_past_their_last_group_does_not_take_the_export_down(self):
        """
        Their helpers stop at мрд. A trillion is a bad parse, and one bad row
        must not turn the whole export into a 500.
        """
        groups = digit_groups(1_234_567_890_123, "amount", leading_space=True)
        assert groups["amount_billions"] == "234"   # the fifth group is dropped, not fatal
        assert groups["amount_digits"] == " 1.234.567.890.123"


class TestOperationLabel:
    """For the receipts that arrive without a headline of their own."""

    def test_cardxabar_marks_everything_with_one_emoji(self):
        style = style_for_source("CardXabar")
        assert operation_label("DEBIT", "Списание", "", style) == "🔴 Списание"
        assert operation_label("CREDIT", "Пополнение", "", style) == "🔴 Пополнение"

    def test_humo_uses_a_per_type_emoji(self):
        style = style_for_source("HUMO Card")
        assert operation_label("DEBIT", "Оплата", "", style) == "💸 Оплата"
        assert operation_label("CREDIT", "Пополнение", "", style) == "🎉 Пополнение"

    def test_cash_withdrawal_gets_its_own_marker(self):
        style = style_for_source("HUMO Card")
        assert operation_label("DEBIT", "Снятие", "BXO ATM TASHKENT", style) == "🏧 Снятие"

    def test_a_balance_change_gets_their_information_marker(self):
        style = style_for_source("HUMO Card")
        assert operation_label("DEBIT", "Изменение счета", "", style) == "ℹ️ Изменение счета"

    def test_empty_label_stays_empty(self):
        assert operation_label("DEBIT", "", "", style_for_source("CardXabar")) == ""


class TestTheBanksOwnWords:
    def test_the_headline_is_taken_verbatim(self):
        assert operation_from_raw(CARDXABAR_SMS) == "🔴 E-Com oplata"

    def test_the_information_notice_keeps_the_banks_wording(self):
        """
        `ℹ️` is U+2139 plus an emoji-presentation selector, and U+2139 on its own
        is a *letter* to unicodedata — so a category test dropped exactly the
        line its own docstring named, on all 11 balance-change notices.
        """
        assert operation_from_raw("ℹ️ Счет по карте изменен\n➕ 2.257.419,87 UZS") == (
            "ℹ️ Счет по карте изменен"
        )

    def test_a_receipt_without_a_headline_falls_back_to_our_label(self):
        assert operation_from_raw("[Photo]\nПеревод через Payme") is None
        assert _row(raw_message="Перевод через Payme")["operation"] == "🔴 Списание"

    def test_a_field_line_is_not_a_headline(self):
        """
        `📍`, `➖` and `💳` are symbols too. A receipt that reaches us without its
        headline — a forward, a truncated notice — must not have its merchant or
        its amount read as the operation.
        """
        assert operation_from_raw("📍 PLUM OPLATA, UZ\n➖ 5 000.00 UZS") is None
        assert operation_from_raw("➖ 5 000.00 UZS\n💳 ***4483") is None
        assert operation_from_raw("🕓 01.01.26 07:16") is None

    def test_a_long_first_line_is_not_a_headline(self):
        assert operation_from_raw("🔴 " + "x" * 80) is None

    def test_the_amount_and_balance_lines_keep_each_banks_own_writing(self):
        assert amount_text_from_raw(HUMO_SMS) == "➖ 40.000,00 UZS"
        assert balance_text_from_raw(HUMO_SMS) == "💰 14.914,00 UZS"
        assert balance_text_from_raw(CARDXABAR_SMS) == "💵 4 229 872.82 UZS"

    def test_a_message_with_no_amount_line_falls_back_to_ours(self):
        assert amount_text_from_raw("Перевод через Payme") is None
        assert _row(raw_message="Перевод через Payme")["amount_text"] == "➖ 5 000.00 UZS"


class TestZerosNotBlanks:
    def test_no_application_is_a_zero(self):
        assert _row(application_mapped="")["application"] == 0

    def test_a_known_application_is_its_name(self):
        assert _row(application_mapped="Payme")["application"] == "Payme"

    def test_p2p_is_zero_rather_than_blank(self):
        assert _row(is_p2p=False)["p2p"] == 0
        assert _row(is_p2p=False)["p2p_confirmed"] is None

    def test_article_without_a_label_stays_a_bare_code(self):
        assert _row(annotation_article_code="р9", article_labels={})["article"] == "р9"


class TestSheetNames:
    def test_forbidden_characters_are_replaced(self):
        assert sanitize_sheet_name("CardXabar: чеки/2026", set()) == "CardXabar чеки 2026"

    def test_truncated_to_excel_limit(self):
        assert len(sanitize_sheet_name("x" * 60, set())) == 31

    def test_collisions_get_a_suffix(self):
        taken = set()
        assert sanitize_sheet_name("HUMO Card", taken) == "HUMO Card"
        assert sanitize_sheet_name("HUMO Card", taken) == "HUMO Card (2)"

    def test_blank_falls_back(self):
        assert sanitize_sheet_name("", set()) == "Лист"

    def test_a_name_in_quotes_would_make_the_file_unopenable(self):
        """
        Excel's other rule, which openpyxl does not enforce: a sheet name may not
        begin or end with an apostrophe. A chat titled `'CardXabar'` produced a
        workbook Excel refused to open, with no error from us.
        """
        assert sanitize_sheet_name("'CardXabar'", set()) == "CardXabar"
        assert sanitize_sheet_name("''", set()) == "Лист"


class TestExportEndpoint:
    """The whole workbook, through the HTTP route."""

    def test_client_layout_is_the_default_and_looks_like_their_file(self, client, db_session):
        _seed_monitored_chat(db_session, chat_id=-700, title="CardXabar")
        _seed_tx(db_session, idx=901, day=4, amount="-40000.00", operator="KORZINKA", source_chat_id=-700)
        res = client.get("/api/transactions/export")
        assert res.status_code == 200, res.text

        book = openpyxl.load_workbook(BytesIO(res.content))
        assert book.sheetnames == ["Все транзакции", "CardXabar"]
        assert book["Все транзакции"]["O1"].value == "Все транзакции"
        assert book["Все транзакции"]["U5"].value == "KORZINKA"
        sheet = book["CardXabar"]
        assert sheet["O1"].value == "Сборка CardXabar"
        assert sheet["O2"].value == "№"
        assert sheet["Q2"].value == "Дата_Время"
        assert sheet["O5"].value == 1
        assert sheet["U5"].value == "KORZINKA"
        assert sheet["Z5"].value == 40000
        assert sheet.freeze_panes == "Q5"
        assert sheet.sheet_view.showZeros is False

    def test_every_receipt_first_then_one_sheet_per_source_in_their_order(self, client, db_session):
        _seed_monitored_chat(db_session, chat_id=-700, title="CardXabar")
        _seed_monitored_chat(db_session, chat_id=-701, title="HUMO Card")
        _seed_tx(db_session, idx=902, day=5, amount="-1000.00", source_chat_id=-701)
        _seed_tx(db_session, idx=903, day=6, amount="-2000.00", source_chat_id=-700)

        res = client.get("/api/transactions/export")
        assert res.status_code == 200, res.text

        book = openpyxl.load_workbook(BytesIO(res.content))
        assert book.sheetnames == ["Все транзакции", "CardXabar", "HUMO Card"]
        combined = book["Все транзакции"]
        assert combined["P3"].value == 2
        assert [combined.cell(r, 19).value for r in (5, 6)] == ["HUMO Card", "CardXabar"]
        assert book["HUMO Card"]["O1"].value == "Сборка HumoCard"
        assert book["HUMO Card"]["P4"].value == 2

    def test_a_chat_without_a_title_still_gets_a_sheet(self, client, db_session):
        _seed_tx(db_session, idx=904, day=7, amount="-333.00", operator="TESTOP")
        res = client.get("/api/transactions/export")
        assert res.status_code == 200, res.text

        book = openpyxl.load_workbook(BytesIO(res.content))
        assert book.sheetnames == ["Все транзакции", "Telegram 100"]
        assert book["Telegram 100"]["O1"].value == "Сборка Telegram 100"

    def test_plain_layout_still_available(self, client, db_session):
        _seed_tx(db_session, idx=905, day=8, amount="-10.00")
        res = client.get("/api/transactions/export", params={"layout": "plain"})
        assert res.status_code == 200

        sheet = openpyxl.load_workbook(BytesIO(res.content)).worksheets[0]
        assert sheet.title == "Транзакции"
        assert sheet.cell(1, 1).value == "№"

class TestTheLiveSheetsFeed:
    """
    The CSV the self-refreshing workbook pulls.

    It has to agree with the .xlsx cell for cell — the customer can have both
    open at once — so it is built from the same `build_row_values`, and it keeps
    the spacer columns so the rows land on their own letters: `Z` is Сумма and
    `AG` is Остаток for anything they have pointed a formula at.
    """

    def test_it_carries_their_whole_block_in_their_order(self):
        columns = block_columns()
        assert len(columns) == 26
        assert columns[0] == ("O", "row_number")
        assert columns[11] == ("Z", "amount_value")
        assert columns[18] == ("AG", "balance_value")
        assert columns[-1] == ("AN", "article")

    def test_the_spacers_are_kept_so_the_letters_still_line_up(self):
        empty = [letter for letter, key in block_columns() if key is None]
        assert empty == ["V", "AA", "AB", "AC", "AD", "AE"]

    def test_the_feed_says_what_the_export_says(self, client, db_session, monkeypatch):
        # The feed is key-guarded, not session-guarded: the workbook refreshes
        # with nobody logged in.
        monkeypatch.setenv("TRANSACTIONS_EXPORT_KEY", "live-sheet-key")
        _seed_monitored_chat(db_session, chat_id=-915, title="CardXabar")
        tx = _seed_tx(
            db_session, idx=950, day=1, amount="-5000.00",
            balance_after="4229872.82", source_chat_id=-915, app_name="",
        )
        tx.raw_message = CARDXABAR_SMS_AS_SENT
        db_session.commit()
        response = client.get(
            "/api/transactions/export.csv?layout=client",
            headers={"X-Export-Key": "live-sheet-key"},
        )
        assert response.status_code == 200
        lines = response.text.lstrip("\ufeff").splitlines()
        header = lines[0].split(",")
        assert header[0] == "id"
        assert header[1:] == [key or f"_{letter}" for letter, key in block_columns()]

        row = dict(zip(header, next(csv.reader(lines[1:]))))
        # The bank's own words, and an unsigned whole number beside the sign.
        assert row["operation"] == "🔴 E-Com oplata"
        assert row["amount_value"] == "5000"
        assert row["sign"] == "-"
        assert row["balance_value"] == "4229872"
        # Their zeros, not blanks.
        assert row["application"] == "0"
        assert row["p2p"] == "0"


class TestTheLiveWorkbooksQuery:
    """
    The Power Query the operator tool writes into their template.

    It is not covered by the workbook build itself — that needs Excel — but the
    query text is where the mistakes live, and one of them cost a rebuild: the
    first version selected only the named columns, which dropped their six empty
    spacers and slid Сумма from `Z` to `X`.
    """

    def _query(self):
        from tools.build_live_workbook import build_m_query

        return build_m_query("https://host/api/transactions/export.csv?layout=client", "KEY")

    def test_it_lands_their_whole_block_in_their_order(self):
        selected = self._query().split("Table.SelectColumns(Пустые, {")[1].split("})")[0]
        columns = [part.strip().strip('"') for part in selected.split(",")]
        assert columns == [key or f"_{letter}" for letter, key in block_columns()]
        assert len(columns) == 26

    def test_the_transaction_id_is_not_one_of_their_columns(self):
        """It would shift the whole block one letter to the right."""
        selected = self._query().split("Table.SelectColumns(Пустые, {")[1].split("})")[0]
        assert '"id"' not in selected

    def test_the_key_travels_in_a_header_not_in_the_url(self):
        query = self._query()
        assert '[Headers=[#"X-Export-Key"="KEY"]]' in query
        assert "KEY" not in query.split("Web.Contents(")[1].split(",")[0]

    def test_the_columns_that_hold_a_word_or_a_zero_keep_the_zero_numeric(self):
        """Their view hides numeric zeros, not the text `0`."""
        query = self._query()
        for key in ("application", "balance_text"):
            assert f'{{"{key}", each try Number.From(_) otherwise _}}' in query

    def test_an_absent_value_becomes_an_empty_cell_not_an_empty_string(self):
        assert 'Table.ReplaceValue(\n        Смешанные, "", null' in self._query()
