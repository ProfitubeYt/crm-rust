//! The sheet that narrows the table down.

use chrono::NaiveDate;
use iced::widget::{Column as Stack, column, container, row};
use iced::{Fill, Length};
use tbs_core::query::{AnnotationState, Filters};

use super::edit::TYPES;
use super::{Msg as Screen, Sheet, Transactions};
use crate::ctx::Ctx;
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Kind};

const WIDTH: f32 = 520.0;
const LIST_HEIGHT: f32 = 168.0;
const LIST_LIMIT: usize = 200;
const DAYS: [(u8, &str); 7] = [(1, "Пн"), (2, "Вт"), (3, "Ср"), (4, "Чт"), (5, "Пт"), (6, "Сб"), (0, "Вс")];
const METHODS: [(&str, &str); 4] = [("regex", "Regex"), ("gpt", "GPT"), ("manual", "Ручной"), ("operator_mapper", "Mapper")];
const CHANNELS: [(&str, &str); 3] = [("TELEGRAM", "Telegram"), ("SMS", "СМС"), ("MANUAL", "Ручной")];

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
    Reset,
    Apply,
    Close,
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
    articles: Vec<String>,
    period_from: String,
    period_to: String,
    state: AnnotationState,
    card: String,
    error: Option<String>,
}

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

pub(super) fn parse_date(input: &str, name: &str) -> Result<Option<NaiveDate>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(input, "%d.%m.%Y").map(Some).map_err(|_| format!("{name}: формат ДД.ММ.ГГГГ"))
}

fn parse_number(input: &str, name: &str) -> Result<Option<f64>, String> {
    let value: String = input.chars().filter(|c| !c.is_whitespace()).map(|c| if c == ',' { '.' } else { c }).collect();
    if value.is_empty() {
        return Ok(None);
    }
    value.parse().map(Some).map_err(|_| format!("{name}: введите число"))
}

fn parse_percent(input: &str, name: &str) -> Result<Option<f64>, String> {
    match parse_number(input, name)? {
        Some(value) if !(0.0..=100.0).contains(&value) => Err(format!("{name}: от 0 до 100")),
        other => Ok(other.map(|v| v / 100.0)),
    }
}

fn parse_period(input: &str, name: &str) -> Result<Option<i32>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let parsed = input.split_once('.').and_then(|(month, year)| Some((month.parse::<i32>().ok()?, year.parse::<i32>().ok()?)));
    match parsed {
        Some((month, year)) if (1..=12).contains(&month) && year > 999 => Ok(Some(year * 100 + month)),
        _ => Err(format!("{name}: формат ММ.ГГГГ")),
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

    pub fn to_filters(&self, search: &str) -> Result<Filters, String> {
        let date_from = parse_date(&self.date_from, "Дата с")?;
        let date_to = parse_date(&self.date_to, "Дата по")?;
        if let (Some(from), Some(to)) = (date_from, date_to)
            && from > to
        {
            return Err("Начало периода позже конца.".into());
        }
        let card = self.card.trim();
        Ok(Filters {
            search: search.to_string(),
            date_from,
            date_to,
            days_of_week: self.days.clone(),
            amount_min: parse_number(&self.amount_min, "Сумма от")?,
            amount_max: parse_number(&self.amount_max, "Сумма до")?,
            currency: self.currency.clone(),
            transaction_types: self.types.clone(),
            operators: self.operators.clone(),
            apps: self.apps.clone(),
            source_channel: self.channel.clone(),
            source_chat_ids: self.chats.clone(),
            card: (!card.is_empty()).then(|| card.to_string()),
            parsing_method: self.method.clone(),
            confidence_min: parse_percent(&self.confidence_min, "Уверенность от")?,
            confidence_max: parse_percent(&self.confidence_max, "Уверенность до")?,
            annotation_details: self.details.trim().to_string(),
            annotation_persons: self.persons.clone(),
            annotation_articles: self.articles.clone(),
            annotation_period_from: parse_period(&self.period_from, "Период с")?,
            annotation_period_to: parse_period(&self.period_to, "Период по")?,
            annotation_state: self.state,
        })
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
        };
        *target = value;
    }
}

