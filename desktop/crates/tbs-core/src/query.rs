//! Filtering and sorting over the local copy of the transactions.
//!
//! Every value the table shows or searches is computed once in
//! [`Dataset::build`], off the UI thread. Scrolling then only reads strings.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::columns::{self, Column, Labels};
use crate::format;
use crate::model::Transaction;

pub struct Row {
    pub tx: Transaction,
    pub when: Option<NaiveDateTime>,
    pub cells: Box<[String]>,
    ts: i64,
    amount_abs: Option<f64>,
    amount_signed: f64,
    balance: f64,
    period: Option<i32>,
    blob: String,
    status_blob: String,
    seller_blob: String,
    amount_text: String,
    date_text: String,
    operator_lc: String,
    app_lc: String,
    details_lc: String,
}

impl Row {
    fn new(tx: Transaction, labels: &Labels) -> Self {
        let when = tx.transaction_date.as_deref().and_then(format::parse_datetime);
        let cells: Box<[String]> = columns::ALL.iter().map(|c| c.display(&tx, when, labels)).collect();
        let parsed = tx.amount.as_deref().and_then(|a| a.trim().parse::<f64>().ok());
        let s = |v: &Option<String>| v.clone().unwrap_or_default();
        // «Операция» as shown: without a headline it is a label the receipt does not contain.
        let operation = Some(cells[Column::Operation.index()].clone());
        // Everything a person may remember a receipt by: who was paid, what the
        // bank wrote, and what the bookkeeper noted next to it.
        let blob = [
            &tx.operator_raw,
            &tx.application_mapped,
            &tx.raw_message,
            &operation,
            &tx.amount,
            &tx.currency,
            &tx.annotation_details,
            &tx.description,
            &tx.receiver_name,
            &tx.receiver_card,
        ]
        .map(|value| value.as_deref().unwrap_or(""))
        .join(" ")
        .to_lowercase();
        let status_blob =
            format!("{} {} {}", s(&tx.parsing_method), s(&tx.transaction_type_display), s(&tx.source_display)).to_lowercase();
        let operator_lc = s(&tx.operator_raw).to_lowercase();
        let app_lc = s(&tx.application_mapped).to_lowercase();
        Row {
            when,
            cells,
            ts: when.map_or(0, |d| d.and_utc().timestamp()),
            amount_abs: parsed.map(f64::abs),
            amount_signed: parsed.unwrap_or(0.0),
            balance: tx.balance_after.as_deref().and_then(|a| a.parse().ok()).unwrap_or(0.0),
            period: tx.annotation_period.as_deref().and_then(period_key),
            seller_blob: format!("{operator_lc} {app_lc}"),
            status_blob,
            blob,
            amount_text: parsed.map(|v| trim_number(v.abs())).unwrap_or_default(),
            date_text: s(&tx.transaction_date).to_lowercase(),
            details_lc: s(&tx.annotation_details).to_lowercase(),
            operator_lc,
            app_lc,
            tx,
        }
    }

    pub fn cell(&self, column: Column) -> &str {
        &self.cells[column.index()]
    }

    /// Rewrites the cells whose text comes from the dictionaries.
    fn relabel(&mut self, labels: &Labels) {
        for column in columns::ALL.into_iter().filter(|c| c.is_labelled()) {
            self.cells[column.index()] = column.display(&self.tx, self.when, labels);
        }
    }

    fn annotated(&self) -> bool {
        let t = &self.tx;
        t.annotation_details.is_some()
            || t.annotation_person_code.is_some()
            || t.annotation_period.is_some()
            || t.annotation_article_code.is_some()
    }
}

/// `5000` rather than `5000.0`, the way JavaScript prints a number.
fn trim_number(v: f64) -> String {
    let text = v.to_string();
    text.strip_suffix(".0").map(str::to_string).unwrap_or(text)
}

/// `MM.YYYY`, or the ISO date the server stores it as, to a sortable year*100+month.
pub fn period_key(raw: &str) -> Option<i32> {
    let raw = raw.trim();
    if let Some((m, y)) = raw.split_once('.') {
        let (m, y) = (m.parse::<i32>().ok()?, y.parse::<i32>().ok()?);
        return (1..=12).contains(&m).then_some(y * 100 + m);
    }
    let mut parts = raw.split('-');
    let (y, m) = (parts.next()?.parse::<i32>().ok()?, parts.next()?.parse::<i32>().ok()?);
    (1..=12).contains(&m).then_some(y * 100 + m)
}

#[derive(Default)]
pub struct Dataset {
    pub rows: Vec<Row>,
    by_id: HashMap<i64, usize>,
    labels: Labels,
    pub operators: Vec<String>,
    pub apps: Vec<String>,
    pub cards: Vec<String>,
    pub methods: Vec<String>,
    pub years: Vec<i32>,
    pub telegram_sources: Vec<(i64, String)>,
}

