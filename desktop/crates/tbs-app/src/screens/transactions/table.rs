//! The table's saved look and the adapter that feeds the grid.

use std::borrow::Cow;
use std::collections::HashMap;

use iced::Color;
use serde::{Deserialize, Serialize};
use tbs_core::columns::{self, Column};
use tbs_core::query::{Dataset, Filters, Sort};

use crate::grid::{Align, CellStyle, GridColumn, GridData, RowTone};

/// Columns a fresh table starts without. The date and time already sit in
/// «Дата и время»; «Описание» is the per-operator note, which the ledger does
/// not carry (its notes are «Подробности»). The view panel brings them back.
pub const QUIET: [Column; 4] = [Column::TransactionDate, Column::Time, Column::Day, Column::Description];

/// A fresh table reads like the customer's ledger, column for column: service,
/// operation, operator, application, money, card, then the bookkeeping notes,
/// and the receiver last. People check one against the other side by side.
pub fn default_order() -> Vec<Column> {
    let ledger = [
        Column::RowNumber,
        Column::OperationNumber,
        Column::DateTime,
        Column::SourceType,
        Column::TransactionType,
        Column::OperatorRaw,
        Column::ApplicationMapped,
        Column::Amount,
        Column::BalanceAfter,
        Column::CardLast4,
        Column::IsP2p,
        Column::AnnotationDetails,
        Column::AnnotationPersonCode,
        Column::AnnotationPeriod,
        Column::AnnotationArticleCode,
        Column::Currency,
        Column::ReceiverName,
        Column::ReceiverCard,
    ];
    let mut order = ledger.to_vec();
    order.extend(columns::ALL.iter().copied().filter(|c| !ledger.contains(c)));
    order
}

/// Which arrangement of the defaults a saved view was made under.
pub const LAYOUT: u32 = 2;

/// What a fresh table looked like before it followed the ledger: four columns
/// that are nearly always empty came first and pushed the notes off the screen.
fn first_layout() -> (Vec<Column>, [Column; 5]) {
    let lead = [Column::RowNumber, Column::OperationNumber, Column::DateTime, Column::OperatorRaw, Column::Amount, Column::TransactionType];
    let mut order = lead.to_vec();
    order.extend(columns::ALL.iter().copied().filter(|c| !lead.contains(c)));
    (order, [Column::TransactionDate, Column::Time, Column::Day, Column::Currency, Column::SourceType])
}

pub const LOW_CONFIDENCE: f64 = 0.6;
pub const MAX_PINNED: usize = 3;

/// Rows the user may leave hidden or move; the row number stays.
pub const LOCKED: Column = Column::RowNumber;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Ultra,
    Compact,
    #[default]
    Standard,
    Comfortable,
}

impl Density {
    pub const ALL: [Density; 4] = [Density::Ultra, Density::Compact, Density::Standard, Density::Comfortable];

    pub fn row_height(self) -> f32 {
        match self {
            Density::Ultra => 22.0,
            Density::Compact => 28.0,
            Density::Standard => 36.0,
            Density::Comfortable => 52.0,
        }
    }

    pub fn font_size(self) -> f32 {
        match self {
            Density::Ultra => 11.0,
            Density::Compact => 12.0,
            Density::Standard => 13.0,
            Density::Comfortable => 14.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Density::Ultra => "Ультра-компактный",
            Density::Compact => "Компактный",
            Density::Standard => "Средний",
            Density::Comfortable => "Крупный",
        }
    }
}

/// How many receipts one page of the table holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageSize {
    Fifty,
    #[default]
    Hundred,
    TwoHundred,
    All,
}

impl PageSize {
    pub const ALL: [PageSize; 4] = [PageSize::Fifty, PageSize::Hundred, PageSize::TwoHundred, PageSize::All];

    pub fn rows(self) -> Option<usize> {
        match self {
            PageSize::Fifty => Some(50),
            PageSize::Hundred => Some(100),
            PageSize::TwoHundred => Some(200),
            PageSize::All => None,
        }
    }

    pub fn pages(self, total: usize) -> usize {
        self.rows().map_or(1, |size| total.div_ceil(size).max(1))
    }