fn send(message: Msg) -> Screen {
    Screen::Filters(message)
}

fn typed<'a>(placeholder: &str, value: &str, field: Field) -> iced::widget::TextInput<'a, Screen> {
    ui::input(placeholder, value).on_input(move |text| send(Msg::Text(field, text)))
}

fn wrap_row<'a>(chips: Vec<El<'a, Screen>>) -> El<'a, Screen> {
    iced::widget::Row::with_children(chips).spacing(space::XS).wrap().vertical_spacing(space::XS).into()
}

/// One choice among several, with «Все» for no choice at all.
fn pick_one<'a>(items: &[(&'a str, &'a str)], active: &Option<String>, make: fn(Option<String>) -> Msg) -> El<'a, Screen> {
    let mut chips = vec![ui::chip("Все", active.is_none(), send(make(None)))];
    for (code, name) in items {
        chips.push(ui::chip(*name, active.as_deref() == Some(code), send(make(Some(code.to_string())))));
    }
    wrap_row(chips)
}

/// A searchable list of checkboxes. What is ticked stays on top.
fn check_list<'a>(items: &[(String, String)], chosen: &[String], search: Option<(&str, &str, Field)>, make: fn(String) -> Msg) -> El<'a, Screen> {
    let needle = search.map(|(_, value, _)| value.trim().to_lowercase()).unwrap_or_default();
    let mut ordered: Vec<&(String, String)> = items.iter().filter(|(code, name)| chosen.contains(code) || name.to_lowercase().contains(&needle)).collect();
    ordered.sort_by_key(|(code, _)| !chosen.contains(code));
    let shown = ordered.len().min(LIST_LIMIT);
    let list = ordered[..shown].iter().fold(Stack::new().spacing(space::XS), |list, (code, name)| {
        let code = code.clone();
        list.push(ui::check(name.clone(), chosen.contains(&code), move |_| send(make(code.clone()))))
    });
    let empty = shown == 0;
    let mut block = column![].spacing(space::SM);
    if let Some((placeholder, value, field)) = search {
        block = block.push(typed(placeholder, value, field));
    }
    if empty {
        return block.push(ui::faint("Нет значений")).into();
    }
    let frame = container(ui::scroll(container(list).padding(space::SM).width(Fill)).height(Length::Fixed(LIST_HEIGHT))).width(Fill).style(theme::inset);
    block.push(frame).into()
}

fn same(items: &[String]) -> Vec<(String, String)> {
    items.iter().map(|item| (item.clone(), item.clone())).collect()
}

impl Transactions {
    pub(super) fn open_filters(&mut self) {
        self.sheet = Sheet::Filters(Box::new(FilterDraft::from(&self.table.filters)));
    }