impl Dataset {
    pub fn build(txs: Vec<Transaction>) -> Self {
        Self::build_with(txs, Labels::default())
    }

    pub fn build_with(txs: Vec<Transaction>, labels: Labels) -> Self {
        let rows: Vec<Row> = txs.into_iter().map(|tx| Row::new(tx, &labels)).collect();
        let by_id = rows.iter().enumerate().map(|(i, r)| (r.tx.id, i)).collect();
        let mut operators = BTreeSet::new();
        let mut apps = BTreeSet::new();
        let mut cards = BTreeSet::new();
        let mut methods = BTreeSet::new();
        let mut years = BTreeSet::new();
        let mut sources = BTreeMap::new();
        for row in &rows {
            let add = |set: &mut BTreeSet<String>, v: &Option<String>| {
                if let Some(v) = v.as_deref().filter(|s| !s.is_empty()) {
                    set.insert(v.to_string());
                }
            };
            add(&mut operators, &row.tx.operator_raw);
            add(&mut apps, &row.tx.application_mapped);
            add(&mut cards, &row.tx.card_last_4);
            add(&mut methods, &row.tx.parsing_method);
            if let Some(d) = row.when {
                years.insert(d.year());
            }
            if let Some(id) = row.tx.source_chat_id {
                sources.entry(id).or_insert_with(|| columns::source_label(&row.tx));
            }
        }
        Dataset {
            rows,
            by_id,
            labels,
            operators: operators.into_iter().collect(),
            apps: apps.into_iter().collect(),
            cards: cards.into_iter().collect(),
            methods: methods.into_iter().collect(),
            years: years.into_iter().rev().collect(),
            telegram_sources: sources.into_iter().collect(),
        }
    }

    pub fn by_id(&self, id: i64) -> Option<&Row> {
        self.by_id.get(&id).map(|i| &self.rows[*i])
    }

    pub fn labels(&self) -> &Labels {
        &self.labels
    }

    /// The dictionaries arrive after the rows, and again whenever someone edits
    /// them. Only the cells they name are rewritten.
    pub fn set_labels(&mut self, labels: Labels) {
        if self.labels == labels {
            return;
        }
        for row in &mut self.rows {
            row.relabel(&labels);
        }
        self.labels = labels;
    }

    /// Replaces a row in place, or appends a new one, without rebuilding the
    /// rest. An edit must not cost a pass over the whole dataset.
    pub fn upsert(&mut self, tx: Transaction) {
        let row = Row::new(tx, &self.labels);
        let add = |list: &mut Vec<String>, value: &Option<String>| {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty())
                && let Err(at) = list.binary_search_by(|probe| probe.as_str().cmp(value))
            {
                list.insert(at, value.to_string());
            }
        };
        add(&mut self.operators, &row.tx.operator_raw);
        add(&mut self.apps, &row.tx.application_mapped);
        add(&mut self.cards, &row.tx.card_last_4);
        add(&mut self.methods, &row.tx.parsing_method);
        if let Some(year) = row.when.map(|d| d.year())
            && !self.years.contains(&year)
        {
            self.years.push(year);
            self.years.sort_unstable_by(|a, b| b.cmp(a));
        }
        match self.by_id.get(&row.tx.id) {
            Some(&at) => self.rows[at] = row,
            None => {
                self.by_id.insert(row.tx.id, self.rows.len());
                self.rows.push(row);
            }
        }
    }

    /// Drops rows by id. Positions shift, so any view must be recomputed.
    pub fn remove(&mut self, ids: &[i64]) {
        let gone: HashSet<i64> = ids.iter().copied().collect();
        self.rows.retain(|row| !gone.contains(&row.tx.id));
        self.by_id = self.rows.iter().enumerate().map(|(i, r)| (r.tx.id, i)).collect();
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row positions that pass the filters, in display order.
    pub fn view(&self, filters: &Filters, scope: &Scope, sort: &Sort) -> Vec<u32> {
        let compiled = Compiled::new(filters);
        let mut keyed: Vec<(Key, u32)> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| scope.contains(r) && compiled.matches(r))
            .map(|(i, r)| (sort.key(r), i as u32))
            .collect();
        // Stable, so equal keys keep server order like Array.prototype.sort.
        keyed.sort_by(|a, b| {
            let ord = a.0.cmp(&b.0);
            if sort.descending { ord.reverse() } else { ord }
        });
        keyed.into_iter().map(|(_, i)| i).collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationState {
    #[default]
    All,
    Annotated,
    Missing,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    pub search: String,
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    /// 0 is Sunday, matching both `Date.getDay` and Postgres `dow`.
    pub days_of_week: Vec<u8>,
    pub amount_min: Option<f64>,
    pub amount_max: Option<f64>,
    pub currency: Option<String>,
    pub transaction_types: Vec<String>,
    pub operators: Vec<String>,
    pub apps: Vec<String>,
    pub source_channel: Option<String>,
    pub source_chat_ids: Vec<i64>,
    pub card: Option<String>,
    pub parsing_method: Option<String>,
    /// Fractions from 0 to 1, as stored.
    pub confidence_min: Option<f64>,
    pub confidence_max: Option<f64>,
    pub annotation_details: String,
    pub annotation_persons: Vec<String>,
    pub annotation_articles: Vec<String>,
    pub annotation_period_from: Option<i32>,
    pub annotation_period_to: Option<i32>,
    pub annotation_state: AnnotationState,
}

impl Filters {
    /// Counts drawer filters only; search has its own box.
    pub fn active_count(&self) -> usize {
        let b = |x: bool| x as usize;
        b(self.date_from.is_some() || self.date_to.is_some())
            + b(!self.days_of_week.is_empty())
            + b(self.amount_min.is_some() || self.amount_max.is_some())
            + b(self.currency.is_some())
            + b(!self.transaction_types.is_empty())
            + b(!self.operators.is_empty())
            + b(!self.apps.is_empty())
            + b(self.source_channel.is_some())
            + b(!self.source_chat_ids.is_empty())
            + b(self.card.is_some())
            + b(self.parsing_method.is_some())
            + b(self.confidence_min.is_some() || self.confidence_max.is_some())
            + b(!self.annotation_details.trim().is_empty())
            + b(!self.annotation_persons.is_empty())
            + b(!self.annotation_articles.is_empty())
            + b(self.annotation_period_from.is_some() || self.annotation_period_to.is_some())
            + b(self.annotation_state != AnnotationState::All)
    }
}

/// What the account may see: the selected year and the unlocked period.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    pub year: Option<i32>,
    pub allowed_years: Vec<i32>,
    pub start: Option<NaiveDateTime>,
    pub end: Option<NaiveDateTime>,
}

