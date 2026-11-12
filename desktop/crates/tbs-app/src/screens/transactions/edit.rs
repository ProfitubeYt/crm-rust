//! Changing the data: what each cell accepts, how a change reaches the server,
//! copy and paste, deletion, undo and redo.

use std::collections::BTreeMap;

use chrono::{NaiveDate, NaiveDateTime, Timelike};
use iced::widget::{self, Id, column, container, mouse_area, opaque, operation, pin, stack, text};
use iced::{Fill, Rectangle, Task};
use serde_json::{Map, Value};
use tbs_core::api::{BulkReply, delete_error_text};
use tbs_core::columns::Column;
use tbs_core::format;
use tbs_core::model::Transaction;

use super::{Msg as Screen, Transactions};
use crate::ctx::{Ctx, Request, text_error};
use crate::theme::{self, Tone, size, space};
use crate::ui::{self, El};

const HISTORY_LIMIT: usize = 50;
const EDITOR_MIN_WIDTH: f32 = 170.0;
const EDITOR_MAX_LIST: f32 = 280.0;

pub const TYPES: [(&str, &str); 4] = [("DEBIT", "Списание"), ("CREDIT", "Пополнение"), ("CONVERSION", "Конверсия"), ("REVERSAL", "Отмена")];
pub const CURRENCIES: [&str; 2] = ["UZS", "USD"];
pub const SOURCES: [(&str, &str); 3] = [("AUTO", "Авто"), ("SMS", "СМС"), ("MANUAL", "Ручной")];

fn editor_id() -> Id {
    Id::new("cell-editor")
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Text,
    Number,
    Time,
    DateTime,
    Month,
    Select(Vec<(String, String)>),
}

/// These cells open the detail sheet instead of an editor. «Операция» only
/// selects: it is read off the receipt, and «Тип» is where the type is set.
/// «(Без дубля)» only selects too: it is worked out from the times of the
/// receipts, not typed.
pub fn read_only(column: Column) -> bool {
    matches!(column, Column::RowNumber | Column::OperationNumber | Column::TransactionDate | Column::Day | Column::Operation | Column::DateTimeDedup)
}

/// The dictionary choices for the annotation columns.
#[derive(Clone, Debug, Default)]
pub struct Choices {
    pub persons: Vec<(String, String)>,
    pub articles: Vec<(String, String)>,
}

fn with_blank(items: &[(String, String)]) -> Vec<(String, String)> {
    let mut out = vec![(String::new(), format::EMPTY.to_string())];
    out.extend(items.iter().cloned());
    out
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items.iter().map(|(value, name)| (value.to_string(), name.to_string())).collect()
}

pub fn kind_of(column: Column, choices: &Choices) -> Kind {
    match column {
        Column::Amount | Column::BalanceAfter => Kind::Number,
        Column::Time => Kind::Time,
        Column::DateTime => Kind::DateTime,
        Column::AnnotationPeriod => Kind::Month,
        Column::TransactionType => Kind::Select(pairs(&TYPES)),
        Column::Currency => Kind::Select(CURRENCIES.iter().map(|c| (c.to_string(), c.to_string())).collect()),
        Column::SourceType => Kind::Select(pairs(&SOURCES)),
        Column::AnnotationPersonCode => Kind::Select(with_blank(&choices.persons)),
        Column::AnnotationArticleCode => Kind::Select(with_blank(&choices.articles)),
        _ => Kind::Text,
    }
}

/// The name the server knows the column's value by.
pub fn field_of(column: Column) -> Option<&'static str> {
    match column {
        Column::DateTime | Column::Time => Some("transaction_date"),
        Column::TransactionType => Some("transaction_type"),
        Column::SourceType => Some("source_type"),
        Column::IsP2p => Some("is_p2p"),
        other => other.edit_field(),
    }
}

/// `2026-03-01` is shown and typed as `03.2026`, in the cell and in its editor alike.
fn month_text(raw: &str) -> String {
    format::period(raw)
}

/// The text an editor opens with, and the text a copy puts on the clipboard.
pub fn edit_text(column: Column, tx: &Transaction, when: Option<NaiveDateTime>) -> String {
    match column {
        Column::DateTime => when.map(|w| w.format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default(),
        Column::Time => when.map(|w| w.format("%H:%M").to_string()).unwrap_or_default(),
        Column::AnnotationPeriod => tx.annotation_period.as_deref().map(month_text).unwrap_or_default(),
        Column::TransactionType => tx.transaction_type.clone().unwrap_or_default(),
        Column::SourceType => tx.source_type.clone().unwrap_or_default(),
        Column::IsP2p => if tx.is_p2p { "да" } else { "нет" }.to_string(),
        other => other.raw_value(tx),
    }
}

fn clean_number(input: &str) -> String {
    input.chars().filter(|c| !c.is_whitespace() && *c != '\u{a0}').map(|c| if c == ',' { '.' } else { c }).collect()
}

fn parse_amount(input: &str) -> Result<String, String> {
    let value = clean_number(input);
    match value.parse::<f64>() {
        Ok(number) if number.is_finite() && number >= 0.0 => Ok(value),
        Ok(_) => Err("Значение не может быть отрицательным".into()),
        Err(_) => Err("Введите число".into()),
    }
}

fn parse_when(input: &str) -> Option<NaiveDateTime> {
    const FORMATS: [&str; 6] = ["%d.%m.%Y %H:%M:%S", "%d.%m.%Y %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"];
    FORMATS.iter().find_map(|f| NaiveDateTime::parse_from_str(input, f).ok())
}

fn iso(when: NaiveDateTime) -> Value {
    Value::String(when.format("%Y-%m-%dT%H:%M:%S").to_string())
}

fn parse_month(input: &str) -> Option<String> {
    let (year, month) = match input.split_once('.') {
        Some((month, year)) => (year, month),
        None => input.get(0..7)?.split_once('-')?,
    };
    let date = NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, 1)?;
    Some(date.format("%Y-%m-01").to_string())
}

