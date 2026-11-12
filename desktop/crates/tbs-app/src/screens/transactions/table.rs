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

/// A fresh table reads like the customer's ledger, column for column: the
/// operation's time, the same time made unique, service, operation, operator,
/// application, money, card, p2p, the note; then the bookkeeping codes and the
/// rest. People check one against the other side by side.
/// «Тип» is the one column the ledger does not have: the server's word for
/// the operation. It stands after the ledger's own columns, apart from the
/// bank's «Операция».
pub fn default_order() -> Vec<Column> {
    let ledger = [
        Column::RowNumber,
        Column::OperationNumber,
        Column::DateTime,
        Column::DateTimeDedup,
        Column::SourceType,
        Column::Operation,
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
        Column::TransactionType,
        Column::Currency,
        Column::ReceiverName,
        Column::ReceiverCard,
    ];
    let mut order = ledger.to_vec();
    order.extend(columns::ALL.iter().copied().filter(|c| !ledger.contains(c)));
    order
}

/// Which arrangement of the defaults a saved view was made under.
pub const LAYOUT: u32 = 4;

/// Columns that came after the first builds, in the order they came. A view
/// saved without one of them gets it at the end, where `repair` puts it.
const LATER: [Column; 2] = [Column::Operation, Column::DateTimeDedup];

/// What a fresh table looked like before it followed the ledger: four columns
/// that are nearly always empty came first and pushed the notes off the screen.
///
/// Like every earlier layout, this is the view as `repair` hands it over: the
/// columns that came later stand at the end, where `columns::ALL` has them.
fn first_layout() -> (Vec<Column>, [Column; 5]) {
    let lead = [Column::RowNumber, Column::OperationNumber, Column::DateTime, Column::OperatorRaw, Column::Amount, Column::TransactionType];
    let mut order = lead.to_vec();
    order.extend(columns::ALL.iter().copied().filter(|c| !lead.contains(c)));
    (order, [Column::TransactionDate, Column::Time, Column::Day, Column::Currency, Column::SourceType])
}

/// The ledger layout of the builds that had no «Операция»: «Тип» stood in its place.
fn second_layout() -> (Vec<Column>, [Column; 4]) {
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
    (order, [Column::TransactionDate, Column::Time, Column::Day, Column::Description])
}

