//! The sheet that narrows the table down, and the bar that says what is on.
//!
//! The sheet edits a draft. Nothing in it touches the table before
//! «Применить»; the bar above the table works the other way and acts at once.

use chrono::{Datelike, Days, Local, NaiveDate};
use iced::widget::{column, row};
use iced::{Alignment, Fill, Task};
use tbs_core::format;
use tbs_core::query::{AnnotationState, Filters, Sort};

use super::edit::TYPES;
use super::facets::{self, CHANNELS, DAYS, Facet, METHODS, Names};
use super::{Msg as Screen, SEARCH_DEBOUNCE, Sheet, Transactions};
use crate::ctx::Ctx;
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Kind};

const WIDTH: f32 = 520.0;
/// A folded group says what it holds in one line of about this many characters.
const SUMMARY_LENGTH: usize = 64;
const STATUS_HEIGHT: f32 = 24.0;
const CURRENCIES: [(&str, &str); 2] = [("UZS", "UZS"), ("USD", "USD")];

const DATE_FORMAT: &str = "Введите дату как ДД.ММ.ГГГГ, например 05.11.2026";
const DATE_ABSENT: &str = "Такой даты нет в календаре";
const MONTH_FORMAT: &str = "Введите месяц как ММ.ГГГГ, например 03.2026";
const NUMBER_FORMAT: &str = "Введите число, например 1500 или 1500,50";
const PERCENT_RANGE: &str = "Введите процент от 0 до 100";
const PERIOD_BACKWARDS: &str = "Конец периода раньше начала";
const BOUNDS_BACKWARDS: &str = "Верхняя граница меньше нижней";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    DateFrom,
    DateTo,
    AmountMin,
    AmountMax,
    ChatSearch,
    OperatorSearch,
    AppSearch,
    ConfidenceMin,
    ConfidenceMax,
    Details,
    PeriodFrom,
    PeriodTo,
    Card,
    PersonSearch,
    ArticleSearch,
}

impl Field {
    /// The name a message uses when it stands away from the field.
    fn title(self) -> &'static str {
        match self {
            Field::DateFrom => "Дата с",
            Field::DateTo => "Дата по",
            Field::AmountMin => "Сумма от",
            Field::AmountMax => "Сумма до",
            Field::ConfidenceMin => "Уверенность от",
            Field::ConfidenceMax => "Уверенность до",
            Field::Details => "Детали",
            Field::PeriodFrom => "Период с",
            Field::PeriodTo => "Период по",
            Field::Card => "Карта",
            Field::ChatSearch | Field::OperatorSearch | Field::AppSearch | Field::PersonSearch | Field::ArticleSearch => "Поиск",
        }
    }

    /// A box that only narrows a list down: typing in it changes no filter.
    fn is_search(self) -> bool {
        matches!(self, Field::ChatSearch | Field::OperatorSearch | Field::AppSearch | Field::PersonSearch | Field::ArticleSearch)
    }
}

/// A period one press fills in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Today,
    Yesterday,
    Week,
    Month,
    ThisMonth,
    LastMonth,
    ThisYear,
}

fn month_end(date: NaiveDate) -> NaiveDate {
    let (year, month) = if date.month() == 12 { (date.year() + 1, 1) } else { (date.year(), date.month() + 1) };
    NaiveDate::from_ymd_opt(year, month, 1).and_then(|next| next.pred_opt()).unwrap_or(date)
}

impl Span {
    pub const ALL: [Span; 7] = [Span::Today, Span::Yesterday, Span::Week, Span::Month, Span::ThisMonth, Span::LastMonth, Span::ThisYear];

    fn label(self) -> &'static str {
        match self {
            Span::Today => "Сегодня",
            Span::Yesterday => "Вчера",
            Span::Week => "7 дней",
            Span::Month => "30 дней",
            Span::ThisMonth => "Этот месяц",
            Span::LastMonth => "Прошлый месяц",
            Span::ThisYear => "Этот год",
        }
    }

    /// The first and the last day of the period, both inside it. A month and
    /// a year are whole, so a saved view reads the same on any day of them.
    pub fn range(self, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        let back = |days: u64| today.checked_sub_days(Days::new(days)).unwrap_or(today);
        let first = |date: NaiveDate| date.with_day(1).unwrap_or(date);
        match self {
            Span::Today => (today, today),
            Span::Yesterday => (back(1), back(1)),
            Span::Week => (back(6), today),
            Span::Month => (back(29), today),
            Span::ThisMonth => (first(today), month_end(today)),
            Span::LastMonth => {
                let last = first(today).pred_opt().unwrap_or(today);
                (first(last), last)
            }
            Span::ThisYear => (today.with_ordinal(1).unwrap_or(today), month_end(today.with_month(12).unwrap_or(today))),
        }
    }
}

/// A part of the sheet that folds on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Period,
    Money,
    Source,
    Operators,
    Apps,
    Parser,
    Accounting,
    Ids,
}

impl Group {
    pub const ALL: [Group; 8] = [Group::Period, Group::Money, Group::Source, Group::Operators, Group::Apps, Group::Parser, Group::Accounting, Group::Ids];

    fn title(self) -> &'static str {
        match self {
            Group::Period => "Период",
            Group::Money => "Финансы",
            Group::Source => "Источник",
            Group::Operators => "Операторы",
            Group::Apps => "Приложения",
            Group::Parser => "Парсер · уверенность",
            Group::Accounting => "Учёт",
            Group::Ids => "Идентификаторы",
        }
    }

    /// What is inside, for the folded line of a group nothing is chosen in.
    fn hint(self) -> &'static str {
        match self {
            Group::Period => "Даты и дни недели",
            Group::Money => "Сумма, валюта, тип операции",
            Group::Source => "Канал и чаты Telegram",
            Group::Operators => "Все операторы",
            Group::Apps => "Все приложения",
            Group::Parser => "Способ разбора и уверенность",
            Group::Accounting => "Детали, период учёта, разметка, люди, статьи",
            Group::Ids => "Последние цифры карты",
        }
    }

    fn facets(self) -> &'static [Facet] {
        match self {
            Group::Period => &[Facet::Date, Facet::Days],
            Group::Money => &[Facet::Amount, Facet::Currency, Facet::Types],
            Group::Source => &[Facet::Channel, Facet::Chats],
            Group::Operators => &[Facet::Operators],
            Group::Apps => &[Facet::Apps],
            Group::Parser => &[Facet::Method, Facet::Confidence],
            Group::Accounting => &[Facet::Details, Facet::Period, Facet::State, Facet::Persons, Facet::Articles],
            Group::Ids => &[Facet::Card],
        }
    }

    fn of(facet: Facet) -> Group {
        Group::ALL.into_iter().find(|group| group.facets().contains(&facet)).unwrap_or(Group::Period)
    }

    /// The group a field is typed in.
    fn holding(field: Field) -> Group {
        match field {
            Field::DateFrom | Field::DateTo => Group::Period,
            Field::AmountMin | Field::AmountMax => Group::Money,
            Field::ChatSearch => Group::Source,
            Field::OperatorSearch => Group::Operators,
            Field::AppSearch => Group::Apps,
            Field::ConfidenceMin | Field::ConfidenceMax => Group::Parser,
            Field::Details | Field::PeriodFrom | Field::PeriodTo | Field::PersonSearch | Field::ArticleSearch => Group::Accounting,
            Field::Card => Group::Ids,
        }
    }

    /// What is chosen in the group, in one line.
    fn summary(self, filters: &Filters, names: &Names<'_>) -> Option<String> {
        let parts: Vec<String> = match self.facets() {
            // The name of the group already says which filter this is.
            [only] => only.value(filters, names).into_iter().collect(),
            several => several.iter().filter_map(|facet| facet.chip(filters, names)).collect(),
        };
        (!parts.is_empty()).then(|| facets::shorten(&parts.join(" · "), SUMMARY_LENGTH))
    }
}

#[derive(Clone, Debug)]
pub enum Msg {
    Text(Field, String),
    Day(u8),
    Currency(Option<String>),
    Type(String),
    Channel(Option<String>),
    Chat(i64),
    Operator(String),
    App(String),
    Method(Option<String>),
    Person(String),
    Article(String),
    State(AnnotationState),
    Span(Span),
    Fold(Group),
    /// Empties one filter of the form: «Снять выбор» under a list.
    Clear(Facet),
    ClearGroup(Group),
    /// Empties the whole form. The table waits for «Применить» like after any other edit.
    Reset,
    /// Back to the filters the table has now.
    Revert,
    Apply,
    Close,
    /// The pause after typing is over; the number says which typing it waited for.
    Counted(u64),
    /// The words of a chip on the bar: open the sheet where that filter is set.
    Edit(Facet),
    /// The cross of a chip: that filter off, at once.
    Remove(Facet),
    /// «Сбросить всё» on the bar: every filter off, at once.
    RemoveAll,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilterDraft {
    date_from: String,
    date_to: String,
    days: Vec<u8>,
    amount_min: String,
    amount_max: String,
    currency: Option<String>,
    types: Vec<String>,
    channel: Option<String>,
    chats: Vec<i64>,
    chat_search: String,
    operators: Vec<String>,
    operator_search: String,
    apps: Vec<String>,
    app_search: String,
    method: Option<String>,
    confidence_min: String,
    confidence_max: String,
    details: String,
    persons: Vec<String>,
    person_search: String,
    articles: Vec<String>,
    article_search: String,
    period_from: String,
    period_to: String,
    state: AnnotationState,
    card: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flaw {
    /// Cannot be read as a value at all.
    Wrong,
    /// Reads like the beginning of a value somebody is still typing.
    Unfinished,
    /// Both ends read well, but the range runs backwards.
    Backwards,
}

/// What is wrong with one field, in words that stand right under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Problem {
    pub field: Field,
    pub text: &'static str,
    flaw: Flaw,
}

impl Problem {
    /// Whether it waits for «Применить» before it turns red. A date half
    /// typed is not a mistake yet, and a range is set one end at a time.
    fn patient(&self) -> bool {
        self.flaw != Flaw::Wrong
    }