fn limited(input: &str, limit: usize) -> Result<Value, String> {
    if input.chars().count() > limit { Err(format!("Не больше {limit} символов")) } else { Ok(Value::String(input.to_string())) }
}

fn optional(input: &str, limit: usize) -> Result<Value, String> {
    if input.is_empty() { Ok(Value::Null) } else { limited(input, limit) }
}

fn digits(input: &str, range: std::ops::RangeInclusive<usize>) -> bool {
    range.contains(&input.len()) && input.bytes().all(|b| b.is_ascii_digit())
}

/// Turns typed text into the server's field and value, or says what is wrong.
pub fn to_field(column: Column, input: &str, current: Option<NaiveDateTime>) -> Result<(&'static str, Value), String> {
    let input = input.trim();
    let field = field_of(column).ok_or("Это поле нельзя изменить")?;
    let value = match column {
        Column::Amount => {
            if input.is_empty() {
                return Err("Укажите сумму".into());
            }
            Value::String(parse_amount(input)?)
        }
        Column::BalanceAfter => {
            if input.is_empty() {
                Value::Null
            } else {
                Value::String(parse_amount(input)?)
            }
        }
        Column::DateTime => iso(parse_when(input).ok_or("Формат: ДД.ММ.ГГГГ ЧЧ:ММ")?),
        Column::Time => {
            let current = current.ok_or("У записи нет даты")?;
            let time = chrono::NaiveTime::parse_from_str(input, "%H:%M").map_err(|_| "Формат: ЧЧ:ММ")?;
            iso(current.date().and_time(time.with_second(0).unwrap_or(time)))
        }
        Column::AnnotationPeriod => {
            if input.is_empty() {
                Value::Null
            } else {
                Value::String(parse_month(input).ok_or("Формат: ММ.ГГГГ")?)
            }
        }
        Column::OperatorRaw => {
            if input.is_empty() {
                return Err("Укажите оператора".into());
            }
            limited(input, 255)?
        }
        Column::ApplicationMapped => optional(input, 100)?,
        Column::ReceiverName => optional(input, 255)?,
        Column::ReceiverCard => {
            if input.is_empty() {
                Value::Null
            } else if digits(input, 4..=32) {
                Value::String(input.to_string())
            } else {
                return Err("От 4 до 32 цифр".into());
            }
        }
        Column::CardLast4 => {
            if digits(input, 4..=4) {
                Value::String(input.to_string())
            } else {
                return Err("Ровно 4 цифры".into());
            }
        }
        Column::AnnotationDetails => optional(input, 500)?,
        Column::AnnotationPersonCode => optional(input, 32)?,
        Column::AnnotationArticleCode => optional(input, 16)?,
        Column::IsP2p => match input.to_lowercase().as_str() {
            "1" | "true" | "да" | "yes" => Value::Bool(true),
            "" | "0" | "false" | "нет" | "no" => Value::Bool(false),
            _ => return Err("Да или нет".into()),
        },
        Column::Currency => {
            let code = input.to_uppercase();
            if !CURRENCIES.contains(&code.as_str()) {
                return Err("UZS или USD".into());
            }
            Value::String(code)
        }
        Column::TransactionType => Value::String(code_of(&TYPES, input).ok_or("Неизвестный тип")?),
        Column::SourceType => Value::String(code_of(&SOURCES, input).ok_or("Неизвестный источник")?),
        _ => return Err("Это поле нельзя изменить".into()),
    };
    Ok((field, value))
}

/// Pasted cells may carry a code or the label a person sees.
fn code_of(items: &[(&str, &str)], input: &str) -> Option<String> {
    let lower = input.to_lowercase();
    items.iter().find(|(code, name)| code.to_lowercase() == lower || name.to_lowercase() == lower).map(|(code, _)| code.to_string())
}

fn opt_string(value: &Option<String>) -> Value {
    value.clone().map_or(Value::Null, Value::String)
}