    pub(super) fn filters_update(&mut self, message: Msg, ctx: &mut Ctx) {
        let Sheet::Filters(draft) = &mut self.sheet else { return };
        match message {
            Msg::Text(field, value) => {
                draft.set_text(field, value);
                draft.error = None;
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
            Msg::Reset => **draft = FilterDraft::default(),
            Msg::Close => self.sheet = Sheet::None,
            Msg::Apply => match draft.to_filters(&self.search) {
                Ok(filters) => {
                    self.table.filters = filters;
                    self.sheet = Sheet::None;
                    self.refresh_rows();
                    self.save(ctx);
                }
                Err(reason) => draft.error = Some(reason),
            },
        }
    }

    pub(super) fn filters_view<'a>(&'a self, draft: &'a FilterDraft) -> El<'a, Screen> {
        let days = DAYS.iter().map(|(id, name)| ui::chip(*name, draft.days.contains(id), send(Msg::Day(*id)))).collect();
        let types = TYPES.iter().map(|(code, name)| ui::chip(*name, draft.types.iter().any(|t| t == code), send(Msg::Type(code.to_string())))).collect();
        let currencies: [(&str, &str); 2] = [("UZS", "UZS"), ("USD", "USD")];

        let period = column![
            row![
                ui::field("с", typed("ДД.ММ.ГГГГ", &draft.date_from, Field::DateFrom)).width(Fill),
                ui::field("по", typed("ДД.ММ.ГГГГ", &draft.date_to, Field::DateTo)).width(Fill),
            ]
            .spacing(space::MD),
            wrap_row(days),
        ]
        .spacing(space::SM);

        let money = column![
            row![typed("0", &draft.amount_min, Field::AmountMin), typed("∞", &draft.amount_max, Field::AmountMax)].spacing(space::MD),
            pick_one(&currencies, &draft.currency, Msg::Currency),
            wrap_row(types),
        ]
        .spacing(space::SM);

        let mut source = column![pick_one(&CHANNELS, &draft.channel, Msg::Channel)].spacing(space::SM);
        if draft.channel.as_deref() == Some("TELEGRAM") && !self.data.telegram_sources.is_empty() {
            let items: Vec<(String, String)> = self.data.telegram_sources.iter().map(|(id, name)| (id.to_string(), name.clone())).collect();
            let chosen: Vec<String> = draft.chats.iter().map(i64::to_string).collect();
            source = source.push(check_list(&items, &chosen, Some(("Бот, группа, канал", &draft.chat_search, Field::ChatSearch)), |id| {
                id.parse().map_or(Msg::Chat(0), Msg::Chat)
            }));
        }

        let parser = column![
            pick_one(&METHODS, &draft.method, Msg::Method),
            row![typed("0", &draft.confidence_min, Field::ConfidenceMin), ui::muted("—"), typed("100", &draft.confidence_max, Field::ConfidenceMax), ui::muted("%")]
                .spacing(space::SM)
                .align_y(iced::Alignment::Center),
        ]
        .spacing(space::SM);

        let states = [(AnnotationState::All, "Все"), (AnnotationState::Annotated, "Размечены"), (AnnotationState::Missing, "Без разметки")];
        let state_chips = states.iter().map(|(state, name)| ui::chip(*name, draft.state == *state, send(Msg::State(*state)))).collect();
        let mut accounting = column![
            typed("ст парковка", &draft.details, Field::Details),
            row![
                ui::field("Период с", typed("ММ.ГГГГ", &draft.period_from, Field::PeriodFrom)).width(Fill),
                ui::field("Период по", typed("ММ.ГГГГ", &draft.period_to, Field::PeriodTo)).width(Fill),
            ]
            .spacing(space::MD),
            wrap_row(state_chips),
        ]
        .spacing(space::SM);
        if !self.choices.persons.is_empty() {
            accounting = accounting.push(ui::field("Люди", check_list(&self.choices.persons, &draft.persons, None, Msg::Person)));
        }
        if !self.choices.articles.is_empty() {
            accounting = accounting.push(ui::field("Статьи", check_list(&self.choices.articles, &draft.articles, None, Msg::Article)));
        }

        let mut body = column![
            ui::section("Период", period),
            ui::section("Финансы", money),
            ui::section("Источник", source),
            ui::section("Операторы", check_list(&same(&self.data.operators), &draft.operators, Some(("Поиск оператора", &draft.operator_search, Field::OperatorSearch)), Msg::Operator)),
            ui::section("Приложения", check_list(&same(&self.data.apps), &draft.apps, Some(("Поиск приложения", &draft.app_search, Field::AppSearch)), Msg::App)),
            ui::section("Парсер · уверенность", parser),
            ui::section("Учёт", accounting),
            ui::section("Идентификаторы", typed("1234", &draft.card, Field::Card)),
        ]
        .spacing(space::XL);
        if let Some(reason) = &draft.error {
            body = body.push(ui::notice(Tone::Danger, reason.as_str()));
        }

        let footer = row![
            ui::btn("Сбросить всё", Kind::Ghost, Some(send(Msg::Reset))),
            ui::fill(),
            ui::btn("Отмена", Kind::Secondary, Some(send(Msg::Close))),
            ui::btn("Применить", Kind::Primary, Some(send(Msg::Apply))),
        ]
        .spacing(space::SM);
        let panel = ui::sheet_panel("Уточнение", "Фильтры", "Точная настройка выборки. Применяется ко всем колонкам.", send(Msg::Close), body, Some(footer.into()));
        ui::sheet(iced::widget::Space::new(), panel, WIDTH, Some(send(Msg::Close)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