    /// Whether the rows can be counted in spite of it.
    fn countable(&self) -> bool {
        self.flaw == Flaw::Backwards
    }
}

type Read<T> = Result<Option<T>, (&'static str, Flaw)>;

fn date_text(date: Option<NaiveDate>) -> String {
    date.map(|d| d.format("%d.%m.%Y").to_string()).unwrap_or_default()
}

fn number_text(value: Option<f64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn percent_text(value: Option<f64>) -> String {
    value.map(|v| (v * 100.0).round().to_string()).unwrap_or_default()
}

fn period_text(key: Option<i32>) -> String {
    key.map(|k| format!("{:02}.{}", k % 100, k / 100)).unwrap_or_default()
}

/// Whether the text could still grow into a value of `full` characters.
fn typing(input: &str, full: usize, marks: &str) -> bool {
    input.chars().count() < full && input.chars().all(|c| c.is_ascii_digit() || marks.contains(c))
}

/// A day typed by hand: 05.11.2026, or the ISO way the search box takes.
fn read_date(input: &str) -> Read<NaiveDate> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let digits = |part: &str, most: usize| ((1..=most).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse::<u32>().ok()).flatten();
    let parts: Vec<&str> = input.split('.').collect();
    if let [day, month, year] = parts.as_slice()
        && year.len() == 4
        && let (Some(day), Some(month), Some(year)) = (digits(day, 2), digits(month, 2), digits(year, 4))
        && year > 999
    {
        return NaiveDate::from_ymd_opt(year as i32, month, day).map(Some).ok_or((DATE_ABSENT, Flaw::Wrong));
    }
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d")
        && date.year() > 999
    {
        return Ok(Some(date));
    }
    Err((DATE_FORMAT, if typing(input, 10, ".-") { Flaw::Unfinished } else { Flaw::Wrong }))
}

fn read_number(input: &str) -> Read<f64> {
    let value: String = input.chars().filter(|c| !c.is_whitespace()).map(|c| if c == ',' { '.' } else { c }).collect();
    if value.is_empty() {
        return Ok(None);
    }
    value.parse::<f64>().ok().filter(|v| v.is_finite()).map(Some).ok_or((NUMBER_FORMAT, Flaw::Wrong))
}

fn read_percent(input: &str) -> Read<f64> {
    match read_number(input) {
        Ok(Some(value)) if (0.0..=100.0).contains(&value) => Ok(Some(value / 100.0)),
        Ok(None) => Ok(None),
        _ => Err((PERCENT_RANGE, Flaw::Wrong)),
    }
}

fn read_month(input: &str) -> Read<i32> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let parsed = input.split_once('.').and_then(|(month, year)| Some((month.parse::<i32>().ok()?, year.parse::<i32>().ok()?)));
    match parsed {
        Some((month, year)) if (1..=12).contains(&month) && (1000..=9999).contains(&year) => Ok(Some(year * 100 + month)),
        _ => Err((MONTH_FORMAT, if typing(input, 7, ".") { Flaw::Unfinished } else { Flaw::Wrong })),
    }
}

/// Keeps what was read; what was not goes to the list with the field it was typed in.
fn keep<T>(problems: &mut Vec<Problem>, field: Field, read: Read<T>) -> Option<T> {
    match read {
        Ok(value) => value,
        Err((text, flaw)) => {
            problems.push(Problem { field, text, flaw });
            None
        }
    }
}

/// A range that runs backwards is told at its far end.
fn ordered<T: PartialOrd>(problems: &mut Vec<Problem>, field: Field, text: &'static str, from: Option<T>, to: Option<T>) {
    if let (Some(from), Some(to)) = (from, to)
        && from > to
    {
        problems.push(Problem { field, text, flaw: Flaw::Backwards });
    }
}

fn toggle<T: PartialEq>(items: &mut Vec<T>, item: T) {
    match items.iter().position(|i| *i == item) {
        Some(index) => {
            items.remove(index);
        }
        None => items.push(item),
    }
}

impl FilterDraft {
    pub fn from(filters: &Filters) -> Self {
        Self {
            date_from: date_text(filters.date_from),
            date_to: date_text(filters.date_to),
            days: filters.days_of_week.clone(),
            amount_min: number_text(filters.amount_min),
            amount_max: number_text(filters.amount_max),
            currency: filters.currency.clone(),
            types: filters.transaction_types.clone(),
            channel: filters.source_channel.clone(),
            chats: filters.source_chat_ids.clone(),
            operators: filters.operators.clone(),
            apps: filters.apps.clone(),
            method: filters.parsing_method.clone(),
            confidence_min: percent_text(filters.confidence_min),
            confidence_max: percent_text(filters.confidence_max),
            details: filters.annotation_details.clone(),
            persons: filters.annotation_persons.clone(),
            articles: filters.annotation_articles.clone(),
            period_from: period_text(filters.annotation_period_from),
            period_to: period_text(filters.annotation_period_to),
            state: filters.annotation_state,
            card: filters.card.clone().unwrap_or_default(),
            ..Self::default()
        }
    }

    /// The form as filters, as far as it reads, and every field that does not.
    /// The search is not the sheet's and is left empty.
    pub fn read(&self) -> (Filters, Vec<Problem>) {
        let mut problems = Vec::new();
        let date_from = keep(&mut problems, Field::DateFrom, read_date(&self.date_from));
        let date_to = keep(&mut problems, Field::DateTo, read_date(&self.date_to));
        ordered(&mut problems, Field::DateTo, PERIOD_BACKWARDS, date_from, date_to);
        let amount_min = keep(&mut problems, Field::AmountMin, read_number(&self.amount_min));
        let amount_max = keep(&mut problems, Field::AmountMax, read_number(&self.amount_max));
        ordered(&mut problems, Field::AmountMax, BOUNDS_BACKWARDS, amount_min, amount_max);
        let confidence_min = keep(&mut problems, Field::ConfidenceMin, read_percent(&self.confidence_min));
        let confidence_max = keep(&mut problems, Field::ConfidenceMax, read_percent(&self.confidence_max));
        ordered(&mut problems, Field::ConfidenceMax, BOUNDS_BACKWARDS, confidence_min, confidence_max);
        let annotation_period_from = keep(&mut problems, Field::PeriodFrom, read_month(&self.period_from));
        let annotation_period_to = keep(&mut problems, Field::PeriodTo, read_month(&self.period_to));
        ordered(&mut problems, Field::PeriodTo, PERIOD_BACKWARDS, annotation_period_from, annotation_period_to);
        let card = self.card.trim();
        let filters = Filters {
            search: String::new(),
            date_from,
            date_to,
            days_of_week: self.days.clone(),
            amount_min,
            amount_max,
            currency: self.currency.clone(),
            transaction_types: self.types.clone(),
            operators: self.operators.clone(),
            apps: self.apps.clone(),
            source_channel: self.channel.clone(),
            source_chat_ids: self.chats.clone(),
            card: (!card.is_empty()).then(|| card.to_string()),
            parsing_method: self.method.clone(),
            confidence_min,
            confidence_max,
            annotation_details: self.details.trim().to_string(),
            annotation_persons: self.persons.clone(),
            annotation_articles: self.articles.clone(),
            annotation_period_from,
            annotation_period_to,
            annotation_state: self.state,
        };
        (filters, problems)
    }

    pub fn to_filters(&self, search: &str) -> Result<Filters, String> {
        let (filters, problems) = self.read();
        match problems.first() {
            None => Ok(Filters { search: search.to_string(), ..filters }),
            Some(problem) => Err(format!("{}: {}", problem.field.title(), problem.text)),
        }
    }

    fn set_text(&mut self, field: Field, value: String) {
        let target = match field {
            Field::DateFrom => &mut self.date_from,
            Field::DateTo => &mut self.date_to,
            Field::AmountMin => &mut self.amount_min,
            Field::AmountMax => &mut self.amount_max,
            Field::ChatSearch => &mut self.chat_search,
            Field::OperatorSearch => &mut self.operator_search,
            Field::AppSearch => &mut self.app_search,
            Field::ConfidenceMin => &mut self.confidence_min,
            Field::ConfidenceMax => &mut self.confidence_max,
            Field::Details => &mut self.details,
            Field::PeriodFrom => &mut self.period_from,
            Field::PeriodTo => &mut self.period_to,
            Field::Card => &mut self.card,
            Field::PersonSearch => &mut self.person_search,
            Field::ArticleSearch => &mut self.article_search,
        };
        *target = value;
    }

    /// Fills the dates with a period. Pressing the one that is set takes it off again.
    fn set_span(&mut self, span: Span, today: NaiveDate) {
        let (from, to) = span.range(today);
        let set = (read_date(&self.date_from).ok().flatten(), read_date(&self.date_to).ok().flatten());
        if set == (Some(from), Some(to)) {
            self.clear(Facet::Date);
        } else {
            (self.date_from, self.date_to) = (date_text(Some(from)), date_text(Some(to)));
        }
    }

    /// Empties one filter of the form and leaves the rest as typed.
    fn clear(&mut self, facet: Facet) {
        match facet {
            Facet::Date => (self.date_from, self.date_to) = Default::default(),
            Facet::Days => self.days.clear(),
            Facet::Amount => (self.amount_min, self.amount_max) = Default::default(),
            Facet::Currency => self.currency = None,
            Facet::Types => self.types.clear(),
            Facet::Channel => self.channel = None,
            Facet::Chats => self.chats.clear(),
            Facet::Operators => self.operators.clear(),
            Facet::Apps => self.apps.clear(),
            Facet::Method => self.method = None,
            Facet::Confidence => (self.confidence_min, self.confidence_max) = Default::default(),
            Facet::Details => self.details.clear(),
            Facet::Period => (self.period_from, self.period_to) = Default::default(),
            Facet::State => self.state = AnnotationState::All,
            Facet::Persons => self.persons.clear(),
            Facet::Articles => self.articles.clear(),
            Facet::Card => self.card.clear(),
        }
    }
}

/// What «Найдено N из M» has to say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Count {
    /// The rows are not here yet.
    #[default]
    Loading,
    /// The period is closed, and so is the number of its rows.
    Locked,
    /// Typing has not paused yet.
    Pending,
    /// A value is half typed: there is nothing to count with.
    Unfinished,
    /// The form does not read, and says where.
    Invalid,
    Found { found: usize, total: usize },
}

/// The sheet while it is open: the form, and the filters it started from.
#[derive(Clone, Debug)]
pub struct FilterForm {
    draft: FilterDraft,
    /// What the table had when the form was started.
    base: Filters,
    /// Opened again with edits nobody applied.
    resumed: bool,
    /// «Применить» was refused. Every problem shows until the form reads well.
    strict: bool,
    count: Count,
    /// Rows before any filter, counted once for as long as the rows stay put.
    total: Option<usize>,
    /// Which count is the latest; an older pause that ends later is ignored.
    token: u64,
}

impl FilterForm {
    fn new(applied: &Filters) -> Self {
        Self { draft: FilterDraft::from(applied), base: applied.clone(), resumed: false, strict: false, count: Count::default(), total: None, token: 0 }
    }

    /// Whether the form says something else than the table shows.
    fn dirty(&self) -> bool {
        let (filters, problems) = self.draft.read();
        !problems.is_empty() || !facets::agree(&filters, &self.base)
    }

    /// The problems that show next to their fields right now.
    fn shown(&self, problems: &[Problem]) -> Vec<Problem> {
        problems.iter().copied().filter(|problem| self.strict || !problem.patient()).collect()
    }
}

/// What the sheet remembers while it is closed.
pub struct Kept {
    open: [bool; Group::ALL.len()],
    /// A form closed with edits nobody applied. It is shown again only while
    /// the table still has the filters it was started from.
    parked: Option<Box<FilterForm>>,
}

impl Default for Kept {
    fn default() -> Self {
        let mut open = [false; Group::ALL.len()];
        open[Group::Period as usize] = true;
        Self { open, parked: None }
    }
}

fn send(message: Msg) -> Screen {
    Screen::Filters(message)
}