impl Scope {
    fn contains(&self, row: &Row) -> bool {
        let limited = !self.allowed_years.is_empty() || self.start.is_some() || self.end.is_some();
        match row.when {
            None => !limited,
            Some(d) => {
                self.year.is_none_or(|y| d.year() == y)
                    && (self.allowed_years.is_empty() || self.allowed_years.contains(&d.year()))
                    && self.start.is_none_or(|s| d >= s)
                    && self.end.is_none_or(|e| d <= e)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort { column: Column::TransactionDate, descending: false }
    }
}

#[derive(PartialEq)]
enum Key {
    Num(f64),
    Text(String),
}

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Key::Num(a), Key::Num(b)) => a.total_cmp(b),
            (Key::Text(a), Key::Text(b)) => a.cmp(b),
            (Key::Num(_), Key::Text(_)) => Ordering::Less,
            (Key::Text(_), Key::Num(_)) => Ordering::Greater,
        }
    }
}

impl Sort {
    fn key(&self, r: &Row) -> Key {
        let text = |v: &Option<String>| Key::Text(v.as_deref().unwrap_or("").to_lowercase());
        match self.column {
            Column::RowNumber | Column::OperationNumber | Column::DateTime | Column::TransactionDate | Column::Time | Column::Day => {
                Key::Num(r.ts as f64)
            }
            Column::Amount => Key::Num(r.amount_signed),
            Column::BalanceAfter => Key::Num(r.balance),
            Column::IsP2p => Key::Num(r.tx.is_p2p as u8 as f64),
            Column::AnnotationPeriod => Key::Num(r.period.unwrap_or(0) as f64),
            Column::TransactionType => Key::Text(r.cell(Column::TransactionType).to_lowercase()),
            Column::SourceType => Key::Text(r.cell(Column::SourceType).to_lowercase()),
            Column::Operation => Key::Text(r.cell(Column::Operation).to_lowercase()),
            Column::OperatorRaw => Key::Text(r.operator_lc.clone()),
            Column::ApplicationMapped => Key::Text(r.app_lc.clone()),
            Column::Description => text(&r.tx.description),
            Column::ReceiverName => text(&r.tx.receiver_name),
            Column::ReceiverCard => text(&r.tx.receiver_card),
            Column::CardLast4 => text(&r.tx.card_last_4),
            Column::Currency => text(&r.tx.currency),
            Column::AnnotationDetails => Key::Text(r.details_lc.clone()),
            Column::AnnotationPersonCode => text(&r.tx.annotation_person_code),
            Column::AnnotationArticleCode => text(&r.tx.annotation_article_code),
        }
    }
}

struct Token {
    negated: bool,
    field: Field,
    value: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Text,
    Amount,
    Date,
    Status,
    Seller,
}