/// The value a field holds now, in the form the server takes it back.
pub fn raw_field(field: &str, tx: &Transaction) -> Value {
    match field {
        "transaction_date" => opt_string(&tx.transaction_date),
        "operator_raw" => opt_string(&tx.operator_raw),
        "application_mapped" => opt_string(&tx.application_mapped),
        "description" => Value::String(tx.description.clone().unwrap_or_default()),
        "amount" => opt_string(&tx.amount),
        "balance_after" => opt_string(&tx.balance_after),
        "card_last_4" => opt_string(&tx.card_last_4),
        "receiver_name" => opt_string(&tx.receiver_name),
        "receiver_card" => opt_string(&tx.receiver_card),
        "transaction_type" => opt_string(&tx.transaction_type),
        "currency" => opt_string(&tx.currency),
        "source_type" => opt_string(&tx.source_type),
        "is_p2p" => Value::Bool(tx.is_p2p),
        "annotation_details" => opt_string(&tx.annotation_details),
        "annotation_person_code" => opt_string(&tx.annotation_person_code),
        "annotation_period" => opt_string(&tx.annotation_period),
        "annotation_article_code" => opt_string(&tx.annotation_article_code),
        _ => Value::Null,
    }
}

/// Writes a value into the local row, with what the server would derive from it.
pub fn apply_field(tx: &mut Transaction, field: &str, value: &Value) {
    let text = value.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    match field {
        "transaction_date" => tx.transaction_date = text,
        "operator_raw" => tx.operator_raw = text,
        "application_mapped" => tx.application_mapped = text,
        "description" => tx.description = text,
        "amount" => tx.amount = text,
        "balance_after" => tx.balance_after = text,
        "card_last_4" => tx.card_last_4 = text,
        "receiver_name" => tx.receiver_name = text,
        "receiver_card" => tx.receiver_card = text,
        "currency" => tx.currency = text,
        "is_p2p" => tx.is_p2p = value.as_bool().unwrap_or(false),
        "annotation_details" => tx.annotation_details = text,
        "annotation_person_code" => tx.annotation_person_code = text,
        "annotation_period" => tx.annotation_period = text,
        "annotation_article_code" => tx.annotation_article_code = text,
        "transaction_type" => {
            tx.transaction_type_display = format::type_label(text.as_deref()).map(str::to_string).or_else(|| text.clone());
            tx.transaction_type = text;
        }
        "source_type" => {
            let keep = tx.source_channel.clone().filter(|c| c != "MANUAL");
            let channel = if text.as_deref() == Some("MANUAL") { "MANUAL".to_string() } else { keep.unwrap_or_else(|| "SMS".into()) };
            tx.source_display = Some(
                match channel.as_str() {
                    "TELEGRAM" => "Телеграм",
                    "SMS" => "СМС",
                    "MANUAL" => "Ручной",
                    other => other,
                }
                .to_string(),
            );
            tx.source_channel = Some(channel);
            tx.source_type = text;
        }
        _ => {}
    }
}

pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n').map(|line| line.trim_end_matches('\r').split('\t').map(str::to_string).collect()).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub id: i64,
    pub field: &'static str,
    pub old: Value,
    pub new: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Edit(Vec<Change>),
    Delete(usize),
}

#[derive(Default)]
pub struct History {
    done: Vec<Action>,
    undone: Vec<Action>,
}

impl History {
    /// A new action ends any chain of redos.
    pub fn push(&mut self, action: Action) {
        self.undone.clear();
        self.remember(action);
    }

    fn remember(&mut self, action: Action) {
        self.done.push(action);
        if self.done.len() > HISTORY_LIMIT {
            self.done.remove(0);
        }
    }
}

/// One request body per row, whichever columns changed.
fn grouped(changes: &[Change], pick: impl Fn(&Change) -> &Value) -> Vec<(i64, Map<String, Value>)> {
    let mut rows: BTreeMap<i64, Map<String, Value>> = BTreeMap::new();
    for change in changes {
        rows.entry(change.id).or_default().insert(change.field.to_string(), pick(change).clone());
    }
    rows.into_iter().collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Undo,
    Redo,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Value(String),
    Commit,
    Cancel,
    Pick(String),
    Saved(Change, Result<Option<Transaction>, String>),
    Applied(Vec<Change>, Direction, Result<BulkReply, String>),
    Pasted(Option<String>),
    AskDelete(Vec<i64>),
    ConfirmDelete,
    CancelDelete,
    Deleted(Vec<i64>, Result<BulkReply, String>),
}

pub struct Editor {
    pub id: i64,
    pub column: Column,
    pub value: String,
    pub original: String,
    pub rect: Rectangle,
    pub kind: Kind,
    pub error: Option<String>,
    pub saving: bool,
}

pub struct DeleteAsk {
    pub ids: Vec<i64>,
    pub title: String,
    pub body: String,
}

impl Transactions {
    pub(super) fn row_at(&self, row: usize) -> Option<&tbs_core::query::Row> {
        self.rows.get(row).map(|i| &self.data.rows[*i as usize])
    }