/// A text box of the form. What is wrong with it stands right under it.
fn typed<'a>(caption: &'a str, placeholder: &str, value: &str, field: Field, shown: &[Problem]) -> iced::widget::Column<'a, Screen> {
    let problem = shown.iter().find(|problem| problem.field == field).map(|problem| problem.text);
    let mut input = ui::input(placeholder, value).on_input(move |text| send(Msg::Text(field, text))).on_submit(send(Msg::Apply));
    if problem.is_some() {
        input = input.style(theme::input_error);
    }
    ui::field_checked(caption, input, problem)
}

/// One choice among several, with «Все» for no choice at all.
fn pick_one<'a>(items: &[(&'a str, &'a str)], active: &Option<String>, make: fn(Option<String>) -> Msg) -> El<'a, Screen> {
    let mut chips = vec![ui::chip("Все", active.is_none(), send(make(None)))];
    for (code, name) in items {
        chips.push(ui::chip(*name, active.as_deref() == Some(code), send(make(Some(code.to_string())))));
    }
    // A value saved by another build is shown as it is stored, so that it can be seen and dropped.
    if let Some(value) = active
        && !items.iter().any(|(code, _)| *code == value.as_str())
    {
        chips.push(ui::chip(value.clone(), true, send(make(Some(value.clone())))));
    }
    ui::chip_row(chips)
}

impl Transactions {
    fn names(&self) -> Names<'_> {
        Names { persons: &self.choices.persons, articles: &self.choices.articles, chats: &self.data.telegram_sources }
    }

    /// Whether a form closed without «Применить» is waiting to be opened again.
    pub(super) fn has_filter_draft(&self) -> bool {
        self.kept.parked.as_ref().is_some_and(|form| facets::agree(&form.base, &self.table.filters))
    }

    pub(super) fn open_filters(&mut self) {
        if matches!(self.sheet, Sheet::Filters(_)) {
            return;
        }
        let form = match self.kept.parked.take() {
            Some(mut form) if facets::agree(&form.base, &self.table.filters) => {
                form.resumed = true;
                form
            }
            _ => Box::new(FilterForm::new(&self.table.filters)),
        };
        self.sheet = Sheet::Filters(form);
        self.filters_refreshed();
    }

    /// Shuts the sheet without applying. Edits nobody applied are not thrown
    /// away: they wait for the next opening.
    pub(super) fn close_filters(&mut self) {
        if !matches!(self.sheet, Sheet::Filters(_)) {
            return;
        }
        if let Sheet::Filters(form) = std::mem::take(&mut self.sheet) {
            self.kept.parked = form.dirty().then_some(form);
        }
    }

    /// The filters of the table changed somewhere else: a chip, a saved view,
    /// a reset. A form made from the old ones must not outlive them.
    pub(super) fn forget_filter_draft(&mut self) {
        self.kept.parked = None;
        if matches!(self.sheet, Sheet::Filters(_)) {
            self.sheet = Sheet::Filters(Box::new(FilterForm::new(&self.table.filters)));
            self.filters_refreshed();
        }
    }

    /// The rows moved under the open sheet: a sync, an edit, another year, another search.
    pub(super) fn filters_refreshed(&mut self) {
        if let Sheet::Filters(form) = &mut self.sheet {
            form.total = None;
            self.recount_filters();
        }
    }

    /// What the counter says without looking at the rows, or the filters to count them with.
    fn count_plan(&self, form: &FilterForm) -> Result<Filters, Count> {
        if !self.loaded {
            return Err(Count::Loading);
        }
        if self.locked_view() {
            return Err(Count::Locked);
        }
        let (filters, problems) = form.draft.read();
        if problems.iter().any(|problem| form.strict || !problem.patient()) {
            return Err(Count::Invalid);
        }
        if problems.iter().any(|problem| !problem.countable()) {
            return Err(Count::Unfinished);
        }
        Ok(filters)
    }

    /// Brings «Найдено N из M» up to date with the form: one pass over the
    /// rows, the same one the table makes for its own filters.
    fn recount_filters(&mut self) {
        let Sheet::Filters(form) = &self.sheet else { return };
        let (count, total) = match self.count_plan(form) {
            Err(count) => (count, form.total),
            Ok(filters) => {
                let (scope, sort) = (self.scope(), Sort::default());
                let total = form.total.unwrap_or_else(|| self.data.view(&Filters::default(), &scope, &sort).len());
                let narrowed = filters.active_count() > 0 || !self.search.trim().is_empty();
                let found = if narrowed { self.data.view(&Filters { search: self.search.clone(), ..filters }, &scope, &sort).len() } else { total };
                (Count::Found { found, total }, Some(total))
            }
        };
        if let Sheet::Filters(form) = &mut self.sheet {
            form.count = count;
            form.total = total;
            form.token += 1;
        }
    }

    /// After a change to the form. Typing waits for a pause before the rows
    /// are counted, as the search box does; a press counts at once.
    fn settle_count(&mut self, typed: bool) -> Task<Screen> {
        let waits = typed && matches!(&self.sheet, Sheet::Filters(form) if self.count_plan(form).is_ok());
        if waits && let Sheet::Filters(form) = &mut self.sheet {
            form.count = Count::Pending;
            form.token += 1;
            let token = form.token;
            return Task::perform(async { tokio::time::sleep(SEARCH_DEBOUNCE).await }, move |_| send(Msg::Counted(token)));
        }
        self.recount_filters();
        Task::none()
    }

    /// The table got other filters without the sheet: the rows follow at once.
    fn filters_replaced(&mut self, ctx: &Ctx) {
        self.forget_filter_draft();
        self.refresh_rows();
        self.save(ctx);
    }

    /// «Применить»: the form becomes the filters of the table, or says next
    /// to each field what stops it.
    fn apply_filters(&mut self, ctx: &Ctx) {
        let Sheet::Filters(form) = &mut self.sheet else { return };
        let (filters, problems) = form.draft.read();
        if problems.is_empty() {
            self.table.filters = Filters { search: self.search.clone(), ..filters };
            self.sheet = Sheet::None;
            self.kept.parked = None;
            self.refresh_rows();
            self.save(ctx);
        } else {
            // The form stays as it was typed. Every problem shows now, in a group that is open.
            form.strict = true;
            for problem in &problems {
                self.kept.open[Group::holding(problem.field) as usize] = true;
            }
            self.recount_filters();
        }
    }

    pub(super) fn filters_update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Screen> {
        match message {
            // The bar changes the table itself, at once, with the sheet or without it.
            Msg::Remove(facet) => {
                facet.clear(&mut self.table.filters);
                self.filters_replaced(ctx);
            }
            Msg::RemoveAll => {
                self.table.filters = facets::cleared(&self.table.filters);
                self.filters_replaced(ctx);
            }
            Msg::Edit(facet) => {
                self.kept.open[Group::of(facet) as usize] = true;
                self.open_filters();
            }
            Msg::Fold(group) => self.kept.open[group as usize] ^= true,
            Msg::Close => self.close_filters(),
            Msg::Apply => self.apply_filters(ctx),
            Msg::Counted(token) => {
                if matches!(&self.sheet, Sheet::Filters(form) if form.token == token) {
                    self.recount_filters();
                }
            }
            edit => return self.edit_filters(edit),
        }
        Task::none()
    }

    /// A change to the form. Nothing here touches the table.
    fn edit_filters(&mut self, message: Msg) -> Task<Screen> {
        let Sheet::Filters(form) = &mut self.sheet else { return Task::none() };
        let draft = &mut form.draft;
        let mut typed = false;
        match message {
            Msg::Text(field, value) => {
                draft.set_text(field, value);
                if field.is_search() {
                    return Task::none();
                }
                typed = true;
            }
            Msg::Day(day) => toggle(&mut draft.days, day),
            Msg::Currency(value) => draft.currency = value,
            Msg::Type(code) => toggle(&mut draft.types, code),
            Msg::Channel(value) => {
                if value.as_deref() != Some("TELEGRAM") {
                    draft.chats.clear();
                }
                draft.channel = value;
            }
            Msg::Chat(id) => toggle(&mut draft.chats, id),
            Msg::Operator(name) => toggle(&mut draft.operators, name),
            Msg::App(name) => toggle(&mut draft.apps, name),
            Msg::Method(value) => draft.method = value,
            Msg::Person(code) => toggle(&mut draft.persons, code),
            Msg::Article(code) => toggle(&mut draft.articles, code),
            Msg::State(value) => draft.state = value,
            Msg::Span(span) => draft.set_span(span, Local::now().date_naive()),
            Msg::Clear(facet) => draft.clear(facet),
            Msg::ClearGroup(group) => group.facets().iter().for_each(|facet| draft.clear(*facet)),
            Msg::Reset => *draft = FilterDraft::default(),
            Msg::Revert => **form = FilterForm::new(&self.table.filters),
            Msg::Edit(_) | Msg::Remove(_) | Msg::RemoveAll | Msg::Fold(_) | Msg::Close | Msg::Apply | Msg::Counted(_) => return Task::none(),
        }
        // A form that reads well again goes back to waiting for «Применить».
        if form.strict && form.draft.read().1.is_empty() {
            form.strict = false;
        }
        self.settle_count(typed)
    }

    /// What is on, above the table: a chip a filter, each with a cross of its own.
    pub(super) fn filter_bar(&self) -> Option<El<'_, Screen>> {
        let names = self.names();
        let filters = &self.table.filters;
        let mut chips: Vec<El<'_, Screen>> = Facet::ALL
            .into_iter()
            .filter_map(|facet| Some(ui::tag(facet.chip(filters, &names)?, Some(send(Msg::Edit(facet))), Some(send(Msg::Remove(facet))))))
            .collect();
        if chips.is_empty() {
            return None;
        }
        chips.push(ui::link("Сбросить всё", Some(send(Msg::RemoveAll))).into());
        Some(ui::chip_row(chips))
    }

    /// The inside of one group of the sheet.
    fn group_view<'a>(&'a self, group: Group, draft: &'a FilterDraft, filters: &Filters, shown: &[Problem], today: NaiveDate) -> El<'a, Screen> {
        match group {
            Group::Period => {
                let spans = Span::ALL.iter().map(|span| ui::chip(span.label(), (filters.date_from, filters.date_to) == some(span.range(today)), send(Msg::Span(*span)))).collect();
                let days = DAYS.iter().map(|(id, name)| ui::chip(*name, draft.days.contains(id), send(Msg::Day(*id)))).collect();
                column![
                    ui::chip_row(spans),
                    row![
                        typed("с", "ДД.ММ.ГГГГ", &draft.date_from, Field::DateFrom, shown).width(Fill),
                        typed("по", "ДД.ММ.ГГГГ", &draft.date_to, Field::DateTo, shown).width(Fill),
                    ]
                    .spacing(space::MD),
                    ui::field("Дни недели", ui::chip_row(days)),
                ]
                .spacing(space::MD)
                .into()
            }
            Group::Money => {
                let mut types: Vec<El<'a, Screen>> = TYPES.iter().map(|(code, name)| ui::chip(*name, draft.types.iter().any(|t| t == code), send(Msg::Type(code.to_string())))).collect();
                for code in draft.types.iter().filter(|code| !TYPES.iter().any(|(known, _)| *known == code.as_str())) {
                    types.push(ui::chip(code.clone(), true, send(Msg::Type(code.clone()))));
                }
                column![
                    row![
                        typed("Сумма от", "0", &draft.amount_min, Field::AmountMin, shown).width(Fill),
                        typed("Сумма до", "∞", &draft.amount_max, Field::AmountMax, shown).width(Fill),
                    ]
                    .spacing(space::MD),
                    ui::field("Валюта", pick_one(&CURRENCIES, &draft.currency, Msg::Currency)),
                    ui::field("Тип операции", ui::chip_row(types)),
                ]
                .spacing(space::MD)
                .into()
            }
            Group::Source => {
                let mut source = column![ui::field("Канал", pick_one(&CHANNELS, &draft.channel, Msg::Channel))].spacing(space::MD);
                if draft.channel.as_deref() == Some("TELEGRAM") || !draft.chats.is_empty() {
                    let ids: Vec<String> = self.data.telegram_sources.iter().map(|(id, _)| id.to_string()).collect();
                    let items: Vec<(&str, &str)> = ids.iter().zip(&self.data.telegram_sources).map(|(id, (_, name))| (id.as_str(), name.as_str())).collect();
                    let chosen: Vec<String> = draft.chats.iter().map(i64::to_string).collect();
                    let list = ui::pick_many(
                        &items,
                        &chosen,
                        "Бот, группа, канал",
                        &draft.chat_search,
                        |text| send(Msg::Text(Field::ChatSearch, text)),
                        |id| send(id.parse().map_or(Msg::Chat(0), Msg::Chat)),
                        send(Msg::Clear(Facet::Chats)),
                    );
                    source = source.push(ui::field("Чаты Telegram", list));
                } else {
                    source = source.push(ui::faint("Чаты выбираются, когда канал — Telegram."));
                }
                source.into()
            }
            Group::Operators => {
                let items: Vec<(&str, &str)> = self.data.operators.iter().map(|name| (name.as_str(), name.as_str())).collect();
                ui::pick_many(
                    &items,
                    &draft.operators,
                    "Поиск оператора",
                    &draft.operator_search,
                    |text| send(Msg::Text(Field::OperatorSearch, text)),
                    |name| send(Msg::Operator(name)),
                    send(Msg::Clear(Facet::Operators)),
                )
            }
            Group::Apps => {
                let items: Vec<(&str, &str)> = self.data.apps.iter().map(|name| (name.as_str(), name.as_str())).collect();
                ui::pick_many(
                    &items,
                    &draft.apps,
                    "Поиск приложения",
                    &draft.app_search,
                    |text| send(Msg::Text(Field::AppSearch, text)),
                    |name| send(Msg::App(name)),
                    send(Msg::Clear(Facet::Apps)),
                )
            }
            Group::Parser => column![
                ui::field("Способ разбора", pick_one(&METHODS, &draft.method, Msg::Method)),
                row![
                    typed("Уверенность от, %", "0", &draft.confidence_min, Field::ConfidenceMin, shown).width(Fill),
                    typed("Уверенность до, %", "100", &draft.confidence_max, Field::ConfidenceMax, shown).width(Fill),
                ]
                .spacing(space::MD),
            ]
            .spacing(space::MD)
            .into(),
            Group::Accounting => {
                let states = [(AnnotationState::All, "Все"), (AnnotationState::Annotated, "Размечены"), (AnnotationState::Missing, "Без разметки")];
                let state_chips = states.iter().map(|(state, name)| ui::chip(*name, draft.state == *state, send(Msg::State(*state)))).collect();
                let mut accounting = column![
                    typed("Детали содержат", "ст парковка", &draft.details, Field::Details, shown),
                    row![
                        typed("Период с", "ММ.ГГГГ", &draft.period_from, Field::PeriodFrom, shown).width(Fill),
                        typed("Период по", "ММ.ГГГГ", &draft.period_to, Field::PeriodTo, shown).width(Fill),
                    ]
                    .spacing(space::MD),
                    ui::field("Разметка", ui::chip_row(state_chips)),
                ]
                .spacing(space::MD);
                if !self.choices.persons.is_empty() || !draft.persons.is_empty() {
                    let items: Vec<(&str, &str)> = self.choices.persons.iter().map(|(code, name)| (code.as_str(), name.as_str())).collect();
                    let list = ui::pick_many(
                        &items,
                        &draft.persons,
                        "Поиск человека",
                        &draft.person_search,
                        |text| send(Msg::Text(Field::PersonSearch, text)),
                        |code| send(Msg::Person(code)),
                        send(Msg::Clear(Facet::Persons)),
                    );
                    accounting = accounting.push(ui::field("Люди", list));
                }
                if !self.choices.articles.is_empty() || !draft.articles.is_empty() {
                    let items: Vec<(&str, &str)> = self.choices.articles.iter().map(|(code, name)| (code.as_str(), name.as_str())).collect();
                    let list = ui::pick_many(
                        &items,
                        &draft.articles,
                        "Поиск статьи",
                        &draft.article_search,
                        |text| send(Msg::Text(Field::ArticleSearch, text)),
                        |code| send(Msg::Article(code)),
                        send(Msg::Clear(Facet::Articles)),
                    );
                    accounting = accounting.push(ui::field("Статьи", list));
                }
                accounting.into()
            }
            Group::Ids => typed("Карта, последние 4 цифры", "1234", &draft.card, Field::Card, shown).into(),
        }
    }

    pub(super) fn filters_view<'a>(&'a self, form: &'a FilterForm) -> El<'a, Screen> {
        let draft = &form.draft;
        let (filters, problems) = draft.read();
        let shown = form.shown(&problems);
        let dirty = !problems.is_empty() || !facets::agree(&filters, &form.base);
        let names = self.names();
        let today = Local::now().date_naive();

        let mut body = column![].spacing(space::SM);
        if form.resumed && dirty {
            body = body.push(ui::notice(Tone::Warning, "Это черновик: в прошлый раз панель закрыли без «Применить». Примените его или отмените изменения."));
        }
        for group in Group::ALL {
            let open = self.kept.open[group as usize];
            let set = group.facets().iter().filter(|facet| facet.is_active(&filters)).count();
            let unread = problems.iter().any(|problem| Group::holding(problem.field) == group);
            let edited = unread || group.facets().iter().any(|facet| !facet.same(&filters, &form.base));
            let mut fold = ui::fold(group.title(), open, send(Msg::Fold(group))).hint(group.hint()).summary(group.summary(&filters, &names)).count(set);
            if shown.iter().any(|problem| Group::holding(problem.field) == group) {
                fold = fold.mark("ошибка", Tone::Danger);
            }
            if edited {
                fold = fold.mark("изменено", Tone::Warning);
            }
            fold = fold.on_clear((set > 0 || unread).then(|| send(Msg::ClearGroup(group))));
            if open {
                fold = fold.body(self.group_view(group, draft, &filters, &shown, today));
            }
            body = body.push(fold);
        }

        let found: El<'a, Screen> = match form.count {
            Count::Found { found, total } => ui::strong(format!("Найдено {} из {}", format::count(found), format::count(total))).into(),
            Count::Pending => ui::muted("Считаем строки…").into(),
            Count::Unfinished => ui::muted("Допишите значение, и строки будут посчитаны").into(),
            Count::Invalid => ui::error_line("Есть ошибки в полях: строки не посчитаны"),
            Count::Locked => ui::muted("Период закрыт: строки не считаются").into(),
            Count::Loading => ui::muted("Данные ещё загружаются").into(),
        };
        let mut status = row![found, ui::fill()].spacing(space::SM).align_y(Alignment::Center).height(STATUS_HEIGHT);
        if dirty {
            status = status.push(ui::hint_on(ui::link("Отменить изменения", Some(send(Msg::Revert))), "Вернуть фильтры, которые сейчас применены к таблице"));
        }
        let blank = problems.is_empty() && filters.active_count() == 0;
        let reset = ui::hint_on(ui::btn("Сбросить", Kind::Ghost, (!blank).then(|| send(Msg::Reset))), "Очистить все группы. Таблица изменится после «Применить».");
        let footer = ui::sheet_foot(
            Some(status.into()),
            vec![reset],
            vec![ui::btn("Закрыть", Kind::Secondary, Some(send(Msg::Close))).into(), ui::btn("Применить", Kind::Primary, Some(send(Msg::Apply))).into()],
        );
        let mark = dirty.then(|| ui::badge("не применено", Tone::Warning));
        let head = ui::sheet_head("Уточнение", "Фильтры", mark, "Точная настройка выборки. Таблица изменится после «Применить».", send(Msg::Close));
        ui::sheet(iced::widget::Space::new(), ui::sheet_frame(head, body, Some(footer)), WIDTH, Some(send(Msg::Close)))
    }
}