/// Words, or `"quoted phrases"`, optionally `-negated` and `field:`-prefixed.
pub fn tokenize(search: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = search.char_indices().peekable();
    while let Some(&(start, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        if ch == '"' {
            if let Some(len) = search[start + 1..].find('"').filter(|l| *l > 0) {
                let end = start + 1 + len + 1;
                out.push(search[start..end].to_string());
                while chars.peek().is_some_and(|(i, _)| *i < end) {
                    chars.next();
                }
                continue;
            }
        }
        let mut end = search.len();
        while let Some(&(i, c)) = chars.peek() {
            if c.is_whitespace() {
                end = i;
                break;
            }
            chars.next();
        }
        out.push(search[start..end].to_string());
    }
    out
}

fn compile_token(raw: &str) -> Option<Token> {
    let (negated, clean) = raw.strip_prefix('-').map_or((false, raw), |r| (true, r));
    if clean.is_empty() {
        return None;
    }
    let unquote = |v: &str| v.trim_matches('"').to_lowercase();
    if let Some((key, rest)) = clean.split_once(':') {
        let field = match key.to_lowercase().as_str() {
            "amount" => Some(Field::Amount),
            "date" => Some(Field::Date),
            "status" => Some(Field::Status),
            "seller" => Some(Field::Seller),
            _ => None,
        };
        if let Some(field) = field {
            let value = unquote(if rest.is_empty() { clean } else { rest });
            return Some(Token { negated, field, value });
        }
    }
    Some(Token { negated, field: Field::Text, value: unquote(clean) })
}

struct Compiled<'a> {
    f: &'a Filters,
    tokens: Vec<Token>,
    operators: Vec<String>,
    apps: Vec<String>,
    details: String,
    date_to_end: Option<NaiveDateTime>,
    date_from: Option<NaiveDateTime>,
}

impl<'a> Compiled<'a> {
    fn new(f: &'a Filters) -> Self {
        let lower = |v: &[String]| v.iter().map(|s| s.to_lowercase()).collect();
        Compiled {
            tokens: tokenize(&f.search).iter().filter_map(|t| compile_token(t)).collect(),
            operators: lower(&f.operators),
            apps: lower(&f.apps),
            details: f.annotation_details.trim().to_lowercase(),
            date_from: f.date_from.and_then(|d| d.and_hms_opt(0, 0, 0)),
            date_to_end: f.date_to.and_then(|d| d.and_hms_milli_opt(23, 59, 59, 999)),
            f,
        }
    }

    fn matches(&self, r: &Row) -> bool {
        let f = self.f;
        let t = &r.tx;
        if let Some(c) = &f.currency {
            if t.currency.as_ref() != Some(c) {
                return false;
            }
        }
        if let Some(d) = r.when {
            if self.date_from.is_some_and(|from| d < from) || self.date_to_end.is_some_and(|to| d > to) {
                return false;
            }
            if !f.days_of_week.is_empty() && !f.days_of_week.contains(&(d.weekday().num_days_from_sunday() as u8)) {
                return false;
            }
        }
        if let Some(a) = r.amount_abs {
            if f.amount_min.is_some_and(|m| a < m) || f.amount_max.is_some_and(|m| a > m) {
                return false;
            }
        }
        if !self.operators.is_empty() && !self.operators.iter().any(|o| r.operator_lc.contains(o)) {
            return false;
        }
        if !self.apps.is_empty() && !self.apps.iter().any(|a| r.app_lc.contains(a)) {
            return false;
        }
        if !f.transaction_types.is_empty() && !t.transaction_type.as_ref().is_some_and(|v| f.transaction_types.contains(v)) {
            return false;
        }
        if f.source_channel.is_some() && t.source_channel != f.source_channel {
            return false;
        }
        if !f.source_chat_ids.is_empty() && !t.source_chat_id.is_some_and(|id| f.source_chat_ids.contains(&id)) {
            return false;
        }
        if f.card.is_some() && t.card_last_4 != f.card {
            return false;
        }
        if f.parsing_method.is_some() && t.parsing_method != f.parsing_method {
            return false;
        }
        if f.confidence_min.is_some() || f.confidence_max.is_some() {
            let Some(c) = t.parsing_confidence else { return false };
            if f.confidence_min.is_some_and(|m| c < m) || f.confidence_max.is_some_and(|m| c > m) {
                return false;
            }
        }
        if !self.details.is_empty() && !r.details_lc.contains(&self.details) {
            return false;
        }
        if !f.annotation_persons.is_empty() && !t.annotation_person_code.as_ref().is_some_and(|v| f.annotation_persons.contains(v)) {
            return false;
        }
        if !f.annotation_articles.is_empty() && !t.annotation_article_code.as_ref().is_some_and(|v| f.annotation_articles.contains(v)) {
            return false;
        }
        if f.annotation_period_from.is_some() || f.annotation_period_to.is_some() {
            let Some(p) = r.period else { return false };
            if f.annotation_period_from.is_some_and(|m| p < m) || f.annotation_period_to.is_some_and(|m| p > m) {
                return false;
            }
        }
        match f.annotation_state {
            AnnotationState::All => {}
            AnnotationState::Annotated if !r.annotated() => return false,
            AnnotationState::Missing if r.annotated() => return false,
            _ => {}
        }
        self.tokens.iter().all(|tok| {
            let hit = match tok.field {
                Field::Text => r.blob.contains(&tok.value),
                Field::Amount => r.amount_text.contains(&tok.value),
                Field::Date => r.date_text.contains(&tok.value),
                Field::Status => r.status_blob.contains(&tok.value),
                Field::Seller => r.seller_blob.contains(&tok.value),
            };
            hit != tok.negated
        })
    }
}