    pub(super) fn begin_edit(&mut self, row: usize, col: usize, rect: Rectangle, seed: Option<String>, ctx: &mut Ctx) -> Task<Screen> {
        if self.editing_blocked() {
            return Task::none();
        }
        let Some(&column) = self.columns.get(col) else { return Task::none() };
        if read_only(column) {
            return Task::none();
        }
        let Some(item) = self.row_at(row) else { return Task::none() };
        let (id, when) = (item.tx.id, item.when);
        if column == Column::IsP2p {
            let (old, new) = (item.tx.is_p2p, !item.tx.is_p2p);
            let change = Change { id, field: "is_p2p", old: Value::Bool(old), new: Value::Bool(new) };
            return self.send_change(change, ctx);
        }
        let original = edit_text(column, &item.tx, when);
        let kind = kind_of(column, &self.choices);
        let typed = seed.is_some();
        let is_select = matches!(kind, Kind::Select(_));
        let value = if is_select { original.clone() } else { seed.unwrap_or_else(|| original.clone()) };
        self.editor = Some(Editor { id, column, value, original, rect, kind, error: None, saving: false });
        if is_select {
            return Task::none();
        }
        let focus = operation::focus(editor_id());
        let cursor = if typed { operation::move_cursor_to_end(editor_id()) } else { operation::select_all(editor_id()) };
        focus.chain(cursor)
    }

    fn send_change(&mut self, change: Change, ctx: &Ctx) -> Task<Screen> {
        let api = ctx.api.clone();
        let mut body = Map::new();
        body.insert(change.field.to_string(), change.new.clone());
        let id = change.id;
        Task::perform(
            async move { (change, api.update_transaction(id, &body).await.map(|r| r.transaction).map_err(text_error)) },
            |(change, result)| Screen::Edit(Msg::Saved(change, result)),
        )
    }

    fn commit(&mut self, ctx: &mut Ctx) -> Task<Screen> {
        let Some(editor) = &self.editor else { return Task::none() };
        if editor.saving {
            return Task::none();
        }
        if editor.value.trim() == editor.original.trim() {
            self.editor = None;
            return Task::none();
        }
        let Some(row) = self.data.by_id(editor.id) else {
            self.editor = None;
            return Task::none();
        };
        match to_field(editor.column, &editor.value, row.when) {
            Ok((field, new)) => {
                let change = Change { id: editor.id, field, old: raw_field(field, &row.tx), new };
                if let Some(editor) = &mut self.editor {
                    editor.saving = true;
                    editor.error = None;
                }
                self.send_change(change, ctx)
            }
            Err(reason) => {
                if let Some(editor) = &mut self.editor {
                    editor.error = Some(reason);
                }
                Task::none()
            }
        }
    }

    /// Takes rows the server confirmed into the table and the local cache.
    pub(super) fn take_rows(&mut self, rows: Vec<Transaction>, ctx: &Ctx) -> Task<Screen> {
        for tx in &rows {
            self.data.upsert(tx.clone());
        }
        self.refresh_keeping_selection();
        let Some(store) = ctx.store.clone() else { return Task::none() };
        Task::perform(
            async move {
                let _ = tokio::task::spawn_blocking(move || {
                    for tx in &rows {
                        if let Err(error) = store.put_transaction(tx) {
                            tracing::warn!("cache not updated: {error}");
                        }
                    }
                })
                .await;
            },
            |_| Screen::Noop,
        )
    }

    fn applied_rows(&self, changes: &[Change], failed: &[i64], direction: Direction) -> Vec<Transaction> {
        let mut rows: BTreeMap<i64, Transaction> = BTreeMap::new();
        for change in changes.iter().filter(|c| !failed.contains(&c.id)) {
            let Some(row) = self.data.by_id(change.id) else { continue };
            let tx = rows.entry(change.id).or_insert_with(|| row.tx.clone());
            let value = if direction == Direction::Undo { &change.old } else { &change.new };
            apply_field(tx, change.field, value);
        }
        rows.into_values().collect()
    }

    fn send_changes(&self, changes: Vec<Change>, direction: Direction, ctx: &Ctx) -> Task<Screen> {
        let api = ctx.api.clone();
        let body = if direction == Direction::Undo { grouped(&changes, |c| &c.old) } else { grouped(&changes, |c| &c.new) };
        Task::perform(async move { (changes, api.bulk_update(&body).await.map_err(text_error)) }, move |(changes, result)| {
            Screen::Edit(Msg::Applied(changes, direction, result))
        })
    }

    pub(super) fn undo(&mut self, ctx: &mut Ctx) -> Task<Screen> {
        match self.history.done.pop() {
            Some(Action::Edit(changes)) => self.send_changes(changes, Direction::Undo, ctx),
            Some(Action::Delete(count)) => {
                let text = if count == 1 { "Невозможно восстановить удаленную запись" } else { "Невозможно восстановить удаленные записи" };
                ctx.toast(Tone::Warning, text);
                Task::none()
            }
            None => Task::none(),
        }
    }

    pub(super) fn redo(&mut self, ctx: &mut Ctx) -> Task<Screen> {
        match self.history.undone.pop() {
            Some(Action::Edit(changes)) => self.send_changes(changes, Direction::Redo, ctx),
            _ => Task::none(),
        }
    }

