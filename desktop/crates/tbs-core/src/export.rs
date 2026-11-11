//! A plain workbook of the rows the user picked.
//!
//! The server only exports by filter, so a hand-made selection is written here.
//! This file does not use the client's report template.

use rust_xlsxwriter::{Format, Workbook, XlsxError};

use crate::columns::{Align, Column};
use crate::query::Row;

const SHEET: &str = "Транзакции";
const AMOUNT_FORMAT: &str = "#,##0.00";

fn number(column: Column, row: &Row) -> Option<f64> {
    if !matches!(column, Column::Amount | Column::BalanceAfter) {
        return None;
    }
    column.raw_value(&row.tx).trim().parse().ok()
}

/// `numbering` gives the text of positional columns, which depend on the view.
pub fn selected_rows_xlsx(
    rows: &[&Row],
    columns: &[Column],
    numbering: impl Fn(Column, usize) -> Option<String>,
) -> Result<Vec<u8>, XlsxError> {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name(SHEET)?;
    let head = Format::new().set_bold();
    let money = Format::new().set_num_format(AMOUNT_FORMAT);

    for (x, column) in columns.iter().enumerate() {
        let x = x as u16;
        sheet.write_string_with_format(0, x, column.title(), &head)?;
        sheet.set_column_width(x, f64::from(column.default_width()) / 7.0)?;
    }
    for (y, row) in rows.iter().enumerate() {
        let line = y as u32 + 1;
        for (x, &column) in columns.iter().enumerate() {
            let x = x as u16;
            if let Some(value) = number(column, row) {
                sheet.write_number_with_format(line, x, value, &money)?;
                continue;
            }
            let text = numbering(column, y).unwrap_or_else(|| row.cell(column).to_string());
            if column.align() == Align::Right
                && let Ok(value) = text.parse::<f64>()
            {
                sheet.write_number(line, x, value)?;
            } else {
                sheet.write_string(line, x, text)?;
            }
        }
    }
    sheet.set_freeze_panes(1, 0)?;
    book.save_to_buffer()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Transaction;
    use crate::query::Dataset;

    #[test]
    fn writes_a_real_workbook() {
        let tx = Transaction {
            id: 7,
            transaction_date: Some("2026-03-05T10:20:00".into()),
            amount: Some("1500.50".into()),
            operator_raw: Some("Korzinka".into()),
            ..Default::default()
        };
        let data = Dataset::build(vec![tx]);
        let rows: Vec<&Row> = data.rows.iter().collect();
        let bytes = selected_rows_xlsx(&rows, &[Column::RowNumber, Column::OperatorRaw, Column::Amount], |column, y| {
            (column == Column::RowNumber).then(|| (y + 1).to_string())
        })
        .unwrap();
        // An xlsx file is a zip archive.
        assert_eq!(&bytes[..2], b"PK");
        assert!(bytes.len() > 1000);
    }

    #[test]
    fn the_operation_goes_out_as_the_text_of_its_cell() {
        let tx = Transaction { id: 8, raw_message: Some("\u{1f4b8} Оплата\n➖ 40.000,00 UZS".into()), ..Default::default() };
        let data = Dataset::build(vec![tx]);
        let rows: Vec<&Row> = data.rows.iter().collect();
        assert_eq!(rows[0].cell(Column::Operation), "\u{1f4b8} Оплата");
        assert_eq!(number(Column::Operation, rows[0]), None);
        let bytes = selected_rows_xlsx(&rows, &[Column::Operation, Column::TransactionType], |_, _| None).unwrap();
        assert_eq!(&bytes[..2], b"PK");
    }
}