    /// The rows of a page, with the page pulled back into range first.
    pub fn window(self, page: usize, total: usize) -> (usize, std::ops::Range<usize>) {
        let Some(size) = self.rows() else { return (0, 0..total) };
        let page = page.min(self.pages(total) - 1);
        let start = page * size;
        (page, start..(start + size).min(total))
    }

    pub fn page_of(self, position: usize) -> usize {
        self.rows().map_or(0, |size| position / size)
    }
}

impl std::fmt::Display for PageSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.rows() {
            Some(size) => write!(f, "{size}"),
            None => f.write_str("Все"),
        }
    }
}

/// "101–200 из 1 906", or "0 из 0" on an empty table.
pub fn range_label(range: &std::ops::Range<usize>, total: usize) -> String {
    use tbs_core::format::count;
    if range.is_empty() {
        return format!("0 из {}", count(total));
    }
    format!("{}–{} из {}", count(range.start + 1), count(range.end), count(total))
}

/// The fills offered in the context menu. Saved by position.
pub const FILLS: [(&str, Color); 6] = [
    ("Красный", Color::from_rgba(1.0, 0.349, 0.353, 0.2)),
    ("Зелёный", Color::from_rgba(0.361, 0.784, 0.369, 0.2)),
    ("Синий", Color::from_rgba(0.2, 0.565, 0.925, 0.2)),
    ("Жёлтый", Color::from_rgba(1.0, 0.647, 0.0, 0.2)),
    ("Серый", Color::from_rgba(0.165, 0.165, 0.165, 0.5)),
    ("Оранжевый", Color::from_rgba(1.0, 0.647, 0.0, 0.15)),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Justify {
    Left,
    Center,
    Right,
}

impl From<Justify> for Align {
    fn from(value: Justify) -> Self {
        match value {
            Justify::Left => Align::Left,
            Justify::Center => Align::Center,
            Justify::Right => Align::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Paint {
    pub fill: Option<u8>,
    pub align: Option<Justify>,
}

impl Paint {
    pub fn is_empty(&self) -> bool {
        self.fill.is_none() && self.align.is_none()
    }

    fn color(&self) -> Option<Color> {
        self.fill.and_then(|i| FILLS.get(i as usize)).map(|(_, color)| *color)
    }
}

/// Cell paint on disk is a list: JSON object keys cannot hold a pair.
mod cell_paint {
    use super::*;
    use serde::{Deserializer, Serializer};

    type Map = HashMap<(i64, Column), Paint>;

    pub fn serialize<S: Serializer>(map: &Map, out: S) -> Result<S::Ok, S::Error> {
        out.collect_seq(map.iter().map(|((id, column), paint)| (id, column, paint)))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<Map, D::Error> {
        let items = Vec::<(i64, Column, Paint)>::deserialize(input)?;
        Ok(items.into_iter().map(|(id, column, paint)| ((id, column), paint)).collect())
    }
}

/// Everything about the table the user may arrange and expect to find again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableView {
    /// Absent in views saved before the ledger layout, which reads as zero.
    #[serde(default)]
    pub layout: u32,
    pub order: Vec<Column>,
    pub hidden: Vec<Column>,
    pub widths: HashMap<Column, f32>,
    pub density: Density,
    pub pinned: usize,
    pub column_paint: HashMap<Column, Paint>,
    #[serde(with = "cell_paint")]
    pub cell_paint: HashMap<(i64, Column), Paint>,
    pub sort: Sort,
    pub filters: Filters,
    pub page_size: PageSize,
}

impl Default for TableView {
    fn default() -> Self {
        Self {
            layout: LAYOUT,
            order: default_order(),
            hidden: QUIET.to_vec(),
            widths: HashMap::new(),
            density: Density::default(),
            pinned: MAX_PINNED,
            column_paint: HashMap::new(),
            cell_paint: HashMap::new(),
            sort: Sort::default(),
            filters: Filters::default(),
            page_size: PageSize::default(),
        }
    }
}

impl TableView {
    /// Saved state may predate a column, or name one twice after a bad write.
    pub fn repair(&mut self) {
        let mut seen = Vec::with_capacity(columns::COUNT);
        self.order.retain(|c| {
            let fresh = !seen.contains(c);
            seen.push(*c);
            fresh
        });
        for column in columns::ALL {
            if !self.order.contains(&column) {
                self.order.push(column);
            }
        }
        self.hidden.retain(|c| *c != LOCKED);
        self.pinned = self.pinned.min(MAX_PINNED);
        self.adopt_ledger_layout();
    }

    /// A view nobody rearranged gets the ledger layout once. One that was
    /// arranged by hand is the user's own and stays as it is.
    fn adopt_ledger_layout(&mut self) {
        if self.layout >= LAYOUT {
            return;
        }
        self.layout = LAYOUT;
        let (order, hidden) = first_layout();
        let untouched = self.order == order && self.hidden.len() == hidden.len() && hidden.iter().all(|c| self.hidden.contains(c));
        if untouched {
            self.order = default_order();
            self.hidden = QUIET.to_vec();
        }
    }

    /// The ledger's order and set of columns, keeping widths, paint and filters.
    pub fn use_ledger_columns(&mut self) {
        self.order = default_order();
        self.hidden = QUIET.to_vec();
    }

    pub fn visible(&self) -> Vec<Column> {
        self.order.iter().copied().filter(|c| !self.hidden.contains(c)).collect()
    }

    pub fn width(&self, column: Column) -> f32 {
        self.widths.get(&column).copied().unwrap_or_else(|| column.default_width())
    }

    pub fn grid_columns(&self, visible: &[Column]) -> Vec<GridColumn> {
        visible
            .iter()
            .map(|&column| {
                let mut out = GridColumn::new(column.title(), self.width(column));
                out.align = match column.align() {
                    columns::Align::Left => Align::Left,
                    columns::Align::Right => Align::Right,
                    columns::Align::Center => Align::Center,
                };
                let paint = self.column_paint.get(&column);
                if let Some(align) = paint.and_then(|p| p.align) {
                    out.align = align.into();
                }
                out.background = paint.and_then(Paint::color);
                out.mono = matches!(column, Column::RowNumber | Column::OperationNumber);
                out.medium = column == Column::Amount;
                if column.sortable() {
                    out = out.sortable((self.sort.column == column).then_some(self.sort.descending));
                }
                out
            })
            .collect()
    }
}

fn date_linked(column: Column) -> bool {
    matches!(column, Column::OperationNumber | Column::DateTime | Column::TransactionDate | Column::Time | Column::Day)
}

/// Number of a row counted through time, whichever way the table is sorted.
pub fn operation_number(sort: &Sort, total: usize, index: usize) -> usize {
    if date_linked(sort.column) && sort.descending { total - index } else { index + 1 }
}

/// One frame's view of the data; built in `view()`, costs nothing to make.
///
/// `rows` is one page; `offset` and `total` place it in the whole filtered
/// table, so the numbers in a row do not restart on every page.
pub struct TxGrid<'a> {
    pub data: &'a Dataset,
    pub rows: &'a [u32],
    pub offset: usize,
    pub total: usize,
    pub columns: &'a [Column],
    pub table: &'a TableView,
    pub highlight: Option<i64>,
}

impl GridData for TxGrid<'_> {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let column = self.columns[col];
        match column {
            Column::RowNumber => Cow::Owned((self.offset + row + 1).to_string()),
            Column::OperationNumber => Cow::Owned(operation_number(&self.table.sort, self.total, self.offset + row).to_string()),
            _ => match self.data.rows[self.rows[row] as usize].cell(column) {
                // An empty field stays empty: a column of dashes reads as data.
                tbs_core::format::EMPTY => Cow::Borrowed(""),
                "1" if column == Column::IsP2p => Cow::Borrowed("П2П"),
                value => Cow::Borrowed(value),
            },
        }
    }

    fn cell_style(&self, row: usize, col: usize) -> CellStyle {
        let column = self.columns[col];
        let tx = &self.data.rows[self.rows[row] as usize].tx;
        // Black on white, like the ledger it mirrors: a value is never coloured by
        // its type. The only fills are the ones the user paints.
        let mut style = CellStyle { medium: column == Column::Amount, ..Default::default() };
        if let Some(paint) = self.table.cell_paint.get(&(tx.id, column)) {
            style.background = paint.color();
            style.align = paint.align.map(Into::into);
        }
        style
    }

    fn row_tone(&self, row: usize) -> RowTone {
        let tx = &self.data.rows[self.rows[row] as usize].tx;
        if self.highlight == Some(tx.id) {
            RowTone::Highlight
        } else if tx.parsing_confidence.is_some_and(|c| c < LOW_CONFIDENCE) {
            RowTone::Warning
        } else {
            RowTone::Plain
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_layout_survives_a_new_column_and_a_duplicate() {
        let mut view = TableView { order: vec![Column::Amount, Column::Amount, Column::Time], hidden: vec![LOCKED], pinned: 9, ..Default::default() };
        view.repair();
        assert_eq!(view.order.len(), columns::COUNT);
        assert_eq!(view.order[..2], [Column::Amount, Column::Time]);
        assert!(view.hidden.is_empty());
        assert_eq!(view.pinned, MAX_PINNED);
    }

    #[test]
    fn a_fresh_table_reads_like_the_ledger() {
        let view = TableView::default();
        let visible = view.visible();
        // The workbook's own order: Сервис, Операция, Оператор, Приложение, Сумма,
        // Остаток, ПК, p2p, Подробности, Лицо, Период, Статья, Валюта, Получатель, Карта.
        let ledger = [
            Column::RowNumber,
            Column::OperationNumber,
            Column::DateTime,
            Column::SourceType,
            Column::TransactionType,
            Column::OperatorRaw,
            Column::ApplicationMapped,
            Column::Amount,
            Column::BalanceAfter,
            Column::CardLast4,
            Column::IsP2p,
            Column::AnnotationDetails,
            Column::AnnotationPersonCode,
            Column::AnnotationPeriod,
            Column::AnnotationArticleCode,
            Column::Currency,
            Column::ReceiverName,
            Column::ReceiverCard,
        ];
        assert_eq!(visible, ledger);
        assert_eq!(view.order.len(), columns::ALL.len());
        // Not in the ledger: the split date and the per-operator note.
        for quiet in [Column::TransactionDate, Column::Time, Column::Day, Column::Description] {
            assert!(view.hidden.contains(&quiet) && view.order.contains(&quiet), "{quiet:?}");
        }
    }

    #[test]
    fn an_untouched_old_view_moves_to_the_ledger_layout_once() {
        let (order, hidden) = first_layout();
        let saved = serde_json::json!({ "order": order, "hidden": hidden, "pinned": 2, "widths": { "amount": 140.0 } });
        let mut view: TableView = serde_json::from_value(saved).unwrap();
        assert_eq!(view.layout, 0, "a view saved before the field must not pass for a new one");
        view.repair();
        assert_eq!(view.order, default_order());
        assert_eq!(view.hidden, QUIET);
        // What the user did set survives the move.
        assert_eq!((view.pinned, view.width(Column::Amount)), (2, 140.0));
        assert_eq!(view.layout, LAYOUT);
        // And it is not done twice: hiding the same columns again is now a choice.
        let (order, hidden) = first_layout();
        let mut chosen = TableView { order: order.clone(), hidden: hidden.to_vec(), ..view };
        chosen.repair();
        assert_eq!(chosen.order, order);
    }

    #[test]
    fn a_view_arranged_by_hand_is_left_alone() {
        let (mut order, hidden) = first_layout();
        order.swap(3, 4);
        let mut moved: TableView = serde_json::from_value(serde_json::json!({ "order": order, "hidden": hidden })).unwrap();
        moved.repair();
        assert_eq!(moved.order, order);
        assert_eq!(moved.layout, LAYOUT);

        let (order, _) = first_layout();
        let mut shown: TableView = serde_json::from_value(serde_json::json!({ "order": order, "hidden": [Column::Time] })).unwrap();
        shown.repair();
        assert_eq!(shown.order, order);
        assert_eq!(shown.hidden, [Column::Time]);
    }

    #[test]
    fn the_ledger_columns_can_be_asked_for_again() {
        let mut view = TableView { order: vec![Column::Amount, Column::RowNumber], hidden: vec![Column::Amount], pinned: 1, ..Default::default() };
        view.repair();
        view.widths.insert(Column::Amount, 200.0);
        view.use_ledger_columns();
        assert_eq!(view.order, default_order());
        assert_eq!(view.hidden, QUIET);
        assert_eq!((view.pinned, view.width(Column::Amount)), (1, 200.0));
    }

    #[test]
    fn view_round_trips_with_cell_paint() {
        let mut view = TableView::default();
        view.cell_paint.insert((7, Column::Amount), Paint { fill: Some(1), align: Some(Justify::Center) });
        view.widths.insert(Column::Time, 91.0);
        let text = serde_json::to_string(&view).unwrap();
        let back: TableView = serde_json::from_str(&text).unwrap();
        assert_eq!(back, view);
    }

    #[test]
    fn pages_cut_the_table_and_stay_in_range() {
        assert_eq!(PageSize::Fifty.pages(0), 1);
        assert_eq!(PageSize::Fifty.pages(50), 1);
        assert_eq!(PageSize::Fifty.pages(51), 2);
        assert_eq!(PageSize::All.pages(5000), 1);
        assert_eq!(PageSize::Hundred.window(1, 250), (1, 100..200));
        assert_eq!(PageSize::Hundred.window(2, 250), (2, 200..250));
        assert_eq!(PageSize::Hundred.window(9, 250), (2, 200..250));
        assert_eq!(PageSize::TwoHundred.window(3, 0), (0, 0..0));
        assert_eq!(PageSize::All.window(4, 250), (0, 0..250));
        assert_eq!(PageSize::Fifty.page_of(49), 0);
        assert_eq!(PageSize::Fifty.page_of(50), 1);
        assert_eq!(PageSize::All.page_of(999), 0);
    }

    #[test]
    fn range_reads_like_a_page_footer() {
        assert_eq!(range_label(&(100..200), 1906), format!("101–200 из {}", tbs_core::format::count(1906)));
        assert_eq!(range_label(&(0..0), 0), "0 из 0");
    }

    #[test]
    fn numbers_continue_across_pages() {
        let data = Dataset::build((1..=5).map(|id| tbs_core::model::Transaction { id, ..Default::default() }).collect());
        let table = TableView::default();
        let columns = [Column::RowNumber, Column::OperationNumber];
        let rows = [2u32, 3];
        let grid = TxGrid { data: &data, rows: &rows, offset: 2, total: 5, columns: &columns, table: &table, highlight: None };
        assert_eq!(grid.row_count(), 2);
        assert_eq!(grid.cell(0, 0), "3");
        assert_eq!(grid.cell(1, 0), "4");
        let newest = TableView { sort: Sort { column: Column::TransactionDate, descending: true }, ..Default::default() };
        let grid = TxGrid { table: &newest, ..grid };
        assert_eq!(grid.cell(0, 1), "3");
        assert_eq!(grid.cell(1, 1), "2");
    }

    #[test]
    fn no_value_is_coloured_by_its_type() {
        let tx = |id, kind: &str, p2p| tbs_core::model::Transaction { id, transaction_type: Some(kind.into()), amount: Some("5000.00".into()), is_p2p: p2p, ..Default::default() };
        let data = Dataset::build(vec![tx(1, "DEBIT", true), tx(2, "CREDIT", false), tx(3, "REVERSAL", false)]);
        let table = TableView::default();
        let rows = [0u32, 1, 2];
        let grid = TxGrid { data: &data, rows: &rows, offset: 0, total: 3, columns: &columns::ALL, table: &table, highlight: None };
        for row in 0..rows.len() {
            for col in 0..columns::COUNT {
                let style = grid.cell_style(row, col);
                assert!(style.tone.is_none() && style.color.is_none() && !style.pill, "{:?} in row {row}", columns::ALL[col]);
            }
        }
        assert_eq!(grid.cell(0, Column::IsP2p.index()), "П2П");
    }

    #[test]
    fn the_page_size_list_names_every_choice() {
        assert_eq!(PageSize::ALL.map(|size| size.to_string()), ["50", "100", "200", "Все"]);
    }

    #[test]
    fn an_old_saved_view_gets_the_default_page() {
        let view: TableView = serde_json::from_str(r#"{"pinned":1}"#).unwrap();
        assert_eq!(view.page_size, PageSize::Hundred);
    }

    #[test]
    fn operation_numbers_follow_time() {
        let newest = Sort { column: Column::TransactionDate, descending: true };
        assert_eq!(operation_number(&newest, 10, 0), 10);
        let by_amount = Sort { column: Column::Amount, descending: true };
        assert_eq!(operation_number(&by_amount, 10, 0), 1);
    }
}