    pub(super) fn copy_selection(&self) -> Option<String> {
        let cells = self.selection.cells();
        if cells.is_empty() {
            return None;
        }
        let mut lines: BTreeMap<usize, Vec<(usize, String)>> = BTreeMap::new();
        for (row, col) in cells {
            let (Some(item), Some(&column)) = (self.row_at(row), self.columns.get(col)) else { continue };
            let value = match column {
                Column::RowNumber => (self.position(row) + 1).to_string(),
                Column::OperationNumber => super::table::operation_number(&self.table.sort, self.matched_len(), self.position(row)).to_string(),
                Column::TransactionDate | Column::Day | Column::Operation | Column::DateTimeDedup => item.cell(column).to_string(),
                _ => edit_text(column, &item.tx, item.when),
            };
            lines.entry(row).or_default().push((col, value));
        }
        let rows: Vec<String> = lines
            .into_values()
            .map(|mut line| {
                line.sort_by_key(|(col, _)| *col);
                line.into_iter().map(|(_, value)| value).collect::<Vec<_>>().join("\t")
            })
            .collect();
        Some(rows.join("\n"))
    }

    /// What a pasted block changes, and how many cells could not be read.
    fn plan_paste(&self, text: &str) -> (Vec<Change>, usize) {
        let Some(&(top, left)) = self.selection.cells().first() else { return (Vec::new(), 0) };
        let (mut changes, mut skipped) = (Vec::new(), 0);
        for (y, line) in parse_tsv(text).iter().enumerate() {
            let Some(item) = self.row_at(top + y) else { break };
            for (x, value) in line.iter().enumerate() {
                let Some(&column) = self.columns.get(left + x) else { break };
                if read_only(column) || value.trim() == edit_text(column, &item.tx, item.when).trim() {
                    continue;
                }
                match to_field(column, value, item.when) {
                    Ok((field, new)) => {
                        let old = raw_field(field, &item.tx);
                        if old != new {
                            changes.push(Change { id: item.tx.id, field, old, new });
                        }
                    }
                    Err(_) => skipped += 1,
                }
            }
        }
        (changes, skipped)
    }

    pub(super) fn ask_delete(&mut self, ids: Vec<i64>) {
        if ids.is_empty() || self.editing_blocked() {
            return;
        }
        let count = ids.len();
        let noun = format::plural(count, "запись", "записи", "записей");
        self.delete_ask = Some(DeleteAsk {
            title: format!("Удалить {} {noun}?", format::count(count)),
            body: "Удаление нельзя отменить.".to_string(),
            ids,
        });
    }

    pub(super) fn selected_ids(&self) -> Vec<i64> {
        self.selection.rows().into_iter().filter_map(|row| self.row_at(row).map(|r| r.tx.id)).collect()
    }