/// Both ends of a range, as a filter keeps them.
fn some<T>((from, to): (T, T)) -> (Option<T>, Option<T>) {
    (Some(from), Some(to))
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use iced::Size;
    use iced_test::Simulator;
    use iced_test::core::Settings;
    use tbs_core::model::{Transaction, UserInfo};
    use tbs_core::query::Dataset;

    use super::super::facets::tests::rich;
    use super::super::table::TableView;
    use super::super::{Parcel, STATE, menus};
    use super::*;

    const FILTERS_FIXTURE: &str = include_str!("fixtures/filters.json");
    const STATE_FIXTURE: &str = include_str!("fixtures/table-state.json");
    const WINDOW: Size = Size::new(1280.0, 860.0);

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    /// What the fixtures were written from, by the build before this sheet.
    fn saved() -> Filters {
        Filters {
            search: "korzinka -status:дубликат".into(),
            date_from: Some(date(2026, 11, 1)),
            date_to: Some(date(2026, 11, 11)),
            days_of_week: vec![1, 3, 0],
            amount_min: Some(1000.0),
            amount_max: Some(5000.5),
            currency: Some("UZS".into()),
            transaction_types: vec!["DEBIT".into(), "REVERSAL".into()],
            operators: vec!["Korzinka".into(), "Магазин Ромашка".into()],
            apps: vec!["Payme".into()],
            source_channel: Some("TELEGRAM".into()),
            source_chat_ids: vec![-1001234567890, 42],
            card: Some("1234".into()),
            parsing_method: Some("gpt".into()),
            confidence_min: Some(0.25),
            confidence_max: Some(0.9),
            annotation_details: "ст парковка".into(),
            annotation_persons: vec!["Б".into(), "ИВ".into()],
            annotation_articles: vec!["ст".into()],
            annotation_period_from: Some(202603),
            annotation_period_to: Some(202611),
            annotation_state: AnnotationState::Missing,
        }
    }

    const SHOPS: [&str; 6] = ["Ромашка Маркет", "Кофейня Арбуз", "Такси Комета", "Аптека Подорожник", "Заправка Вектор", "Маяк Доставка"];

    /// Sixty made-up rows: ten of each shop, one a day from 1 October 2026.
    fn rows() -> Vec<Transaction> {
        (0..60)
            .map(|i| {
                let day = date(2026, 10, 1) + Duration::days(i);
                Transaction {
                    id: i + 1,
                    transaction_date: Some(format!("{}T10:{:02}:00", day.format("%Y-%m-%d"), i)),
                    amount: Some(format!("{}.00", (i + 1) * 1000)),
                    currency: Some(if i % 10 == 9 { "USD" } else { "UZS" }.into()),
                    operator_raw: Some(SHOPS[i as usize % SHOPS.len()].into()),
                    application_mapped: Some(if i % 2 == 0 { "Кошелёк" } else { "Копилка" }.into()),
                    transaction_type: Some(if i % 4 == 3 { "CREDIT" } else { "DEBIT" }.into()),
                    card_last_4: Some(if i % 3 == 0 { "1111" } else { "2222" }.into()),
                    parsing_method: Some(if i % 5 == 0 { "gpt" } else { "regex" }.into()),
                    parsing_confidence: Some(0.5 + (i % 5) as f64 / 10.0),
                    source_channel: Some(if i % 2 == 0 { "TELEGRAM" } else { "SMS" }.into()),
                    source_chat_id: (i % 2 == 0).then_some(-100 - (i % 4)),
                    source_chat_title: (i % 2 == 0).then(|| if i % 4 == 0 { "Касса · чеки" } else { "Бот выписок" }.to_string()),
                    annotation_person_code: (i % 3 == 0).then(|| "АК".to_string()),
                    annotation_article_code: (i % 6 == 0).then(|| "пр".to_string()),
                    annotation_period: (i % 3 == 0).then(|| "10.2026".to_string()),
                    ..Default::default()
                }
            })
            .collect()
    }

    struct Bench {
        screen: Transactions,
        ctx: Ctx,
        _dir: tempfile::TempDir,
    }

    impl Bench {
        fn new(admin: bool) -> Self {
            Self::with(admin, rows())
        }

        fn with(admin: bool, rows: Vec<Transaction>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut ctx = Ctx::for_tests(dir.path());
            ctx.window = WINDOW;
            if admin {
                ctx.user = Some(UserInfo { username: "anna".into(), role: "admin".into(), ..Default::default() });
            }
            let (mut screen, _) = Transactions::new(&ctx);
            let _ = screen.update(Screen::Cached(Parcel::new(Dataset::build(rows))), &mut ctx);
            let _ = screen.update(Screen::Year(2026), &mut ctx);
            screen.choices.persons = vec![("АК".into(), "Анна К.".into()), ("БЛ".into(), "Борис Л.".into())];
            screen.choices.articles = vec![("пр".into(), "Продукты".into()), ("тр".into(), "Транспорт".into())];
            Self { screen, ctx, _dir: dir }
        }

        fn send(&mut self, message: Msg) {
            let _ = self.screen.update(Screen::Filters(message), &mut self.ctx);
        }

        fn type_in(&mut self, field: Field, value: &str) {
            self.send(Msg::Text(field, value.into()));
        }

        /// Ends the pause that typing waits for before it counts.
        fn settle(&mut self) {
            let token = self.form().token;
            self.send(Msg::Counted(token));
        }

        fn escape(&mut self) {
            let _ = self.screen.update(Screen::Escape, &mut self.ctx);
        }

        fn is_open(&self) -> bool {
            matches!(self.screen.sheet, Sheet::Filters(_))
        }

        fn form(&self) -> &FilterForm {
            match &self.screen.sheet {
                Sheet::Filters(form) => form,
                _ => panic!("the filter sheet is closed"),
            }
        }

        fn applied(&self) -> &Filters {
            &self.screen.table.filters
        }

        fn rows_shown(&self) -> usize {
            self.screen.peek().shown
        }

        /// The page with whatever floats above it, as the window draws them.
        fn window(&self) -> El<'_, Screen> {
            let mut layers = vec![self.screen.view(&self.ctx)];
            layers.extend(self.screen.overlay(&self.ctx));
            iced::widget::Stack::with_children(layers).width(Fill).height(Fill).into()
        }

        fn sees(&self, words: &str) -> bool {
            let mut sim = Simulator::with_size(Settings::default(), WINDOW, self.window());
            sim.find(words).is_ok()
        }

        /// Reads the table state back from where the screen saves it.
        fn on_disk(&self) -> TableView {
            tbs_core::config::load_json(&self.ctx.state_path(STATE))
        }
    }

    #[test]
    fn a_filter_survives_the_form() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 3, d);
        let filters = Filters {
            search: "x".into(),
            date_from: day(1),
            date_to: day(9),
            days_of_week: vec![1, 0],
            amount_min: Some(1500.5),
            currency: Some("USD".into()),
            operators: vec!["Korzinka".into()],
            source_channel: Some("TELEGRAM".into()),
            source_chat_ids: vec![-5],
            confidence_min: Some(0.25),
            confidence_max: Some(0.75),
            annotation_period_from: Some(202603),
            annotation_state: AnnotationState::Missing,
            card: Some("1234".into()),
            ..Default::default()
        };
        assert_eq!(FilterDraft::from(&filters).to_filters("x").unwrap(), filters);
    }

    #[test]
    fn wrong_input_is_named() {
        let draft = FilterDraft { date_from: "1 марта".into(), ..Default::default() };
        assert!(draft.to_filters("").unwrap_err().contains("Дата с"));
        let draft = FilterDraft { confidence_max: "150".into(), ..Default::default() };
        assert!(draft.to_filters("").unwrap_err().contains("Уверенность до"));
        let draft = FilterDraft { period_to: "13.2026".into(), ..Default::default() };
        assert!(draft.to_filters("").unwrap_err().contains("Период по"));
        let draft = FilterDraft { date_from: "09.03.2026".into(), date_to: "01.03.2026".into(), ..Default::default() };
        assert!(draft.to_filters("").is_err());
    }

    #[test]
    fn an_empty_form_filters_nothing() {
        assert_eq!(FilterDraft::default().to_filters("").unwrap().active_count(), 0);
    }

    #[test]
    fn toggling_twice_leaves_nothing() {
        let mut items = vec![1];
        toggle(&mut items, 2);
        toggle(&mut items, 1);
        assert_eq!(items, [2]);
        toggle(&mut items, 2);
        assert!(items.is_empty());
    }

    #[test]
    fn every_kind_of_filter_survives_the_form() {
        for filters in [rich(), saved(), Filters::default()] {
            let draft = FilterDraft::from(&filters);
            assert_eq!(draft.read().1, []);
            assert_eq!(draft.to_filters(&filters.search).unwrap(), filters);
            // And once more through the form it came out of.
            assert_eq!(FilterDraft::from(&draft.to_filters("").unwrap()), draft);
        }
    }

    #[test]
    fn each_period_fills_its_dates() {
        let today = date(2026, 11, 11);
        let expected = [
            (Span::Today, date(2026, 11, 11), date(2026, 11, 11)),
            (Span::Yesterday, date(2026, 11, 10), date(2026, 11, 10)),
            (Span::Week, date(2026, 11, 5), date(2026, 11, 11)),
            (Span::Month, date(2026, 10, 13), date(2026, 11, 11)),
            (Span::ThisMonth, date(2026, 11, 1), date(2026, 11, 30)),
            (Span::LastMonth, date(2026, 10, 1), date(2026, 10, 31)),
            (Span::ThisYear, date(2026, 1, 1), date(2026, 12, 31)),
        ];
        assert_eq!(expected.len(), Span::ALL.len());
        for (span, from, to) in expected {
            assert_eq!(span.range(today), (from, to), "{span:?}");
        }
    }

    #[test]
    fn periods_cross_months_and_years() {
        // The first day of a year: everything that looks back lands in the year before.
        let new_year = date(2026, 1, 1);
        assert_eq!(Span::Yesterday.range(new_year), (date(2025, 12, 31), date(2025, 12, 31)));
        assert_eq!(Span::Week.range(new_year), (date(2025, 12, 26), new_year));
        assert_eq!(Span::Month.range(new_year), (date(2025, 12, 3), new_year));
        assert_eq!(Span::ThisMonth.range(new_year), (new_year, date(2026, 1, 31)));
        assert_eq!(Span::LastMonth.range(new_year), (date(2025, 12, 1), date(2025, 12, 31)));
        assert_eq!(Span::ThisYear.range(new_year), (new_year, date(2026, 12, 31)));
        // The last day of a year stays in it.
        let last = date(2026, 12, 31);
        assert_eq!(Span::ThisMonth.range(last), (date(2026, 12, 1), last));
        assert_eq!(Span::LastMonth.range(last), (date(2026, 11, 1), date(2026, 11, 30)));
        assert_eq!(Span::ThisYear.range(last), (date(2026, 1, 1), last));
        // February, in a leap year and in a plain one.
        assert_eq!(Span::Yesterday.range(date(2024, 3, 1)).0, date(2024, 2, 29));
        assert_eq!(Span::LastMonth.range(date(2024, 3, 31)), (date(2024, 2, 1), date(2024, 2, 29)));
        assert_eq!(Span::LastMonth.range(date(2026, 3, 31)), (date(2026, 2, 1), date(2026, 2, 28)));
        assert_eq!(Span::ThisMonth.range(date(2024, 2, 10)), (date(2024, 2, 1), date(2024, 2, 29)));
        assert_eq!(Span::ThisYear.range(date(2024, 2, 29)), (date(2024, 1, 1), date(2024, 12, 31)));
    }

    #[test]
    fn a_period_fills_the_form_and_typing_still_works() {
        let today = date(2026, 11, 11);
        let mut draft = FilterDraft { days: vec![1], ..Default::default() };
        draft.set_span(Span::LastMonth, today);
        assert_eq!((draft.date_from.as_str(), draft.date_to.as_str()), ("01.10.2026", "31.10.2026"));
        let filters = draft.to_filters("").unwrap();
        assert_eq!((filters.date_from, filters.date_to), some(Span::LastMonth.range(today)));
        // Another period replaces it; the same one again takes the dates off. Nothing else moves.
        draft.set_span(Span::Week, today);
        assert_eq!((draft.date_from.as_str(), draft.date_to.as_str()), ("05.11.2026", "11.11.2026"));
        draft.set_span(Span::Week, today);
        assert_eq!((draft.date_from.as_str(), draft.date_to.as_str()), ("", ""));
        assert_eq!(draft.days, [1]);
        // By hand, with or without the leading zeros, and the ISO way.
        for (typed, day) in [("05.11.2026", 5), ("5.11.2026", 5), ("2026-11-07", 7), (" 09.11.2026 ", 9)] {
            draft.set_text(Field::DateFrom, typed.into());
            assert_eq!(draft.to_filters("").unwrap().date_from, Some(date(2026, 11, day)), "{typed}");
        }
    }

    #[test]
    fn each_problem_is_told_at_its_own_field() {
        let draft = FilterDraft {
            date_from: "31.02.2026".into(),
            date_to: "вчера".into(),
            amount_min: "1 500,50".into(),
            amount_max: "много".into(),
            confidence_min: "150".into(),
            confidence_max: "90".into(),
            period_from: "03.2026".into(),
            period_to: "13.2026".into(),
            ..Default::default()
        };
        let (filters, problems) = draft.read();
        let told: Vec<(Field, &str)> = problems.iter().map(|problem| (problem.field, problem.text)).collect();
        assert_eq!(
            told,
            [
                (Field::DateFrom, DATE_ABSENT),
                (Field::DateTo, DATE_FORMAT),
                (Field::AmountMax, NUMBER_FORMAT),
                (Field::ConfidenceMin, PERCENT_RANGE),
                (Field::PeriodTo, MONTH_FORMAT),
            ]
        );
        // What reads is kept; what does not is simply not a filter yet.
        assert_eq!((filters.amount_min, filters.amount_max), (Some(1500.5), None));
        assert_eq!((filters.confidence_min, filters.confidence_max), (None, Some(0.9)));
        assert_eq!((filters.annotation_period_from, filters.annotation_period_to), (Some(202603), None));
        assert_eq!((filters.date_from, filters.date_to), (None, None));
    }

    #[test]
    fn a_range_that_runs_backwards_is_told_at_its_far_end() {
        let draft = FilterDraft {
            date_from: "09.03.2026".into(),
            date_to: "01.03.2026".into(),
            amount_min: "5000".into(),
            amount_max: "1000".into(),
            confidence_min: "80".into(),
            confidence_max: "20".into(),
            period_from: "05.2026".into(),
            period_to: "03.2026".into(),
            ..Default::default()
        };
        let told: Vec<(Field, &str)> = draft.read().1.iter().map(|problem| (problem.field, problem.text)).collect();
        assert_eq!(
            told,
            [
                (Field::DateTo, PERIOD_BACKWARDS),
                (Field::AmountMax, BOUNDS_BACKWARDS),
                (Field::ConfidenceMax, BOUNDS_BACKWARDS),
                (Field::PeriodTo, PERIOD_BACKWARDS),
            ]
        );
        assert!(draft.to_filters("").unwrap_err().starts_with("Дата по"));
        // The same day at both ends is a range of one day, not a mistake.
        let one_day = FilterDraft { date_from: "09.03.2026".into(), date_to: "09.03.2026".into(), ..Default::default() };
        assert_eq!(one_day.read().1, []);
    }

    #[test]
    fn half_a_date_is_not_scolded_until_apply() {
        let mut form = FilterForm::new(&Filters::default());
        for typed in ["0", "05.", "05.11", "05.11.202", "2026-1"] {
            form.draft.set_text(Field::DateFrom, typed.into());
            let problems = form.draft.read().1;
            assert_eq!(problems.len(), 1, "{typed}");
            assert_eq!(form.shown(&problems), [], "{typed} was scolded while being typed");
        }
        form.draft.set_text(Field::PeriodFrom, "03.20".into());
        let problems = form.draft.read().1;
        assert_eq!(form.shown(&problems), []);
        // A value that cannot become right is told at once.
        form.draft.set_text(Field::DateTo, "05/11/2026".into());
        let problems = form.draft.read().1;
        assert_eq!(form.shown(&problems).iter().map(|problem| problem.field).collect::<Vec<_>>(), [Field::DateTo]);
        // After a refused «Применить» nothing is waited for any more.
        form.strict = true;
        assert_eq!(form.shown(&problems).iter().map(|problem| problem.field).collect::<Vec<_>>(), [Field::DateFrom, Field::DateTo, Field::PeriodFrom]);
    }

    #[test]
    fn a_folded_group_says_what_is_chosen_in_it() {
        let persons = [("Б".to_string(), "Борисов Б.".to_string())];
        let names = Names { persons: &persons, ..Default::default() };
        let filters = rich();
        let summary = |group: Group| group.summary(&filters, &names);
        assert_eq!(summary(Group::Period).unwrap(), "Дата: 01.11.2026 – 11.11.2026 · Дни недели: пн, ср, вс");
        assert_eq!(summary(Group::Money).unwrap(), "Сумма: от 1\u{a0}000 до 5\u{a0}000 · Валюта: USD · Тип: Списание, Отмена");
        assert_eq!(summary(Group::Source).unwrap(), "Источник: Telegram · Чат: -100500, 42");
        // A group of one filter does not repeat its own name.
        assert_eq!(summary(Group::Operators).unwrap(), "Korzinka, Магазин Ромашка +2");
        assert_eq!(summary(Group::Apps).unwrap(), "Payme");
        assert_eq!(summary(Group::Parser).unwrap(), "Парсер: GPT · Уверенность: от 25% до 90%");
        assert_eq!(summary(Group::Ids).unwrap(), "1234");
        // A long one is cut to a line.
        let accounting = summary(Group::Accounting).unwrap();
        assert!(accounting.starts_with("Детали: ст парковка · Период учёта: 03.2026 – 11.2026 · "), "{accounting}");
        assert!(accounting.ends_with('…') && accounting.chars().count() == SUMMARY_LENGTH, "{accounting}");
        // Nothing chosen, nothing to say: the group shows what it is for instead.
        for group in Group::ALL {
            assert_eq!(group.summary(&Filters::default(), &names), None);
            assert!(!group.hint().is_empty());
        }
    }

    #[test]
    fn every_filter_has_its_group() {
        let mut seen = Vec::new();
        for group in Group::ALL {
            for facet in group.facets() {
                assert_eq!(Group::of(*facet), group);
                seen.push(*facet);
            }
        }
        assert_eq!(seen, Facet::ALL);
    }

    #[test]
    fn clearing_a_group_leaves_the_other_groups() {
        let full = rich();
        for group in Group::ALL {
            let mut draft = FilterDraft::from(&full);
            for facet in group.facets() {
                draft.clear(*facet);
            }
            let left = draft.to_filters(&full.search).unwrap();
            for facet in Facet::ALL {
                let inside = group.facets().contains(&facet);
                assert_eq!(facet.is_active(&left), !inside, "{facet:?} after clearing {group:?}");
                assert!(inside || facet.same(&left, &full), "{facet:?} changed with {group:?}");
            }
            assert_eq!(group.summary(&left, &Names::default()), None);
        }
    }

    #[test]
    fn the_fixtures_of_the_old_build_still_load() {
        // Written by the build before this sheet, and read back to the same filters.
        let filters: Filters = serde_json::from_str(FILTERS_FIXTURE).unwrap();
        assert_eq!(filters, saved());
        let state: TableView = serde_json::from_str(STATE_FIXTURE).unwrap();
        assert_eq!(state.filters, saved());
        // What this build writes for them is the same text, so the old build reads it too.
        assert_eq!(serde_json::to_string_pretty(&filters).unwrap(), FILTERS_FIXTURE);
        let written = serde_json::to_value(&state).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(STATE_FIXTURE).unwrap();
        assert_eq!(written["filters"], fixture["filters"]);
        // A state that knows fewer fields than there are now still loads.
        let sparse: TableView = serde_json::from_str(r#"{"filters":{"operators":["Ромашка Маркет"],"annotation_state":"Annotated"}}"#).unwrap();
        assert_eq!(sparse.filters, Filters { operators: vec!["Ромашка Маркет".into()], annotation_state: AnnotationState::Annotated, ..Default::default() });
    }

    #[test]
    fn a_saved_table_state_opens_with_its_filters_and_is_saved_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_tests(dir.path());
        std::fs::write(ctx.state_path(STATE), STATE_FIXTURE).unwrap();
        let (mut screen, _) = Transactions::new(&ctx);
        assert_eq!(screen.table.filters, saved());
        assert_eq!(screen.search, saved().search);
        // The sheet shows them without a complaint and has nothing to apply.
        screen.open_filters();
        let Sheet::Filters(form) = &screen.sheet else { panic!("the sheet did not open") };
        assert_eq!(form.draft.read(), (Filters { search: String::new(), ..saved() }, vec![]));
        assert!(!form.dirty());
        // Saved again, the filters are on disk exactly as the old build wrote them.
        screen.save(&ctx);
        let written: serde_json::Value = serde_json::from_slice(&std::fs::read(ctx.state_path(STATE)).unwrap()).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(STATE_FIXTURE).unwrap();
        assert_eq!(written["filters"], fixture["filters"]);
    }

    #[test]
    fn a_chip_takes_its_filter_off_at_once() {
        let mut bench = Bench::new(true);
        let all = bench.rows_shown();
        assert_eq!(all, 60);
        bench.screen.table.filters = Filters { operators: vec![SHOPS[0].into()], currency: Some("UZS".into()), amount_min: Some(20000.0), ..Default::default() };
        bench.screen.refresh_rows();
        let narrow = bench.rows_shown();
        assert!(narrow > 0 && narrow < 10, "{narrow}");

        bench.send(Msg::Remove(Facet::Amount));
        let wider = bench.rows_shown();
        assert!(wider > narrow, "the table did not follow the chip");
        assert_eq!(bench.applied(), &Filters { operators: vec![SHOPS[0].into()], currency: Some("UZS".into()), ..Default::default() });
        assert_eq!(bench.on_disk().filters, *bench.applied());
        assert!(!bench.is_open());

        bench.send(Msg::RemoveAll);
        assert_eq!(bench.applied().active_count(), 0);
        assert_eq!(bench.rows_shown(), all);
        assert_eq!(bench.on_disk().filters, Filters::default());
    }

    #[test]
    fn resetting_on_the_bar_keeps_the_search() {
        let mut bench = Bench::new(true);
        bench.screen.search = "ромашка".into();
        bench.screen.table.filters = Filters { currency: Some("USD".into()), ..Default::default() };
        bench.screen.refresh_rows();
        bench.send(Msg::RemoveAll);
        assert_eq!(bench.applied(), &Filters { search: "ромашка".into(), ..Default::default() });
        assert_eq!(bench.rows_shown(), 10);
    }

    #[test]
    fn the_bar_shows_what_is_on_and_nothing_when_nothing_is() {
        let mut bench = Bench::new(true);
        assert!(bench.screen.filter_bar().is_none());
        assert!(!bench.sees("Сбросить всё"));

        bench.screen.table.filters = Filters { operators: vec![SHOPS[0].into(), SHOPS[1].into(), SHOPS[2].into()], amount_min: Some(1000.0), amount_max: Some(5000.0), ..Default::default() };
        bench.screen.refresh_rows();
        assert!(bench.sees("Оператор: Ромашка Маркет, Кофейня Арбуз +1"));
        assert!(bench.sees("Сумма: от 1\u{a0}000 до 5\u{a0}000"));
        assert!(bench.sees("Сбросить всё"));
        assert!(bench.sees("Фильтры · 2"));
    }

    #[test]
    fn many_chips_wrap_inside_the_page() {
        let mut bench = Bench::new(true);
        bench.screen.table.filters = rich();
        bench.screen.refresh_rows();
        let narrow = Size::new(900.0, 700.0);
        let mut sim = Simulator::with_size(Settings::default(), narrow, bench.window());
        let mut bottom: f32 = 0.0;
        for facet in Facet::ALL {
            let chip = facet.chip(&bench.screen.table.filters, &bench.screen.names()).unwrap();
            let found = sim.find(chip.as_str()).unwrap_or_else(|_| panic!("no chip «{chip}»"));
            let bounds = found.visible_bounds().unwrap_or_else(|| panic!("«{chip}» is out of sight"));
            assert!(bounds.x >= 0.0 && bounds.x + bounds.width <= narrow.width, "«{chip}» sticks out: {bounds:?}");
            bottom = bottom.max(bounds.y + bounds.height);
        }
        // Seventeen chips take more than one line, and the table still has its room below them.
        assert!(bottom > 160.0 && bottom < narrow.height / 2.0, "the chips end at {bottom}");
        assert!(sim.find("Сбросить всё").is_ok());
    }

    #[test]
    fn apply_with_an_error_keeps_the_sheet_and_the_draft() {
        let mut bench = Bench::new(true);
        bench.send(Msg::Fold(Group::Period));
        assert!(!bench.screen.kept.open[Group::Period as usize]);
        bench.screen.open_filters();
        bench.send(Msg::Operator(SHOPS[1].into()));
        bench.type_in(Field::DateFrom, "31.02.2026".into());
        bench.type_in(Field::AmountMax, "5 000".into());
        let typed = bench.form().draft.clone();

        bench.send(Msg::Apply);
        assert!(bench.is_open(), "the sheet closed on an error");
        assert_eq!(bench.form().draft, typed);
        assert!(bench.form().strict);
        assert_eq!(bench.form().count, Count::Invalid);
        assert_eq!(bench.applied().active_count(), 0, "a form with an error reached the table");
        assert_eq!(bench.rows_shown(), 60);
        // The group with the problem is opened, and the problem stands in it.
        assert!(bench.screen.kept.open[Group::Period as usize]);
        assert!(bench.sees(DATE_ABSENT));
        assert!(bench.sees("ошибка"));
        assert!(bench.sees("Есть ошибки в полях: строки не посчитаны"));

        bench.type_in(Field::DateFrom, "28.02.2026".into());
        assert!(!bench.form().strict);
        assert!(!bench.sees(DATE_ABSENT));
        bench.send(Msg::Apply);
        assert!(!bench.is_open());
        assert_eq!(bench.applied(), &Filters { date_from: Some(date(2026, 2, 28)), amount_max: Some(5000.0), operators: vec![SHOPS[1].into()], ..Default::default() });
        assert_eq!(bench.on_disk().filters, *bench.applied());
        assert!(!bench.screen.has_filter_draft());
    }

    #[test]
    fn escape_keeps_a_draft_nobody_applied() {
        let mut bench = Bench::new(true);
        bench.screen.table.filters = Filters { currency: Some("UZS".into()), ..Default::default() };
        bench.screen.refresh_rows();
        let before = (bench.applied().clone(), bench.rows_shown());

        bench.screen.open_filters();
        assert!(!bench.form().dirty() && !bench.form().resumed);
        bench.send(Msg::Operator(SHOPS[2].into()));
        bench.type_in(Field::AmountMin, "abc".into());
        assert!(bench.form().dirty());
        assert!(bench.sees("не применено"));
        let typed = bench.form().draft.clone();

        bench.escape();
        assert!(!bench.is_open());
        assert_eq!((bench.applied().clone(), bench.rows_shown()), before, "Esc applied something");
        assert!(bench.screen.has_filter_draft());
        assert!(bench.sees("Фильтры · 1 · черновик"));

        // Opened again, the draft is back as it was typed, and says that it is one.
        bench.screen.open_filters();
        assert_eq!(bench.form().draft, typed);
        assert!(bench.form().resumed && bench.form().dirty());
        assert!(bench.sees("не применено"));
        assert!(bench.sees("Отменить изменения"));

        // Discarded, the form is the table's filters again and nothing waits any more.
        bench.send(Msg::Revert);
        assert_eq!(bench.form().draft, FilterDraft::from(&before.0));
        assert!(!bench.form().dirty() && !bench.form().resumed);
        assert!(!bench.sees("не применено"));
        bench.escape();
        assert!(!bench.screen.has_filter_draft());
        assert_eq!((bench.applied().clone(), bench.rows_shown()), before);
    }

    #[test]
    fn a_sheet_closed_untouched_leaves_no_draft() {
        let mut bench = Bench::new(true);
        bench.screen.open_filters();
        bench.send(Msg::Close);
        assert!(!bench.is_open() && !bench.screen.has_filter_draft());
        // An edit taken back is no edit either.
        bench.screen.open_filters();
        bench.send(Msg::Day(1));
        bench.send(Msg::Day(1));
        bench.type_in(Field::OperatorSearch, "ром".into());
        bench.send(Msg::Close);
        assert!(!bench.screen.has_filter_draft());
    }

    #[test]
    fn a_draft_does_not_outlive_the_filters_it_was_made_from() {
        let park = |bench: &mut Bench| {
            bench.screen.open_filters();
            bench.send(Msg::Operator(SHOPS[3].into()));
            bench.escape();
            assert!(bench.screen.has_filter_draft());
        };
        let fresh = |bench: &mut Bench| {
            assert!(!bench.screen.has_filter_draft(), "the draft outlived its filters");
            bench.screen.open_filters();
            assert_eq!(bench.form().draft, FilterDraft::from(bench.applied()));
            assert!(!bench.form().resumed && !bench.form().dirty());
            bench.send(Msg::Close);
        };
        let mut bench = Bench::new(true);
        bench.screen.table.filters = Filters { currency: Some("UZS".into()), days_of_week: vec![1, 2], ..Default::default() };
        bench.screen.refresh_rows();

        // A chip taken off.
        park(&mut bench);
        bench.send(Msg::Remove(Facet::Days));
        fresh(&mut bench);
        // One of the quick type chips above the table.
        park(&mut bench);
        let _ = bench.screen.update(Screen::Quick(Some("DEBIT")), &mut bench.ctx);
        fresh(&mut bench);
        // A saved view put on.
        let _ = bench.screen.update(Screen::Menu(menus::Msg::PresetName("Дебет".into())), &mut bench.ctx);
        let _ = bench.screen.update(Screen::Menu(menus::Msg::PresetSave(false)), &mut bench.ctx);
        bench.send(Msg::RemoveAll);
        park(&mut bench);
        let _ = bench.screen.update(Screen::Menu(menus::Msg::PresetLoad(0)), &mut bench.ctx);
        assert_eq!(bench.applied(), &Filters { currency: Some("UZS".into()), transaction_types: vec!["DEBIT".into()], ..Default::default() });
        fresh(&mut bench);
        // The whole view reset.
        park(&mut bench);
        let _ = bench.screen.update(Screen::Menu(menus::Msg::Reset), &mut bench.ctx);
        fresh(&mut bench);
        // «Сбросить всё» on the bar.
        bench.screen.table.filters = Filters { currency: Some("USD".into()), ..Default::default() };
        park(&mut bench);
        bench.send(Msg::RemoveAll);
        fresh(&mut bench);
    }

    #[test]
    fn filters_changed_under_an_open_sheet_restart_its_form() {
        let mut bench = Bench::new(true);
        bench.screen.table.filters = Filters { currency: Some("UZS".into()), card: Some("1111".into()), ..Default::default() };
        bench.screen.refresh_rows();
        bench.screen.open_filters();
        bench.send(Msg::Operator(SHOPS[0].into()));
        bench.send(Msg::Remove(Facet::Card));
        assert!(bench.is_open());
        assert_eq!(bench.form().draft, FilterDraft::from(&Filters { currency: Some("UZS".into()), ..Default::default() }));
        assert!(!bench.form().dirty());
    }

    #[test]
    fn resetting_the_sheet_clears_every_group_and_waits_for_apply() {
        let mut bench = Bench::new(true);
        bench.screen.table.filters = Filters { search: "маркет".into(), ..rich() };
        bench.screen.search = "маркет".into();
        bench.screen.refresh_rows();
        let before = (bench.applied().clone(), bench.rows_shown());
        bench.screen.open_filters();
        bench.type_in(Field::OperatorSearch, "ром".into());

        bench.send(Msg::Reset);
        assert_eq!(bench.form().draft, FilterDraft::default());
        assert!(bench.form().dirty());
        // The table has not moved, and the sheet says so.
        assert_eq!((bench.applied().clone(), bench.rows_shown()), before);
        assert!(bench.sees("не применено"));
        assert_eq!(bench.form().count, Count::Found { found: 10, total: 60 });

        bench.send(Msg::Apply);
        assert!(!bench.is_open());
        assert_eq!(bench.applied(), &Filters { search: "маркет".into(), ..Default::default() });
        assert_eq!(bench.rows_shown(), 10);
    }

    #[test]
    fn the_counter_follows_the_draft() {
        let mut bench = Bench::new(true);
        bench.screen.open_filters();
        assert_eq!(bench.form().count, Count::Found { found: 60, total: 60 });
        assert!(bench.sees("Найдено 60 из 60"));

        // A press is counted at once.
        bench.send(Msg::Operator(SHOPS[0].into()));
        assert_eq!(bench.form().count, Count::Found { found: 10, total: 60 });
        bench.send(Msg::Currency(Some("USD".into())));
        assert_eq!(bench.form().count, Count::Found { found: 0, total: 60 });
        bench.send(Msg::ClearGroup(Group::Money));
        assert_eq!(bench.form().count, Count::Found { found: 10, total: 60 });
        assert!(bench.sees("Найдено 10 из 60"));
        assert_eq!(bench.rows_shown(), 60, "the table moved before «Применить»");

        // Typing waits for a pause, then counts.
        bench.type_in(Field::AmountMin, "30000".into());
        assert_eq!(bench.form().count, Count::Pending);
        assert!(bench.sees("Считаем строки…"));
        bench.settle();
        assert_eq!(bench.form().count, Count::Found { found: 5, total: 60 });

        // A pause that ended for an older text does not count for the newer one.
        let stale = bench.form().token;
        bench.type_in(Field::AmountMin, "55000".into());
        bench.send(Msg::Counted(stale));
        assert_eq!(bench.form().count, Count::Pending);
        bench.settle();
        assert_eq!(bench.form().count, Count::Found { found: 1, total: 60 });

        // Typing in a search box of a list changes no filter and counts nothing.
        let token = bench.form().token;
        bench.type_in(Field::OperatorSearch, "ком".into());
        assert_eq!((bench.form().count, bench.form().token), (Count::Found { found: 1, total: 60 }, token));
    }

    #[test]
    fn the_counter_says_when_the_draft_cannot_be_counted() {
        let mut bench = Bench::new(true);
        bench.screen.open_filters();
        bench.send(Msg::Operator(SHOPS[0].into()));
        assert_eq!(bench.form().count, Count::Found { found: 10, total: 60 });

        // Wrong: said at once, and no number is left standing.
        bench.type_in(Field::AmountMin, "abc".into());
        assert_eq!(bench.form().count, Count::Invalid);
        assert!(bench.sees(NUMBER_FORMAT));
        assert!(!bench.sees("Найдено 10 из 60"));
        bench.type_in(Field::AmountMin, "".into());
        bench.settle();
        assert_eq!(bench.form().count, Count::Found { found: 10, total: 60 });

        // Half typed: nothing red, and no number either.
        bench.type_in(Field::DateFrom, "05.1".into());
        assert_eq!(bench.form().count, Count::Unfinished);
        assert!(!bench.sees(DATE_FORMAT));
        assert!(bench.sees("Допишите значение, и строки будут посчитаны"));
        bench.type_in(Field::DateFrom, "05.11.2026".into());
        bench.settle();
        assert_eq!(bench.form().count, Count::Found { found: 4, total: 60 });

        // A range that runs backwards leaves no rows, which is what the counter says.
        bench.type_in(Field::DateTo, "01.11.2026".into());
        bench.settle();
        assert_eq!(bench.form().count, Count::Found { found: 0, total: 60 });
        bench.send(Msg::Apply);
        assert!(bench.is_open());
        assert_eq!(bench.form().count, Count::Invalid);
        assert!(bench.sees(PERIOD_BACKWARDS));
    }

    #[test]
    fn the_counter_takes_the_search_and_the_year_into_account() {
        let mut rows = rows();
        rows.push(Transaction { id: 900, transaction_date: Some("2025-12-31T23:00:00".into()), operator_raw: Some(SHOPS[0].into()), ..Default::default() });
        let mut bench = Bench::with(true, rows);
        // Rows of another year are out of scope: they are neither found nor counted in the total.
        bench.screen.open_filters();
        assert_eq!(bench.form().count, Count::Found { found: 60, total: 60 });

        // The search box narrows what is found, not what there is.
        bench.screen.search = "арбуз".into();
        bench.screen.refresh_rows();
        assert_eq!(bench.form().count, Count::Found { found: 10, total: 60 });
        bench.send(Msg::Operator(SHOPS[0].into()));
        assert_eq!(bench.form().count, Count::Found { found: 0, total: 60 });
        bench.send(Msg::Operator(SHOPS[1].into()));
        assert_eq!(bench.form().count, Count::Found { found: 10, total: 60 });

        // Another year under the open sheet is counted anew.
        let _ = bench.screen.update(Screen::Year(2025), &mut bench.ctx);
        assert_eq!(bench.form().count, Count::Found { found: 0, total: 1 });
    }

    #[test]
    fn a_locked_period_keeps_its_filters_and_hides_its_rows() {
        // Nobody signed in as an admin and no access snapshot: the view is locked.
        let mut bench = Bench::new(false);
        assert!(bench.screen.locked_view());
        bench.screen.table.filters = Filters { currency: Some("UZS".into()), days_of_week: vec![1], ..Default::default() };
        bench.screen.refresh_rows();
        assert_eq!(bench.rows_shown(), 0);

        // The bar is there and works, like the «Фильтры» button beside it; the rows stay hidden.
        assert!(bench.sees("Валюта: UZS"));
        assert!(bench.sees("Период закрыт"));
        bench.send(Msg::Remove(Facet::Days));
        assert_eq!(bench.applied(), &Filters { currency: Some("UZS".into()), ..Default::default() });
        assert_eq!(bench.rows_shown(), 0);

        // The sheet opens, and the counter does not tell how many rows the period has.
        bench.screen.open_filters();
        bench.send(Msg::Operator(SHOPS[0].into()));
        assert_eq!(bench.form().count, Count::Locked);
        assert!(bench.sees("Период закрыт: строки не считаются"));
        bench.send(Msg::Apply);
        assert_eq!(bench.applied().operators, [SHOPS[0]]);
        assert_eq!(bench.rows_shown(), 0);
    }

    #[test]
    fn the_sheet_folds_and_marks_its_groups() {
        let mut bench = Bench::new(true);
        bench.screen.table.filters = Filters { operators: vec![SHOPS[0].into(), SHOPS[1].into()], ..Default::default() };
        bench.screen.refresh_rows();
        bench.screen.open_filters();
        // «Период» is open at first, the rest folded to a line: a summary where something is chosen, a hint where not.
        assert!(bench.sees("Сегодня") && bench.sees("Прошлый месяц"));
        assert!(bench.sees("Ромашка Маркет, Кофейня Арбуз"));
        assert!(bench.sees(Group::Money.hint()));
        assert!(!bench.sees("Поиск оператора"));
        assert!(!bench.sees("изменено"));

        bench.send(Msg::Fold(Group::Operators));
        assert!(bench.sees("Выбрано: 2"));
        assert!(bench.sees("Снять выбор"));
        assert!(!bench.sees("Ромашка Маркет, Кофейня Арбуз"), "an open group repeats its summary");
        bench.send(Msg::Clear(Facet::Operators));
        assert!(bench.sees("Ничего не выбрано: подходят все"));
        assert!(bench.sees("изменено"));
        assert_eq!(bench.form().draft.operators, Vec::<String>::new());

        bench.send(Msg::Fold(Group::Period));
        assert!(!bench.sees("Сегодня"));
        assert!(bench.sees(Group::Period.hint()));
        // The folds are remembered for the next opening.
        bench.send(Msg::Revert);
        bench.send(Msg::Close);
        bench.screen.open_filters();
        assert!(!bench.sees("Сегодня") && bench.sees("Выбрано: 2"));
    }

    #[test]
    fn a_chip_opens_the_sheet_at_its_group() {
        let mut bench = Bench::new(true);
        bench.screen.table.filters = Filters { card: Some("1111".into()), ..Default::default() };
        bench.screen.refresh_rows();
        bench.send(Msg::Edit(Facet::Card));
        assert!(bench.is_open());
        assert!(bench.screen.kept.open[Group::Ids as usize]);
        assert!(bench.sees("Карта, последние 4 цифры"));
    }

    #[test]
    fn the_table_filters_a_big_table_once_for_a_change_of_the_draft() {
        // The same hundred thousand rows the core filters for typing; here a press on the form counts them.
        let rows: Vec<Transaction> = (0..100_000)
            .map(|i| Transaction {
                id: i,
                transaction_date: Some("2026-01-04T07:16:00".into()),
                amount: Some("5000.00".into()),
                operator_raw: Some(if i % 2 == 0 { "Ромашка Маркет" } else { "Кофейня Арбуз" }.into()),
                ..Default::default()
            })
            .collect();
        let mut bench = Bench::with(true, rows);
        bench.screen.open_filters();
        let started = std::time::Instant::now();
        bench.send(Msg::Operator("Ромашка Маркет".into()));
        let took = started.elapsed();
        assert_eq!(bench.form().count, Count::Found { found: 50_000, total: 100_000 });
        // As generous as the core's own limit for one pass over these rows.
        assert!(took.as_millis() < 1500, "took {took:?}");
        // Typing does not count at all until it pauses.
        let started = std::time::Instant::now();
        bench.type_in(Field::AmountMin, "1".into());
        bench.type_in(Field::AmountMin, "10".into());
        bench.type_in(Field::AmountMin, "100".into());
        assert_eq!(bench.form().count, Count::Pending);
        assert!(started.elapsed().as_millis() < 200, "typing took {:?}", started.elapsed());
    }

    /// Saves pictures of the bar and the sheet, for a person to look at. Only when asked:
    /// `TBS_SHOTS=<folder> ICED_TEST_BACKEND=tiny-skia cargo test -p tbs-app p3_shots`.
    #[test]
    fn p3_shots() {
        let Ok(folder) = std::env::var("TBS_SHOTS") else { return };
        let folder = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        let shoot = |bench: &Bench, name: &str| {
            let mut sim = Simulator::with_size(Settings::default(), WINDOW, bench.window());
            let shot = sim.snapshot(&theme::iced_theme(false)).expect("a snapshot");
            // The simulator names the file after its renderer and leaves one that is there alone.
            let staging = folder.join(format!(".{name}"));
            let _ = std::fs::remove_dir_all(&staging);
            assert!(shot.matches_image(staging.join(name)).expect("a picture"));
            let written = std::fs::read_dir(&staging).unwrap().flatten().next().expect("a file").path();
            std::fs::rename(written, folder.join(format!("{name}.png"))).unwrap();
            let _ = std::fs::remove_dir_all(&staging);
        };

        // Rows of the last sixty days, so that «Этот месяц» always has some.
        let today = Local::now().date_naive();
        let mut rows = rows();
        for (i, row) in rows.iter_mut().enumerate() {
            let day = today - Duration::days(i as i64);
            row.transaction_date = Some(format!("{}T{:02}:{:02}:00", day.format("%Y-%m-%d"), 9 + i % 10, (i * 7) % 60));
            row.description = Some(["Продукты на неделю", "Кофе и выпечка", "Поездка по городу", "Лекарства", "Топливо", "Доставка заказа"][i % 6].into());
            row.receiver_name = (i % 4 == 3).then(|| "Получатель Образцов".to_string());
            row.balance_after = Some(format!("{}.00", 5_000_000 - i * 37_500));
        }
        let mut bench = Bench::with(true, rows);
        let _ = bench.screen.update(Screen::Year(today.year()), &mut bench.ctx);

        // The table with what is on above it.
        bench.screen.open_filters();
        bench.send(Msg::Span(Span::ThisMonth));
        bench.send(Msg::Operator(SHOPS[0].into()));
        bench.send(Msg::Operator(SHOPS[1].into()));
        bench.send(Msg::Operator(SHOPS[2].into()));
        bench.send(Msg::Operator(SHOPS[5].into()));
        bench.type_in(Field::AmountMin, "1000".into());
        bench.type_in(Field::AmountMax, "45000".into());
        bench.send(Msg::Type("DEBIT".into()));
        bench.send(Msg::State(AnnotationState::Missing));
        bench.send(Msg::Apply);
        assert!(!bench.is_open());
        shoot(&bench, "p3-bar");

        // The sheet with every group folded to its line.
        bench.send(Msg::Fold(Group::Period));
        bench.screen.open_filters();
        shoot(&bench, "p3-sheet-folded");

        // A period and a searchable list open, with an edit nobody applied yet.
        bench.send(Msg::Fold(Group::Period));
        bench.send(Msg::Fold(Group::Operators));
        bench.send(Msg::Day(6));
        bench.send(Msg::Day(0));
        bench.type_in(Field::OperatorSearch, "ма".into());
        shoot(&bench, "p3-sheet-open");

        // A value that does not read, told under its field after «Применить» was refused.
        bench.send(Msg::Fold(Group::Operators));
        bench.type_in(Field::DateTo, "31.11.2026".into());
        bench.type_in(Field::AmountMax, "сорок тысяч".into());
        bench.send(Msg::Apply);
        assert!(bench.is_open());
        shoot(&bench, "p3-sheet-error");

        // The same sheet once the values read again: the counter is back.
        bench.type_in(Field::DateTo, "30.11.2026".into());
        bench.type_in(Field::AmountMax, "40000".into());
        bench.settle();
        shoot(&bench, "p3-sheet-counter");

        // Closed without «Применить»: the draft waits, and the toolbar says so.
        bench.escape();
        shoot(&bench, "p3-bar-draft");
        bench.screen.open_filters();
        shoot(&bench, "p3-sheet-draft");
    }
}