/// The ledger layout of the builds that had «Операция» with «Тип» beside it and
/// no «(Без дубля)».
fn third_layout() -> (Vec<Column>, [Column; 4]) {
    let ledger = [
        Column::RowNumber,
        Column::OperationNumber,
        Column::DateTime,
        Column::SourceType,
        Column::Operation,
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
    (order, [Column::TransactionDate, Column::Time, Column::Day, Column::Description])
}

pub const LOW_CONFIDENCE: f64 = 0.6;
pub const MAX_PINNED: usize = 3;

/// Rows the user may leave hidden or move; the row number stays.
pub const LOCKED: Column = Column::RowNumber;

/// One place along the order of the columns, for the arrows of the view sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
}

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
        let knew: Vec<Column> = LATER.into_iter().filter(|column| self.order.contains(column)).collect();
        for column in columns::ALL {
            if !self.order.contains(&column) {
                self.order.push(column);
            }
        }
        self.hidden.retain(|c| *c != LOCKED);
        self.pinned = self.pinned.min(MAX_PINNED);
        self.adopt_ledger_layout(&knew);
    }

    /// A view nobody rearranged gets the ledger layout once. One that was
    /// arranged by hand is the user's own and stays as it is: a column it was
    /// saved without waits hidden at the end, a tick away in the view panel.
    fn adopt_ledger_layout(&mut self, knew: &[Column]) {
        if self.layout >= LAYOUT {
            return;
        }
        let (order, hidden) = match self.layout {
            0 | 1 => {
                let (order, hidden) = first_layout();
                (order, hidden.to_vec())
            }
            2 => {
                let (order, hidden) = second_layout();
                (order, hidden.to_vec())
            }
            _ => {
                let (order, hidden) = third_layout();
                (order, hidden.to_vec())
            }
        };
        let untouched = self.has_layout(&order, &hidden);
        self.layout = LAYOUT;
        if untouched {
            self.order = default_order();
            self.hidden = QUIET.to_vec();
        } else {
            for column in LATER {
                if !knew.contains(&column) && !self.hidden.contains(&column) {
                    self.hidden.push(column);
                }
            }
        }
    }

    /// The same columns in the same places, and the same ones hidden.
    fn has_layout(&self, order: &[Column], hidden: &[Column]) -> bool {
        self.order == order && self.hidden.len() == hidden.len() && hidden.iter().all(|c| self.hidden.contains(c))
    }

    /// The ledger's order and set of columns, keeping widths, paint and filters.
    pub fn use_ledger_columns(&mut self) {
        self.order = default_order();
        self.hidden = QUIET.to_vec();
    }

    /// Whether the columns already stand as the ledger has them.
    pub fn has_ledger_columns(&self) -> bool {
        self.has_layout(&default_order(), &QUIET)
    }

    /// Moves a column one place along the order, past whatever stands there,
    /// shown or not. `false` when it is already at that end.
    pub fn move_column(&mut self, column: Column, step: Step) -> bool {
        let Some(from) = self.order.iter().position(|c| *c == column) else { return false };
        let to = match step {
            Step::Up if from > 0 => from - 1,
            Step::Down if from + 1 < self.order.len() => from + 1,
            _ => return false,
        };
        self.order.swap(from, to);
        true
    }

    /// How many columns stay in place on the left, of the ones that are shown.
    pub fn pinned_shown(&self) -> usize {
        self.pinned.min(self.visible().len())
    }

    /// The look of a fresh table: columns, widths, density, pinning, sorting
    /// and the page. What narrows the table stays, and so do the cells painted
    /// by hand: both are about the receipts, not about the look.
    pub fn reset_layout(&mut self) {
        let filters = std::mem::take(&mut self.filters);
        let cell_paint = std::mem::take(&mut self.cell_paint);
        *self = Self { filters, cell_paint, ..Self::default() };
    }

    /// Whether `reset_layout` would leave the table as it is.
    pub fn has_fresh_layout(&self) -> bool {
        let fresh = Self::default();
        self.has_ledger_columns()
            && self.widths.is_empty()
            && self.column_paint.is_empty()
            && (self.density, self.pinned, self.sort, self.page_size) == (fresh.density, fresh.pinned, fresh.sort, fresh.page_size)
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
    matches!(column, Column::OperationNumber | Column::DateTime | Column::DateTimeDedup | Column::TransactionDate | Column::Time | Column::Day)
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
                // «П2П» is the ledger's flag as it stands there: 1, or nothing.
                tbs_core::format::EMPTY => Cow::Borrowed(""),
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
        // The workbook's own order: Дата_Время, (Без дубля), Сервис, Операция, Оператор,
        // Приложение, Сумма, Остаток, ПК, p2p, Подробности, then Лицо, Период, Статья.
        // «Тип» is ours: it follows the ledger's columns and is not mixed into them.
        let ledger = [
            Column::RowNumber,
            Column::OperationNumber,
            Column::DateTime,
            Column::DateTimeDedup,
            Column::SourceType,
            Column::Operation,
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
            Column::TransactionType,
            Column::Currency,
            Column::ReceiverName,
            Column::ReceiverCard,
        ];
        assert_eq!(visible, ledger);
        // From the first date through the note, the ledger's sequence without a gap.
        let through_the_note = ["Дата и время", "(Без дубля)", "Источник", "Операция", "Оператор/Продавец", "Приложение", "Сумма", "Остаток", "ПК", "П2П", "Подробности"];
        assert_eq!(visible[2..13].iter().map(|c| c.title()).collect::<Vec<_>>(), through_the_note);
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

    /// The default view of the first builds, by the names it has on disk.
    fn first_view_on_disk() -> serde_json::Value {
        serde_json::json!({
            "order": [
                "row_number", "operation_number", "date_time", "operator_raw", "amount", "transaction_type", "transaction_date", "time",
                "day", "application_mapped", "description", "receiver_name", "receiver_card", "balance_after", "card_last4", "is_p2p",
                "currency", "source_type", "annotation_details", "annotation_person_code", "annotation_period", "annotation_article_code",
            ],
            "hidden": ["transaction_date", "time", "day", "currency", "source_type"],
        })
    }

    /// The default view of the builds that followed the ledger and had no «Операция» yet.
    fn ledger_view_on_disk() -> serde_json::Value {
        serde_json::json!({
            "layout": 2,
            "order": [
                "row_number", "operation_number", "date_time", "source_type", "transaction_type", "operator_raw", "application_mapped", "amount",
                "balance_after", "card_last4", "is_p2p", "annotation_details", "annotation_person_code", "annotation_period", "annotation_article_code", "currency",
                "receiver_name", "receiver_card", "transaction_date", "time", "day", "description",
            ],
            "hidden": ["transaction_date", "time", "day", "description"],
        })
    }

    /// The default view of the builds that had «Операция» beside «Тип» and no «(Без дубля)».
    fn previous_view_on_disk() -> serde_json::Value {
        serde_json::json!({
            "layout": 3,
            "order": [
                "row_number", "operation_number", "date_time", "source_type", "operation", "transaction_type", "operator_raw", "application_mapped",
                "amount", "balance_after", "card_last4", "is_p2p", "annotation_details", "annotation_person_code", "annotation_period",
                "annotation_article_code", "currency", "receiver_name", "receiver_card", "transaction_date", "time", "day", "description",
            ],
            "hidden": ["transaction_date", "time", "day", "description"],
        })
    }

    fn read(saved: &serde_json::Value) -> TableView {
        serde_json::from_value(saved.clone()).unwrap()
    }

    #[test]
    fn the_views_on_disk_are_the_layouts_they_were_saved_under() {
        let without = |order: Vec<Column>, later: &[Column]| serde_json::json!(order.into_iter().filter(|c| !later.contains(c)).collect::<Vec<_>>());
        assert_eq!(first_view_on_disk()["order"], without(first_layout().0, &LATER));
        assert_eq!(first_view_on_disk()["hidden"], serde_json::json!(first_layout().1));
        assert_eq!(ledger_view_on_disk()["order"], without(second_layout().0, &LATER));
        assert_eq!(ledger_view_on_disk()["hidden"], serde_json::json!(second_layout().1));
        assert_eq!(previous_view_on_disk()["order"], without(third_layout().0, &[Column::DateTimeDedup]));
        assert_eq!(previous_view_on_disk()["hidden"], serde_json::json!(third_layout().1));
        // Each of them ends, once repaired, with the columns it was saved without.
        assert_eq!(first_layout().0[columns::COUNT - 2..], LATER);
        assert_eq!(second_layout().0[columns::COUNT - 2..], LATER);
        assert_eq!(third_layout().0.last(), Some(&Column::DateTimeDedup));
    }

    #[test]
    fn an_untouched_view_of_the_previous_build_gets_the_second_date_and_the_ledgers_sequence() {
        let mut saved = previous_view_on_disk();
        saved["pinned"] = 2.into();
        saved["widths"] = serde_json::json!({ "amount": 140.0 });
        saved["sort"] = serde_json::json!({ "column": "operation", "descending": true });
        let mut view = read(&saved);
        assert_eq!((view.layout, view.order.len()), (3, columns::COUNT - 1));
        view.repair();
        assert_eq!((&view.order, &view.hidden, view.layout), (&default_order(), &QUIET.to_vec(), LAYOUT));
        // «(Без дубля)» stands after the first date, and «Тип» has left the ledger's run.
        let visible = view.visible();
        assert_eq!(visible[2..7], [Column::DateTime, Column::DateTimeDedup, Column::SourceType, Column::Operation, Column::OperatorRaw]);
        let at = |column| visible.iter().position(|c| *c == column).unwrap();
        assert!(at(Column::TransactionType) > at(Column::AnnotationArticleCode));
        // What the user did set survives the move.
        assert_eq!((view.pinned, view.width(Column::Amount)), (2, 140.0));
        assert_eq!(view.sort, Sort { column: Column::Operation, descending: true });

        // Saved and opened again, nothing more happens.
        let mut reopened = read(&serde_json::to_value(&view).unwrap());
        reopened.repair();
        assert_eq!(reopened, view);
        // And it is not done twice: going back to the old arrangement is now a choice.
        let (order, hidden) = third_layout();
        let mut chosen = TableView { order: order.clone(), hidden: hidden.to_vec(), ..view };
        chosen.repair();
        assert_eq!((chosen.order, chosen.hidden), (order, hidden.to_vec()));
    }

    #[test]
    fn a_view_of_the_previous_build_arranged_by_hand_keeps_its_columns() {
        let mut moved = previous_view_on_disk();
        moved["order"].as_array_mut().unwrap().swap(6, 7);
        let mut shown = previous_view_on_disk();
        shown["hidden"] = serde_json::json!(["transaction_date", "time", "day"]);
        let mut put_away = previous_view_on_disk();
        put_away["hidden"].as_array_mut().unwrap().push("operation".into());

        for saved in [moved, shown, put_away] {
            let before = read(&saved);
            let mut view = before.clone();
            view.repair();
            // Exactly the columns it showed, in the places it showed them.
            assert_eq!(view.visible(), before.visible(), "{saved}");
            assert_eq!(view.order[..before.order.len()], before.order[..], "{saved}");
            // The second date waits at the end, unticked; «Операция», which the view knew, is as it was left.
            assert_eq!(view.order[before.order.len()..], [Column::DateTimeDedup], "{saved}");
            assert_eq!(view.hidden, [before.hidden.clone(), vec![Column::DateTimeDedup]].concat(), "{saved}");
            assert_eq!(view.layout, LAYOUT);

            // «Как в книге Excel» is still one press away.
            assert!(!view.has_ledger_columns());
            view.use_ledger_columns();
            assert_eq!((&view.order, &view.hidden), (&default_order(), &QUIET.to_vec()));
        }
    }

    #[test]
    fn an_untouched_ledger_view_gets_the_operation_column_once() {
        let mut saved = ledger_view_on_disk();
        saved["pinned"] = 2.into();
        saved["widths"] = serde_json::json!({ "amount": 140.0 });
        saved["sort"] = serde_json::json!({ "column": "transaction_type", "descending": true });
        let mut view = read(&saved);
        assert_eq!((view.layout, view.order.len()), (2, columns::COUNT - 2));
        view.repair();
        assert_eq!(view.order, default_order());
        assert_eq!(view.hidden, QUIET);
        // «Операция» stands where the ledger has it, shown, between «Источник» and the operator.
        assert_eq!(view.visible()[4..7], [Column::SourceType, Column::Operation, Column::OperatorRaw]);
        // What the user did set survives the move.
        assert_eq!((view.pinned, view.width(Column::Amount)), (2, 140.0));
        assert_eq!(view.sort, Sort { column: Column::TransactionType, descending: true });
        assert_eq!(view.layout, LAYOUT);

        // Saved and opened again, nothing more happens.
        let mut reopened = read(&serde_json::to_value(&view).unwrap());
        reopened.repair();
        assert_eq!(reopened, view);
        // And it is not done twice: putting the column back at the end, or away, is now a choice.
        let (order, hidden) = second_layout();
        let mut chosen = TableView { order: order.clone(), hidden: hidden.to_vec(), ..view.clone() };
        chosen.repair();
        assert_eq!((chosen.order, chosen.hidden), (order, hidden.to_vec()));
        let mut put_away = TableView { hidden: [QUIET.to_vec(), vec![Column::Operation]].concat(), ..view };
        put_away.repair();
        assert_eq!(put_away.order, default_order());
        assert!(!put_away.visible().contains(&Column::Operation));
    }

    #[test]
    fn a_ledger_view_arranged_by_hand_keeps_its_columns() {
        let mut moved = ledger_view_on_disk();
        moved["order"].as_array_mut().unwrap().swap(4, 5);
        let mut shown = ledger_view_on_disk();
        shown["hidden"] = serde_json::json!(["transaction_date", "time", "day"]);
        let mut put_away = ledger_view_on_disk();
        put_away["hidden"].as_array_mut().unwrap().push("currency".into());
        let mut all_shown = ledger_view_on_disk();
        all_shown["hidden"] = serde_json::json!([]);

        for saved in [moved, shown, put_away, all_shown] {
            let before = read(&saved);
            let mut view = before.clone();
            view.repair();
            // Exactly the columns it showed, in the places it showed them.
            assert_eq!(view.visible(), before.visible(), "{saved}");
            assert_eq!(view.order[..before.order.len()], before.order[..], "{saved}");
            // The new columns wait at the end, unticked, for the view panel.
            assert_eq!(view.order[before.order.len()..], LATER, "{saved}");
            assert_eq!(view.hidden, [before.hidden.clone(), LATER.to_vec()].concat(), "{saved}");
            assert_eq!(view.layout, LAYOUT);

            // Ticked there, it shows after the user's own columns and stays shown.
            view.hidden.retain(|c| *c != Column::Operation);
            let ticked = view.clone();
            let mut reopened = read(&serde_json::to_value(&view).unwrap());
            reopened.repair();
            assert_eq!(reopened, ticked, "{saved}");
            assert_eq!(reopened.visible().last(), Some(&Column::Operation), "{saved}");
        }
    }

    #[test]
    fn the_first_builds_view_moves_to_the_ledger_layout_as_it_stands_on_disk() {
        let mut saved = first_view_on_disk();
        saved["density"] = "compact".into();
        let mut view = read(&saved);
        assert_eq!((view.layout, view.order.len()), (0, columns::COUNT - 2));
        view.repair();
        assert_eq!(view.order, default_order());
        assert_eq!(view.hidden, QUIET);
        assert!(view.visible().contains(&Column::Operation) && view.visible().contains(&Column::DateTimeDedup));
        assert_eq!((view.density, view.layout), (Density::Compact, LAYOUT));

        // Arranged by hand, it is the user's own: nothing moves and nothing new shows.
        let mut by_hand = first_view_on_disk();
        by_hand["hidden"] = serde_json::json!(["time"]);
        let before = read(&by_hand);
        let mut view = before.clone();
        view.repair();
        assert_eq!(view.visible(), before.visible());
        assert_eq!(view.order[..before.order.len()], before.order[..]);
        assert_eq!(view.order[before.order.len()..], LATER);
        assert_eq!(view.hidden, [Column::Time, Column::Operation, Column::DateTimeDedup]);
        assert_eq!(view.layout, LAYOUT);
    }

    #[test]
    fn a_view_saved_without_its_columns_reads_as_a_fresh_one() {
        // Only settings on disk: the order is today's, so «Операция» is already in its place.
        let mut bare: TableView = serde_json::from_str(r#"{"pinned":1}"#).unwrap();
        bare.repair();
        assert_eq!((bare.order, bare.hidden, bare.layout), (default_order(), QUIET.to_vec(), LAYOUT));
        let mut fresh = TableView::default();
        fresh.repair();
        assert_eq!(fresh, TableView::default());
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
    fn a_column_moves_one_place_and_stops_at_the_ends() {
        let mut view = TableView::default();
        let start = view.order.clone();
        assert_eq!(start.len(), columns::COUNT);
        // Down and back up is where it was.
        assert!(view.move_column(Column::Amount, Step::Down));
        let at = |view: &TableView, column| view.order.iter().position(|c| *c == column).unwrap();
        assert_eq!((at(&view, Column::Amount), at(&view, Column::BalanceAfter)), (9, 8));
        assert!(view.move_column(Column::Amount, Step::Up));
        assert_eq!(view.order, start);
        // The ends hold.
        let (first, last) = (start[0], start[columns::COUNT - 1]);
        assert!(!view.move_column(first, Step::Up));
        assert!(!view.move_column(last, Step::Down));
        assert_eq!(view.order, start);
        // All the way down, a column passes every other one, hidden ones too, and nothing is lost on the way.
        let mut steps = 0;
        while view.move_column(first, Step::Down) {
            steps += 1;
        }
        assert_eq!(steps, columns::COUNT - 1);
        assert_eq!(view.order.last(), Some(&first));
        assert_eq!(view.order[..columns::COUNT - 1], start[1..]);
        let mut sorted = view.order.clone();
        sorted.sort_by_key(|c| c.index());
        assert_eq!(sorted, columns::ALL);
        // Moving changes the order only.
        assert_eq!(view.hidden, QUIET);
    }

    #[test]
    fn a_hidden_column_is_a_place_in_the_order_too() {
        let mut view = TableView { order: vec![Column::RowNumber, Column::Time, Column::Amount], hidden: vec![Column::Time], ..Default::default() };
        view.repair();
        assert_eq!(view.visible()[..2], [Column::RowNumber, Column::Amount]);
        // One step takes «Сумма» past the hidden column: the table shows the same.
        assert!(view.move_column(Column::Amount, Step::Up));
        assert_eq!(view.order[..3], [Column::RowNumber, Column::Amount, Column::Time]);
        assert_eq!(view.visible()[..2], [Column::RowNumber, Column::Amount]);
        // The next one changes it.
        assert!(view.move_column(Column::Amount, Step::Up));
        assert_eq!(view.visible()[..2], [Column::Amount, Column::RowNumber]);
    }

    #[test]
    fn resetting_the_layout_keeps_the_filters_and_the_painted_cells() {
        let filters = Filters { search: "такси".into(), currency: Some("UZS".into()), amount_min: Some(50_000.0), ..Default::default() };
        let mut view = TableView {
            order: columns::ALL.to_vec(),
            hidden: vec![Column::Amount, Column::Operation],
            density: Density::Compact,
            pinned: 1,
            sort: Sort { column: Column::Amount, descending: true },
            filters: filters.clone(),
            page_size: PageSize::All,
            ..Default::default()
        };
        view.widths.insert(Column::OperatorRaw, 300.0);
        view.column_paint.insert(Column::Amount, Paint { fill: Some(2), align: None });
        view.cell_paint.insert((17, Column::AnnotationDetails), Paint { fill: Some(3), align: None });
        assert!(!view.has_fresh_layout());

        view.reset_layout();
        assert!(view.has_fresh_layout());
        let fresh = TableView::default();
        assert_eq!((&view.order, &view.hidden, view.layout), (&fresh.order, &fresh.hidden, LAYOUT));
        assert_eq!((view.density, view.pinned, view.sort, view.page_size), (fresh.density, fresh.pinned, fresh.sort, fresh.page_size));
        assert!(view.widths.is_empty() && view.column_paint.is_empty());
        // What narrows the table is not the look of it, and neither is a cell somebody marked.
        assert_eq!(view.filters, filters);
        assert_eq!(view.cell_paint.len(), 1);
        // Nothing but those two sets it apart from a fresh table.
        assert_eq!(TableView { filters: Filters::default(), cell_paint: HashMap::new(), ..view }, fresh);
    }

    #[test]
    fn a_fresh_layout_is_told_from_one_that_was_arranged() {
        let fresh = TableView::default;
        assert!(fresh().has_fresh_layout() && fresh().has_ledger_columns());
        // Filters and painted cells are not the layout.
        let mut marked = TableView { filters: Filters { currency: Some("USD".into()), ..Default::default() }, ..fresh() };
        marked.cell_paint.insert((1, Column::Amount), Paint { fill: Some(0), align: None });
        assert!(marked.has_fresh_layout());
        // Hidden columns are the same set in whatever order they were hidden.
        let mut reversed = fresh();
        reversed.hidden.reverse();
        assert!(reversed.has_ledger_columns() && reversed.has_fresh_layout());
        // Anything the view sheet can change is.
        let mut moved = fresh();
        moved.move_column(Column::Amount, Step::Up);
        let mut wide = fresh();
        wide.widths.insert(Column::Amount, 200.0);
        let mut painted = fresh();
        painted.column_paint.insert(Column::Amount, Paint { fill: Some(1), align: None });
        let arranged = [
            moved,
            TableView { hidden: Vec::new(), ..fresh() },
            wide,
            painted,
            TableView { density: Density::Ultra, ..fresh() },
            TableView { pinned: 0, ..fresh() },
            TableView { sort: Sort { column: Column::TransactionDate, descending: true }, ..fresh() },
            TableView { page_size: PageSize::Fifty, ..fresh() },
        ];
        for (index, view) in arranged.iter().enumerate() {
            assert!(!view.has_fresh_layout(), "case {index}");
        }
        // Widths and density leave the columns the ledger's.
        assert!(arranged[2].has_ledger_columns() && arranged[4].has_ledger_columns());
        assert!(!arranged[0].has_ledger_columns() && !arranged[1].has_ledger_columns());
    }

    #[test]
    fn pinned_columns_are_counted_among_the_shown() {
        let mut view = TableView::default();
        assert_eq!(view.pinned_shown(), MAX_PINNED);
        view.hidden = columns::ALL.iter().copied().filter(|c| *c != LOCKED).collect();
        assert_eq!(view.visible(), [LOCKED]);
        assert_eq!(view.pinned_shown(), 1);
        view.pinned = 0;
        assert_eq!(view.pinned_shown(), 0);
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
    }

    #[test]
    fn the_card_and_p2p_read_as_in_the_ledger() {
        let tx = |id, card: &str, p2p| tbs_core::model::Transaction { id, card_last_4: Some(card.into()), is_p2p: p2p, ..Default::default() };
        let data = Dataset::build(vec![tx(1, "0626", true), tx(2, "4483", false)]);
        let table = TableView::default();
        let rows = [0u32, 1];
        let grid = TxGrid { data: &data, rows: &rows, offset: 0, total: 2, columns: &columns::ALL, table: &table, highlight: None };
        // «ПК» is text: the zero in front is part of the card.
        assert_eq!(grid.cell(0, Column::CardLast4.index()), "0626");
        // «П2П» is 1 or an empty cell, never a word.
        assert_eq!(grid.cell(0, Column::IsP2p.index()), "1");
        assert_eq!(grid.cell(1, Column::IsP2p.index()), "");
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