    pub(super) fn edit_update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Screen> {
        match message {
            Msg::Value(value) => {
                if let Some(editor) = &mut self.editor {
                    editor.value = value;
                    editor.error = None;
                }
            }
            Msg::Commit => return self.commit(ctx),
            Msg::Cancel => self.editor = None,
            Msg::Pick(value) => {
                if let Some(editor) = &mut self.editor {
                    editor.value = value;
                }
                return self.commit(ctx);
            }
            Msg::Saved(change, result) => match result {
                Ok(fresh) => {
                    self.editor = None;
                    // The server writes a description after it builds the reply.
                    let described = change.field == "description";
                    let confirmed = fresh.filter(|_| !described);
                    let tx = confirmed.or_else(|| {
                        let mut tx = self.data.by_id(change.id)?.tx.clone();
                        apply_field(&mut tx, change.field, &change.new);
                        Some(tx)
                    });
                    if described {
                        ctx.request(Request::TransactionsChanged);
                    }
                    self.history.push(Action::Edit(vec![change]));
                    return self.take_rows(tx.into_iter().collect(), ctx);
                }
                Err(reason) => match &mut self.editor {
                    Some(editor) => {
                        editor.saving = false;
                        editor.error = Some(reason);
                    }
                    None => ctx.toast(Tone::Danger, reason),
                },
            },
            Msg::Applied(changes, direction, result) => match result {
                Ok(reply) => {
                    let rows = self.applied_rows(&changes, &reply.failed_ids, direction);
                    if direction == Direction::Forward {
                        ctx.toast(Tone::Success, pasted_text(rows.len()));
                    }
                    if !reply.failed_ids.is_empty() {
                        let reason = reply.errors.first().map(|e| delete_error_text(e)).unwrap_or_default();
                        ctx.toast(Tone::Warning, format!("Не сохранено строк: {}. {reason}", reply.failed_ids.len()));
                    }
                    let kept: Vec<Change> = changes.into_iter().filter(|c| !reply.failed_ids.contains(&c.id)).collect();
                    // A description belongs to the operator, so other rows change too.
                    if kept.iter().any(|c| c.field == "description") {
                        ctx.request(Request::TransactionsChanged);
                    }
                    if !kept.is_empty() {
                        match direction {
                            Direction::Forward => self.history.push(Action::Edit(kept)),
                            Direction::Undo => self.history.undone.push(Action::Edit(kept)),
                            Direction::Redo => self.history.remember(Action::Edit(kept)),
                        }
                    }
                    return self.take_rows(rows, ctx);
                }
                Err(reason) => {
                    let text = if direction == Direction::Forward { "Не удалось вставить из буфера обмена" } else { "Не удалось применить изменение" };
                    ctx.toast(Tone::Danger, format!("{text}: {reason}"));
                }
            },
            Msg::Pasted(None) => ctx.toast(Tone::Danger, "Не удалось вставить из буфера обмена"),
            Msg::Pasted(Some(text)) => {
                let (changes, skipped) = self.plan_paste(&text);
                if skipped > 0 {
                    ctx.toast(Tone::Warning, format!("Пропущено значений: {skipped}. Формат не подходит к колонке."));
                }
                if !changes.is_empty() {
                    return self.send_changes(changes, Direction::Forward, ctx);
                }
            }
            Msg::AskDelete(ids) => self.ask_delete(ids),
            Msg::CancelDelete => self.delete_ask = None,
            Msg::ConfirmDelete => {
                let Some(ask) = self.delete_ask.take() else { return Task::none() };
                let api = ctx.api.clone();
                return Task::perform(async move { (ask.ids.clone(), api.bulk_delete(&ask.ids).await.map_err(text_error)) }, |(ids, result)| {
                    Screen::Edit(Msg::Deleted(ids, result))
                });
            }
            Msg::Deleted(ids, result) => match result {
                Ok(reply) => {
                    let gone: Vec<i64> = ids.iter().copied().filter(|id| !reply.failed_ids.contains(id)).collect();
                    let reason = reply.errors.first().map(|e| delete_error_text(e)).unwrap_or_default();
                    if gone.is_empty() {
                        ctx.toast(Tone::Danger, format!("Не удалось удалить: {reason}"));
                        return Task::none();
                    }
                    let noun = format::plural(gone.len(), "запись", "записи", "записей");
                    if reply.failed_ids.is_empty() {
                        ctx.toast(Tone::Success, format!("Удалено {} {noun}", format::count(gone.len())));
                    } else {
                        ctx.toast(Tone::Warning, format!("Удалено {}, не удалось {}: {reason}", gone.len(), reply.failed_ids.len()));
                    }
                    self.data.remove(&gone);
                    self.refresh_rows();
                    self.history.push(Action::Delete(gone.len()));
                    let Some(store) = ctx.store.clone() else { return Task::none() };
                    return Task::perform(
                        async move {
                            let _ = tokio::task::spawn_blocking(move || store.remove_transactions(&gone)).await;
                        },
                        |_| Screen::Noop,
                    );
                }
                Err(reason) => ctx.toast(Tone::Danger, format!("Ошибка при удалении: {reason}")),
            },
        }
        Task::none()
    }

    /// The in-place editor, drawn over the cell it edits.
    pub(super) fn editor_view(&self, ctx: &Ctx) -> Option<El<'_, Screen>> {
        let editor = self.editor.as_ref()?;
        let rect = editor.rect;
        let room = ctx.window.width - space::SM;
        let dismiss = if matches!(editor.kind, Kind::Select(_)) { Msg::Cancel } else { Msg::Commit };
        let catcher = opaque(mouse_area(container(widget::Space::new()).width(Fill).height(Fill)).on_press(Screen::Edit(dismiss)));

        let width = rect.width.max(EDITOR_MIN_WIDTH);
        let x = rect.x.min(room - width).max(0.0);
        let body: El<'_, Screen> = match &editor.kind {
            Kind::Select(options) => {
                let mut list = column![].spacing(space::XXS);
                for (value, name) in options {
                    let chosen = *value == editor.value;
                    let caption = if chosen { ui::strong(name.as_str()) } else { ui::label(name.as_str()) };
                    list = list.push(
                        widget::button(caption)
                            .width(Fill)
                            .padding([space::XS, space::SM])
                            .style(theme::btn_menu(false))
                            .on_press(Screen::Edit(Msg::Pick(value.clone()))),
                    );
                }
                let list = ui::scroll(list).height(iced::Length::Shrink);
                container(list).padding(space::XS).width(width).max_height(EDITOR_MAX_LIST).style(theme::popover).into()
            }
            _ => {
                let input = widget::text_input("", &editor.value)
                    .id(editor_id())
                    .size(size::SM)
                    .padding([space::XXS, space::XS])
                    .style(theme::input_cell)
                    .on_input(|value| Screen::Edit(Msg::Value(value)))
                    .on_submit(Screen::Edit(Msg::Commit));
                let hint: Option<&str> = match editor.kind {
                    Kind::DateTime => Some("ДД.ММ.ГГГГ ЧЧ:ММ"),
                    Kind::Time => Some("ЧЧ:ММ"),
                    Kind::Month => Some("ММ.ГГГГ"),
                    _ => None,
                };
                let mut stacked = column![container(input).width(width).height(rect.height.max(26.0)).center_y(rect.height.max(26.0))];
                if let Some(reason) = &editor.error {
                    stacked = stacked.push(container(ui::toned(reason.as_str(), Tone::Danger)).padding([space::XS, space::SM]).width(width).style(theme::popover));
                } else if let Some(hint) = hint {
                    stacked = stacked.push(container(ui::faint(hint)).padding([space::XXS, space::SM]).width(width).style(theme::popover));
                }
                if editor.saving {
                    stacked = stacked.push(container(text("Сохраняем…").size(size::XS)).padding([space::XXS, space::SM]).width(width).style(theme::popover));
                }
                stacked.into()
            }
        };
        Some(stack![catcher, pin(body).x(x).y(rect.y)].into())
    }
}