/// The query the server needs to produce the same rows as a file.
///
/// `range` overrides the period; without it the export covers the filter's
/// dates, or the selected year.
pub fn export_params(
    filters: &Filters,
    scope: &Scope,
    sort: &Sort,
    columns: &[Column],
    range: Option<(NaiveDate, NaiveDate)>,
) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let positional = matches!(sort.column, Column::RowNumber | Column::OperationNumber | Column::DateTime);
    out.push(("sort_by", if positional { "transaction_date" } else { sort.column.id() }.to_string()));
    out.push(("sort_dir", if sort.descending { "desc" } else { "asc" }.to_string()));

    let year_start = scope.year.and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1));
    let year_end = scope.year.and_then(|y| NaiveDate::from_ymd_opt(y, 12, 31));
    let (from, to) = match range {
        Some((from, to)) => (Some(from), Some(to)),
        None => (filters.date_from.or(year_start), filters.date_to.or(year_end)),
    };
    if let Some(from) = from {
        out.push(("date_from", format!("{}T00:00:00", from.format("%Y-%m-%d"))));
    }
    if let Some(to) = to {
        out.push(("date_to", format!("{}T23:59:59", to.format("%Y-%m-%d"))));
    }

    let lists: [(&'static str, &Vec<String>); 5] = [
        ("operators", &filters.operators),
        ("apps", &filters.apps),
        ("transaction_types", &filters.transaction_types),
        ("annotation_persons", &filters.annotation_persons),
        ("annotation_articles", &filters.annotation_articles),
    ];
    for (key, items) in lists {
        if !items.is_empty() {
            out.push((key, items.join(",")));
        }
    }

    if let Some(min) = filters.amount_min {
        out.push(("amount_min", min.to_string()));
    }
    if let Some(max) = filters.amount_max {
        out.push(("amount_max", max.to_string()));
    }
    if let Some(currency) = filters.currency.as_ref().filter(|c| c.as_str() != "ALL") {
        out.push(("currency", currency.clone()));
    }
    if let Some(card) = &filters.card {
        out.push(("card", card.clone()));
    }
    if !filters.days_of_week.is_empty() {
        let days: Vec<String> = filters.days_of_week.iter().map(u8::to_string).collect();
        out.push(("days_of_week", days.join(",")));
    }
    // The export knows source types; Telegram rows are the automatic ones.
    if let Some(channel) = &filters.source_channel {
        out.push(("source_type", if channel == "TELEGRAM" { "AUTO" } else { channel.as_str() }.to_string()));
    }
    if !filters.source_chat_ids.is_empty() {
        let ids: Vec<String> = filters.source_chat_ids.iter().map(i64::to_string).collect();
        out.push(("source_chat_ids", ids.join(",")));
    }
    if let Some(method) = &filters.parsing_method {
        out.push(("parsing_method", method.clone()));
    }
    if let Some(min) = filters.confidence_min {
        out.push(("confidence_min", min.to_string()));
    }
    if let Some(max) = filters.confidence_max {
        out.push(("confidence_max", max.to_string()));
    }
    let search = filters.search.trim();
    if !search.is_empty() {
        out.push(("search", search.to_string()));
    }
    let details = filters.annotation_details.trim();
    if !details.is_empty() {
        out.push(("annotation_details", details.to_string()));
    }
    let month = |key: i32| format!("{:04}-{:02}-01", key / 100, key % 100);
    if let Some(from) = filters.annotation_period_from {
        out.push(("annotation_period_from", month(from)));
    }
    if let Some(to) = filters.annotation_period_to {
        out.push(("annotation_period_to", month(to)));
    }
    match filters.annotation_state {
        AnnotationState::All => {}
        AnnotationState::Annotated => out.push(("annotation_state", "annotated".to_string())),
        AnnotationState::Missing => out.push(("annotation_state", "missing".to_string())),
    }
    for column in columns {
        out.push(("columns", column.id().to_string()));
    }
    out.push(("layout", "client".to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_carries_every_visible_filter() {
        let filters = Filters {
            search: " korzinka ".into(),
            operators: vec!["A".into(), "B".into()],
            amount_min: Some(100.0),
            currency: Some("USD".into()),
            days_of_week: vec![1, 0],
            source_channel: Some("TELEGRAM".into()),
            source_chat_ids: vec![-5],
            annotation_persons: vec!["P1".into()],
            annotation_period_from: Some(202601),
            annotation_state: AnnotationState::Missing,
            ..Default::default()
        };
        let scope = Scope { year: Some(2026), ..Default::default() };
        let sort = Sort { column: Column::OperationNumber, descending: true };
        let params = export_params(&filters, &scope, &sort, &[Column::Amount, Column::CardLast4], None);
        let get = |key: &str| params.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str());
        assert_eq!(get("sort_by"), Some("transaction_date"));
        assert_eq!(get("sort_dir"), Some("desc"));
        assert_eq!(get("date_from"), Some("2026-01-01T00:00:00"));
        assert_eq!(get("date_to"), Some("2026-12-31T23:59:59"));
        assert_eq!(get("operators"), Some("A,B"));
        assert_eq!(get("amount_min"), Some("100"));
        assert_eq!(get("days_of_week"), Some("1,0"));
        assert_eq!(get("source_type"), Some("AUTO"));
        assert_eq!(get("source_chat_ids"), Some("-5"));
        assert_eq!(get("search"), Some("korzinka"));
        assert_eq!(get("annotation_persons"), Some("P1"));
        assert_eq!(get("annotation_period_from"), Some("2026-01-01"));
        assert_eq!(get("annotation_state"), Some("missing"));
        assert_eq!(get("layout"), Some("client"));
        let columns: Vec<&str> = params.iter().filter(|(k, _)| *k == "columns").map(|(_, v)| v.as_str()).collect();
        assert_eq!(columns, ["amount", "card_last_4"]);
    }

    #[test]
    fn export_period_overrides_filter_dates() {
        let day = |d| NaiveDate::from_ymd_opt(2025, 3, d).unwrap();
        let filters = Filters { date_from: Some(day(1)), ..Default::default() };
        let params = export_params(&filters, &Scope::default(), &Sort::default(), &[], Some((day(5), day(9))));
        assert!(params.contains(&("date_from", "2025-03-05T00:00:00".to_string())));
        assert!(params.contains(&("date_to", "2025-03-09T23:59:59".to_string())));
    }

    #[test]
    fn upsert_keeps_position_and_learns_new_values() {
        let mut ds = Dataset::build(vec![tx(1, "2026-01-01T10:00:00", "100.00", "alpha"), tx(2, "2026-01-02T10:00:00", "200.00", "gamma")]);
        ds.upsert(tx(1, "2026-01-01T10:00:00", "150.00", "beta"));
        assert_eq!(ds.rows[0].tx.amount.as_deref(), Some("150.00"));
        assert_eq!(ds.operators, ["alpha", "beta", "gamma"]);
        ds.upsert(tx(3, "2025-01-02T10:00:00", "1.00", "alpha"));
        assert_eq!(ds.len(), 3);
        assert_eq!(ds.years, [2026, 2025]);
        ds.remove(&[1]);
        assert!(ds.by_id(1).is_none());
        assert_eq!(ds.by_id(3).map(|r| r.tx.id), Some(3));
    }

    fn tx(id: i64, date: &str, amount: &str, op: &str) -> Transaction {
        Transaction {
            id,
            transaction_date: Some(date.into()),
            amount: Some(amount.into()),
            operator_raw: Some(op.into()),
            currency: Some("UZS".into()),
            transaction_type: Some("DEBIT".into()),
            ..Default::default()
        }
    }

    fn data() -> Dataset {
        Dataset::build(vec![
            tx(1, "2025-12-31T23:59:00", "100.00", "Korzinka Market"),
            tx(2, "2026-01-04T07:16:00", "5000.00", "PAYME Oplata"),
            Transaction { currency: Some("USD".into()), transaction_type: Some("CREDIT".into()), ..tx(3, "2026-01-05T10:00:00", "20.50", "Uzum Bank") },
            Transaction { annotation_person_code: Some("Б".into()), annotation_period: Some("03.2026".into()), ..tx(4, "2026-03-01T00:00:00", "-7.00", "Магазин Ромашка") },
        ])
    }

    fn ids(ds: &Dataset, f: &Filters) -> Vec<i64> {
        ds.view(f, &Scope::default(), &Sort::default()).iter().map(|i| ds.rows[*i as usize].tx.id).collect()
    }

    #[test]
    fn default_sort_is_date_ascending_and_descending_reverses() {
        let ds = data();
        assert_eq!(ids(&ds, &Filters::default()), [1, 2, 3, 4]);
        let v = ds.view(&Filters::default(), &Scope::default(), &Sort { column: Column::TransactionDate, descending: true });
        assert_eq!(v, [3, 2, 1, 0]);
    }

    #[test]
    fn numeric_sort_is_not_lexical() {
        let ds = data();
        let v = ds.view(&Filters::default(), &Scope::default(), &Sort { column: Column::Amount, descending: false });
        let got: Vec<i64> = v.iter().map(|i| ds.rows[*i as usize].tx.id).collect();
        assert_eq!(got, [4, 3, 1, 2]);
    }

    #[test]
    fn date_to_covers_the_whole_day() {
        let ds = data();
        let f = Filters { date_to: NaiveDate::from_ymd_opt(2025, 12, 31), ..Default::default() };
        assert_eq!(ids(&ds, &f), [1]);
    }

    #[test]
    fn weekday_zero_is_sunday() {
        let ds = data();
        let f = Filters { days_of_week: vec![0], ..Default::default() };
        assert_eq!(ids(&ds, &f), [2, 4]);
    }

    #[test]
    fn amount_uses_absolute_value() {
        let ds = data();
        let f = Filters { amount_min: Some(5.0), amount_max: Some(10.0), ..Default::default() };
        assert_eq!(ids(&ds, &f), [4]);
    }

    #[test]
    fn operator_filter_is_case_insensitive_substring_including_cyrillic() {
        let ds = data();
        let f = Filters { operators: vec!["РОМАШКА".into(), "payme".into()], ..Default::default() };
        assert_eq!(ids(&ds, &f), [2, 4]);
    }

    #[test]
    fn search_tokens() {
        let ds = data();
        let s = |q: &str| ids(&ds, &Filters { search: q.into(), ..Default::default() });
        assert_eq!(s("payme"), [2]);
        assert_eq!(s("\"Korzinka Market\""), [1]);
        assert_eq!(s("-uzs"), [3]);
        assert_eq!(s("amount:5000"), [2]);
        assert_eq!(s("date:2026-03"), [4]);
        assert_eq!(s("seller:uzum"), [3]);
        assert_eq!(s("-seller:uzum uzs"), [1, 2, 4]);
        assert_eq!(s("-"), [1, 2, 3, 4]);
        assert_eq!(s("   "), [1, 2, 3, 4]);
    }

    #[test]
    fn tokenizer_keeps_quotes_and_unbalanced_input() {
        assert_eq!(tokenize(r#"a "b c" -d"#), ["a", "\"b c\"", "-d"]);
        assert_eq!(tokenize(r#"say "oops"#), ["say", "\"oops"]);
    }

    #[test]
    fn annotations() {
        let ds = data();
        let f = |s| Filters { annotation_state: s, ..Default::default() };
        assert_eq!(ids(&ds, &f(AnnotationState::Annotated)), [4]);
        assert_eq!(ids(&ds, &f(AnnotationState::Missing)), [1, 2, 3]);
        let p = Filters { annotation_period_from: Some(202601), annotation_period_to: Some(202603), ..Default::default() };
        assert_eq!(ids(&ds, &p), [4]);
        let q = Filters { annotation_persons: vec!["Б".into()], ..Default::default() };
        assert_eq!(ids(&ds, &q), [4]);
    }

    #[test]
    fn period_key_reads_both_shapes() {
        assert_eq!(period_key("03.2026"), Some(202603));
        assert_eq!(period_key("2026-03-01"), Some(202603));
        assert_eq!(period_key("13.2026"), None);
        assert_eq!(period_key(""), None);
    }

    #[test]
    fn scope_limits_year_and_range() {
        let ds = data();
        let view = |s: &Scope| ds.view(&Filters::default(), s, &Sort::default()).len();
        assert_eq!(view(&Scope { year: Some(2026), ..Default::default() }), 3);
        assert_eq!(view(&Scope { allowed_years: vec![2025], ..Default::default() }), 1);
        let start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap().and_hms_opt(0, 0, 0);
        assert_eq!(view(&Scope { start, ..Default::default() }), 1);
    }

    #[test]
    fn options_and_lookup() {
        let ds = data();
        assert_eq!(ds.years, [2026, 2025]);
        assert_eq!(ds.operators.len(), 4);
        assert_eq!(ds.by_id(3).unwrap().cell(Column::Amount), "20,50");
        assert!(ds.by_id(99).is_none());
        assert_eq!(Filters { card: Some("1".into()), ..Default::default() }.active_count(), 1);
    }

    #[test]
    fn dictionaries_name_the_codes_whenever_they_arrive() {
        let coded = Transaction { annotation_article_code: Some("р1".into()), annotation_person_code: Some("ДиК".into()), ..tx(1, "2026-01-01T10:00:00", "1.00", "a") };
        let mut ds = Dataset::build(vec![coded.clone()]);
        assert_eq!(ds.rows[0].cell(Column::AnnotationArticleCode), "р1");
        let labels = Labels::new([("ДиК".to_string(), "Дильшод К.".to_string())], [("р1".to_string(), "ФЗП,ФМП,МП".to_string())]);
        ds.set_labels(labels.clone());
        assert_eq!(ds.rows[0].cell(Column::AnnotationArticleCode), "р1=ФЗП,ФМП,МП");
        assert_eq!(ds.rows[0].cell(Column::AnnotationPersonCode), "ДиК — Дильшод К.");
        // A row that comes later, and a whole new snapshot, read the same way.
        ds.upsert(Transaction { id: 2, ..coded.clone() });
        assert_eq!(ds.by_id(2).unwrap().cell(Column::AnnotationArticleCode), "р1=ФЗП,ФМП,МП");
        let rebuilt = Dataset::build_with(vec![coded], labels);
        assert_eq!(rebuilt.rows[0].cell(Column::AnnotationArticleCode), "р1=ФЗП,ФМП,МП");
        // The filter and the request body still speak in codes.
        let by_code = Filters { annotation_articles: vec!["р1".into()], ..Default::default() };
        assert_eq!(ids(&ds, &by_code), [1, 2]);
        ds.set_labels(Labels::default());
        assert_eq!(ds.rows[0].cell(Column::AnnotationArticleCode), "р1");
    }

    #[test]
    fn search_reads_the_ledger_fields_too() {
        let noted = Transaction {
            annotation_details: Some("Аренда склада".into()),
            receiver_name: Some("IVANOV IVAN".into()),
            receiver_card: Some("860012******0907".into()),
            description: Some("Интернет-магазин".into()),
            ..tx(1, "2026-01-01T10:00:00", "1.00", "a")
        };
        let ds = Dataset::build(vec![noted, tx(2, "2026-01-02T10:00:00", "2.00", "b")]);
        let s = |q: &str| ids(&ds, &Filters { search: q.into(), ..Default::default() });
        assert_eq!(s("склада"), [1]);
        assert_eq!(s("ivanov"), [1]);
        assert_eq!(s("0907"), [1]);
        assert_eq!(s("интернет"), [1]);
        assert_eq!(s("-склада"), [2]);
    }

    #[test]
    fn the_operation_is_searched_and_sorted_as_shown() {
        let humo = |id, kind: &str, raw: &str| Transaction {
            transaction_type: Some(kind.into()),
            raw_message: Some(raw.into()),
            source_display: Some("HUMO Card".into()),
            ..tx(id, "2026-01-01T10:00:00", "1.00", "shop")
        };
        let ds = Dataset::build(vec![
            humo(1, "DEBIT", "\u{1f4b8} Оплата\n➖ 40.000,00 UZS"),
            humo(2, "CREDIT", "[Photo]\nперевод"),
            humo(3, "DEBIT", "[Photo]\nперевод"),
            tx(4, "2026-01-02T10:00:00", "2.00", "b"),
        ]);
        let shown: Vec<&str> = ds.rows.iter().map(|r| r.cell(Column::Operation)).collect();
        assert_eq!(shown, ["\u{1f4b8} Оплата", "\u{1f389} Пополнение", "\u{1f4b8} Списание", "\u{1f534} Списание"]);

        let s = |q: &str| ids(&ds, &Filters { search: q.into(), ..Default::default() });
        // A headline is the receipt's own text; a label is nowhere but in the cell.
        assert_eq!(s("оплата"), [1]);
        assert_eq!(s("пополнение"), [2]);
        assert_eq!(s("СПИСАНИЕ"), [3, 4]);
        assert_eq!(s("-списание"), [1, 2]);
        assert_eq!(s("\"\u{1f4b8} списание\""), [3]);
        assert_eq!(s("\u{1f534}"), [4]);

        let sorted = |descending| {
            let sort = Sort { column: Column::Operation, descending };
            ds.view(&Filters::default(), &Scope::default(), &sort).iter().map(|i| ds.rows[*i as usize].tx.id).collect::<Vec<_>>()
        };
        // As text: by the mark first, then by the words.
        assert_eq!(sorted(false), [2, 1, 3, 4]);
        assert_eq!(sorted(true), [4, 3, 1, 2]);
    }

    #[test]
    fn hundred_thousand_rows_filter_fast_enough_for_typing() {
        let txs: Vec<Transaction> = (0..100_000)
            .map(|i| tx(i, "2026-01-04T07:16:00", "5000.00", if i % 2 == 0 { "PAYME Oplata" } else { "Uzum Bank" }))
            .collect();
        let ds = Dataset::build(txs);
        let started = std::time::Instant::now();
        let v = ds.view(&Filters { search: "payme".into(), ..Default::default() }, &Scope::default(), &Sort::default());
        assert_eq!(v.len(), 50_000);
        // Generous: an unoptimised test build on a loaded laptop.
        assert!(started.elapsed().as_millis() < 1500, "took {:?}", started.elapsed());
    }
}