pub fn pasted_text(rows: usize) -> String {
    format!("Вставлено {} {}", format::count(rows), format::plural(rows, "строка", "строки", "строк"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn when() -> NaiveDateTime {
        format::parse_datetime("2026-03-05T14:30:45").unwrap()
    }

    #[test]
    fn amounts_accept_spaces_and_commas() {
        assert_eq!(to_field(Column::Amount, " 1 250\u{a0}000,50 ", None).unwrap(), ("amount", json!("1250000.50")));
        assert!(to_field(Column::Amount, "", None).is_err());
        assert!(to_field(Column::Amount, "-5", None).is_err());
        assert!(to_field(Column::Amount, "abc", None).is_err());
        assert_eq!(to_field(Column::BalanceAfter, "", None).unwrap(), ("balance_after", Value::Null));
    }

    #[test]
    fn dates_keep_what_was_not_typed() {
        assert_eq!(to_field(Column::DateTime, "01.02.2026 09:05", None).unwrap(), ("transaction_date", json!("2026-02-01T09:05:00")));
        assert_eq!(to_field(Column::DateTime, "2026-02-01T09:05:10", None).unwrap().1, json!("2026-02-01T09:05:10"));
        assert!(to_field(Column::DateTime, "вчера", None).is_err());
        assert_eq!(to_field(Column::Time, "09:15", Some(when())).unwrap().1, json!("2026-03-05T09:15:00"));
        assert!(to_field(Column::Time, "09:15", None).is_err());
    }

    #[test]
    fn periods_are_stored_as_the_first_of_the_month() {
        assert_eq!(to_field(Column::AnnotationPeriod, "03.2026", None).unwrap().1, json!("2026-03-01"));
        assert_eq!(to_field(Column::AnnotationPeriod, "2026-03", None).unwrap().1, json!("2026-03-01"));
        assert_eq!(to_field(Column::AnnotationPeriod, "", None).unwrap().1, Value::Null);
        assert!(to_field(Column::AnnotationPeriod, "13.2026", None).is_err());
        assert_eq!(month_text("2026-03-01"), "03.2026");
    }

    #[test]
    fn cards_follow_the_server_rules() {
        assert!(to_field(Column::CardLast4, "1234", None).is_ok());
        assert!(to_field(Column::CardLast4, "123", None).is_err());
        assert!(to_field(Column::CardLast4, "12a4", None).is_err());
        assert!(to_field(Column::ReceiverCard, "860012345678", None).is_ok());
        assert!(to_field(Column::ReceiverCard, "123", None).is_err());
        assert_eq!(to_field(Column::ReceiverCard, "", None).unwrap().1, Value::Null);
    }

    #[test]
    fn pasted_labels_become_codes() {
        assert_eq!(to_field(Column::TransactionType, "Пополнение", None).unwrap().1, json!("CREDIT"));
        assert_eq!(to_field(Column::TransactionType, "debit", None).unwrap().1, json!("DEBIT"));
        assert!(to_field(Column::TransactionType, "что-то", None).is_err());
        assert_eq!(to_field(Column::SourceType, "Ручной", None).unwrap().1, json!("MANUAL"));
        assert_eq!(to_field(Column::Currency, "usd", None).unwrap().1, json!("USD"));
        assert_eq!(to_field(Column::IsP2p, "да", None).unwrap().1, json!(true));
        assert_eq!(to_field(Column::IsP2p, "нет", None).unwrap().1, json!(false));
    }

    #[test]
    fn long_text_is_refused_before_the_server_does() {
        assert!(to_field(Column::OperatorRaw, &"я".repeat(256), None).is_err());
        assert!(to_field(Column::OperatorRaw, &"я".repeat(255), None).is_ok());
        assert!(to_field(Column::OperatorRaw, "", None).is_err());
    }

    #[test]
    fn read_only_columns_have_no_field() {
        for column in [Column::RowNumber, Column::OperationNumber, Column::TransactionDate, Column::Day, Column::Operation, Column::DateTimeDedup] {
            assert!(read_only(column));
            assert!(to_field(column, "1", None).is_err());
        }
    }

    #[test]
    fn the_second_date_copies_as_it_reads_and_is_not_pasted_over() {
        let dir = tempfile::tempdir().unwrap();
        let (mut screen, _) = Transactions::new(&Ctx::for_tests(dir.path()));
        let receipt = |id| Transaction { id, transaction_date: Some("2026-01-12T10:00:00".into()), ..Default::default() };
        screen.data = tbs_core::query::Dataset::build(vec![receipt(1), receipt(2)]);
        screen.rows = vec![0, 1];
        screen.columns = vec![Column::DateTime, Column::DateTimeDedup];
        screen.selection = crate::grid::Selection::single(0, 0);
        screen.selection.extend_to((1, 1));
        // The first date as its editor has it; the second as the cell reads, the repeat moved on.
        assert_eq!(screen.copy_selection().as_deref(), Some("12.01.2026 10:00\t12.01.2026 10:00:00\n12.01.2026 10:00\t12.01.2026 10:00:08"));

        // Pasted over with anything, the column is passed by: nothing changes and nothing counts as unread.
        let (changes, skipped) = screen.plan_paste("12.01.2026 10:00\t13.01.2026 11:00:00\n12.01.2026 10:00\tчто-то");
        assert_eq!((changes.len(), skipped), (0, 0));
    }

    #[test]
    fn the_operation_copies_as_it_reads_and_is_not_pasted_over() {
        let dir = tempfile::tempdir().unwrap();
        let (mut screen, _) = Transactions::new(&Ctx::for_tests(dir.path()));
        let receipt = |id, raw: &str| Transaction { id, raw_message: Some(raw.into()), transaction_type: Some("DEBIT".into()), ..Default::default() };
        screen.data = tbs_core::query::Dataset::build(vec![receipt(1, "\u{1f4b8} Оплата\n➖ 40.000,00 UZS"), receipt(2, "[Photo]")]);
        screen.rows = vec![0, 1];
        screen.columns = vec![Column::Operation, Column::TransactionType];
        screen.selection = crate::grid::Selection::single(0, 0);
        screen.selection.extend_to((1, 1));
        // The headline, or the label that stands in for it; «Тип» still copies as its code.
        assert_eq!(screen.copy_selection().as_deref(), Some("\u{1f4b8} Оплата\tDEBIT\n\u{1f534} Списание\tDEBIT"));

        // Pasted back, or over with anything else, the column is passed by and not counted as unread.
        let (changes, skipped) = screen.plan_paste("\u{1f4b8} Оплата\tCREDIT\nчто-то\tDEBIT");
        assert_eq!(skipped, 0);
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].id, changes[0].field, &changes[0].new), (1, "transaction_type", &json!("CREDIT")));
    }

    #[test]
    fn local_rows_follow_derived_fields() {
        let mut tx = Transaction { source_channel: Some("TELEGRAM".into()), ..Default::default() };
        apply_field(&mut tx, "transaction_type", &json!("CREDIT"));
        assert_eq!(tx.transaction_type_display.as_deref(), Some("Пополнение"));
        apply_field(&mut tx, "source_type", &json!("AUTO"));
        assert_eq!(tx.source_channel.as_deref(), Some("TELEGRAM"));
        apply_field(&mut tx, "source_type", &json!("MANUAL"));
        assert_eq!(tx.source_channel.as_deref(), Some("MANUAL"));
        assert_eq!(tx.source_display.as_deref(), Some("Ручной"));
        apply_field(&mut tx, "source_type", &json!("SMS"));
        assert_eq!(tx.source_channel.as_deref(), Some("SMS"));
    }

    #[test]
    fn a_field_reads_back_what_it_was_given() {
        let mut tx = Transaction::default();
        for (field, value) in [("amount", json!("10.50")), ("operator_raw", json!("Korzinka")), ("is_p2p", json!(true)), ("annotation_period", json!("2026-03-01"))] {
            apply_field(&mut tx, field, &value);
            assert_eq!(raw_field(field, &tx), value);
        }
        apply_field(&mut tx, "balance_after", &Value::Null);
        assert_eq!(raw_field("balance_after", &tx), Value::Null);
    }

    #[test]
    fn clipboard_blocks_split_like_a_spreadsheet() {
        assert_eq!(parse_tsv("a\tb\r\nc\td\r\n"), [["a", "b"], ["c", "d"]]);
        assert_eq!(parse_tsv("один"), [["один"]]);
    }

    #[test]
    fn changes_group_by_row() {
        let change = |id, field, new: &str| Change { id, field, old: Value::Null, new: json!(new) };
        let body = grouped(&[change(2, "amount", "1"), change(1, "currency", "USD"), change(2, "currency", "UZS")], |c| &c.new);
        assert_eq!(body.len(), 2);
        assert_eq!(body[0].0, 1);
        assert_eq!(body[1].1.len(), 2);
    }

    #[test]
    fn history_is_bounded_and_new_work_drops_redo() {
        let mut history = History::default();
        for _ in 0..(HISTORY_LIMIT + 5) {
            history.push(Action::Delete(1));
        }
        assert_eq!(history.done.len(), HISTORY_LIMIT);
        history.undone.push(Action::Delete(2));
        history.push(Action::Delete(3));
        assert!(history.undone.is_empty());
    }
}
