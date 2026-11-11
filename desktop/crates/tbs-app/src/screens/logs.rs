//! What the worker did with each receipt message.

use std::borrow::Cow;
use std::fmt;
use std::time::Duration;

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Id, Space, column, container, operation, row, scrollable, text};
use iced::{Alignment, Event, Fill, Subscription, Task, event, window};
use tbs_core::api::{LogEntry, LogPage, LogStats};
use tbs_core::format::{self, EMPTY, count, parse_datetime};

use crate::ctx::{Ctx, text_error};
use crate::grid::{self, CellStyle, GridColumn, GridData, GridEvent, Selection};
use crate::theme::{self, DARK, LIGHT, Tone, size, space};
use crate::ui::{self, El, Icon, Kind};

const PAGE_SIZE: i64 = 50;
const REFRESH: Duration = Duration::from_secs(10);
const DEBOUNCE: Duration = Duration::from_millis(240);
const SEARCH_ID: &str = "logs-search";

const STATUS: usize = 2;
const ERROR: usize = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    All,
    Done,
    Failed,
    Processing,
    Queued,
}

impl Status {
    const ALL: [Status; 5] = [Status::All, Status::Done, Status::Failed, Status::Processing, Status::Queued];

    fn code(self) -> Option<&'static str> {
        match self {
            Status::All => None,
            Status::Done => Some("done"),
            Status::Failed => Some("failed"),
            Status::Processing => Some("processing"),
            Status::Queued => Some("queued"),
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::All => "Все статусы",
            Status::Done => "Успешно",
            Status::Failed => "Ошибка",
            Status::Processing => "В процессе",
            Status::Queued => "В очереди",
        })
    }
}

/// Words and tone for one line. Statuses the client does not know stay readable as they came.
fn status_of(entry: &LogEntry) -> (Cow<'_, str>, Tone) {
    if entry.is_duplicate {
        return (Cow::Borrowed("Дубликат"), Tone::Warning);
    }
    match entry.status.as_str() {
        "done" => (Cow::Borrowed("Успешно"), Tone::Success),
        "failed" | "dead" => (Cow::Borrowed("Ошибка"), Tone::Danger),
        "retry" => (Cow::Borrowed("Повтор"), Tone::Warning),
        "processing" => (Cow::Borrowed("В процессе"), Tone::Accent),
        "queued" => (Cow::Borrowed("В очереди"), Tone::Neutral),
        other => (Cow::Borrowed(other), Tone::Neutral),
    }
}

fn amount_of(entry: &LogEntry) -> String {
    match &entry.amount {
        Some(amount) => format!("{} {}", format::amount(Some(amount)), entry.currency.as_deref().unwrap_or("UZS")),
        None => EMPTY.to_owned(),
    }
}

fn with_seconds(raw: Option<&str>) -> String {
    raw.and_then(parse_datetime).map_or_else(|| EMPTY.to_owned(), |moment| moment.format("%d.%m.%Y %H:%M:%S").to_string())
}

fn short_print(print: &str) -> String {
    let head: String = print.chars().take(16).collect();
    if head.len() < print.len() { format!("{head}...") } else { head }
}

fn page_count(total: i64) -> i64 {
    ((total + PAGE_SIZE - 1) / PAGE_SIZE).max(1)
}

/// "Показано A–B из N" for the page on screen.
fn range_label(page: i64, shown: usize, total: i64) -> String {
    let first = (page - 1) * PAGE_SIZE + 1;
    format!("Показано {first}–{} из {}", first - 1 + shown as i64, count(total.max(0) as usize))
}

#[derive(Clone, Debug)]
pub enum Msg {
    Tick,
    Refresh,
    Loaded(u64, Result<LogPage, String>),
    Stats(Result<LogStats, String>),
    Search(String),
    SearchDue(u64),
    Status(Status),
    Duplicates(bool),
    ToggleAuto,
    Page(i64),
    Grid(GridEvent),
    CloseDetail,
    Escape,
    FocusSearch,
}

#[cfg(test)]
impl Logs {
    pub fn peek(&self) -> (usize, i64, i64) {
        (self.items.len(), self.total, self.page)
    }
}

pub struct Logs {
    items: Vec<LogEntry>,
    total: i64,
    stats: Option<LogStats>,
    page: i64,
    search: String,
    status: Status,
    duplicates: bool,
    auto: bool,
    loaded: bool,
    busy: bool,
    failed: Option<String>,
    stats_failed: bool,
    /// Only the newest request may land.
    token: u64,
    typed: u64,
    selection: Selection,
    columns: Vec<GridColumn>,
    detail: Option<i64>,
}

struct Rows<'a> {
    items: &'a [LogEntry],
    dark: bool,
}

impl GridData for Rows<'_> {
    fn row_count(&self) -> usize {
        self.items.len()
    }

    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let Some(entry) = self.items.get(row) else { return Cow::Borrowed("") };
        match col {
            0 => Cow::Owned(format::datetime_str(entry.created_at.as_deref())),
            1 => Cow::Owned(format!("{} / {}", entry.chat_id, entry.message_id)),
            STATUS => status_of(entry).0,
            3 => Cow::Borrowed(entry.operator_raw.as_deref().unwrap_or(EMPTY)),
            4 => Cow::Owned(amount_of(entry)),
            _ => Cow::Borrowed(entry.error.as_deref().unwrap_or("")),
        }
    }

    fn cell_style(&self, row: usize, col: usize) -> CellStyle {
        let palette = if self.dark { &DARK } else { &LIGHT };
        let Some(entry) = self.items.get(row) else { return CellStyle::default() };
        let color = match col {
            STATUS => match status_of(entry).1 {
                Tone::Neutral => None,
                tone => Some(palette.tone(tone).0),
            },
            ERROR => Some(palette.danger),
            _ => None,
        };
        CellStyle { color, ..CellStyle::default() }
    }
}

fn columns() -> Vec<GridColumn> {
    vec![
        GridColumn::new("Дата", 136.0),
        GridColumn::new("Чат / Сообщение", 200.0).mono(),
        GridColumn::new("Статус", 104.0).medium(),
        GridColumn::new("Оператор", 240.0),
        GridColumn::new("Сумма", 150.0).align(grid::Align::Right).mono(),
        GridColumn::new("Ошибка", 420.0),
    ]
}

fn hotkey(event: Event, _status: event::Status, _window: window::Id) -> Option<Msg> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, physical_key, .. }) = event else { return None };
    if key == Key::Named(Named::Escape) {
        return Some(Msg::Escape);
    }
    (modifiers.command() && key.to_latin(physical_key) == Some('k')).then_some(Msg::FocusSearch)
}

impl Logs {
    pub fn new(ctx: &Ctx) -> (Self, Task<Msg>) {
        let mut screen = Self {
            items: Vec::new(),
            total: 0,
            stats: None,
            page: 1,
            search: String::new(),
            status: Status::All,
            duplicates: false,
            auto: true,
            loaded: false,
            busy: false,
            failed: None,
            stats_failed: false,
            token: 0,
            typed: 0,
            selection: Selection::default(),
            columns: columns(),
            detail: None,
        };
        let load = screen.reload(ctx);
        (screen, load)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        let keys = event::listen_with(hotkey);
        if self.auto && self.detail.is_none() {
            Subscription::batch([keys, iced::time::every(REFRESH).map(|_| Msg::Tick)])
        } else {
            keys
        }
    }

    #[cfg(feature = "devtools")]
    pub fn probe(name: &str) -> Option<Msg> {
        (name == "log-detail").then_some(Msg::Grid(GridEvent::Click { row: 0, col: 0, at: iced::Point::ORIGIN }))
    }

    fn query(&self) -> Vec<(&'static str, String)> {
        let mut query = vec![("page", self.page.to_string()), ("page_size", PAGE_SIZE.to_string())];
        if let Some(code) = self.status.code() {
            query.push(("status", code.to_owned()));
        }
        let needle = self.search.trim();
        if !needle.is_empty() {
            query.push(("search", needle.to_owned()));
        }
        if self.duplicates {
            query.push(("duplicates_only", "true".to_owned()));
        }
        query
    }

    fn reload(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.token += 1;
        self.busy = true;
        let token = self.token;
        let query = self.query();
        let (list_api, stats_api) = (ctx.api.clone(), ctx.api.clone());
        Task::batch([
            Task::perform(
                async move {
                    let query: Vec<(&str, String)> = query.iter().map(|(key, value)| (*key, value.clone())).collect();
                    list_api.logs(&query).await.map_err(text_error)
                },
                move |reply| Msg::Loaded(token, reply),
            ),
            Task::perform(async move { stats_api.log_stats().await.map_err(text_error) }, Msg::Stats),
        ])
    }

    /// Filters change what page 1 means, so they always go back to it.
    fn first_page(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.page = 1;
        self.selection = Selection::default();
        self.reload(ctx)
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Tick => {
                if !self.busy {
                    return self.reload(ctx);
                }
            }
            Msg::Refresh => return self.reload(ctx),
            Msg::Loaded(token, reply) => {
                if token != self.token {
                    return Task::none();
                }
                self.busy = false;
                self.loaded = true;
                match reply {
                    Ok(page) => {
                        self.items = page.items;
                        self.total = page.total;
                        self.failed = None;
                        self.selection = Selection::default();
                    }
                    Err(reason) => {
                        // A dead server would otherwise repeat this every ten seconds.
                        if self.failed.is_none() {
                            ctx.toast(Tone::Danger, "Ошибка загрузки логов");
                        }
                        self.failed = Some(reason);
                    }
                }
            }
            Msg::Stats(reply) => match reply {
                Ok(stats) => {
                    self.stats = Some(stats);
                    self.stats_failed = false;
                }
                Err(_) => {
                    if !self.stats_failed {
                        ctx.toast(Tone::Danger, "Ошибка загрузки статистики");
                    }
                    self.stats_failed = true;
                }
            },
            Msg::Search(value) => {
                self.search = value;
                self.typed += 1;
                let typed = self.typed;
                return Task::perform(tokio::time::sleep(DEBOUNCE), move |()| Msg::SearchDue(typed));
            }
            Msg::SearchDue(typed) => {
                if typed == self.typed {
                    return self.first_page(ctx);
                }
            }
            Msg::Status(status) => {
                self.status = status;
                return self.first_page(ctx);
            }
            Msg::Duplicates(value) => {
                self.duplicates = value;
                return self.first_page(ctx);
            }
            Msg::ToggleAuto => self.auto = !self.auto,
            Msg::Page(page) => {
                self.page = page;
                return self.reload(ctx);
            }
            Msg::Grid(event) => match event {
                GridEvent::Select(selection) => self.selection = selection,
                GridEvent::Click { row, .. } | GridEvent::Activate { row, .. } => self.detail = self.items.get(row).map(|entry| entry.id),
                GridEvent::Resize { col, width } => {
                    if let Some(column) = self.columns.get_mut(col) {
                        column.width = width;
                    }
                }
                _ => {}
            },
            Msg::CloseDetail | Msg::Escape => self.detail = None,
            Msg::FocusSearch => return operation::focus(Id::new(SEARCH_ID)),
        }
        Task::none()
    }

    fn tiles(&self) -> El<'_, Msg> {
        let value = |pick: fn(&LogStats) -> i64| self.stats.as_ref().map_or_else(|| EMPTY.to_owned(), |stats| count(pick(stats).max(0) as usize));
        let tile = |caption: &'static str, amount: String, tone: Tone| -> El<'_, Msg> {
            let body = column![ui::faint(caption), ui::toned(amount, tone).size(size::XL).font(theme::FONT_BOLD)].spacing(space::XXS);
            container(body).padding([space::SM, space::MD]).width(Fill).style(theme::card).into()
        };
        row![
            tile("Всего", value(|s| s.total), Tone::Neutral),
            tile("Успешно", value(|s| s.success), Tone::Success),
            tile("Ошибок", value(|s| s.failed), Tone::Danger),
            tile("Дубликатов", value(|s| s.duplicates), Tone::Warning),
            tile("В процессе", value(|s| s.processing), Tone::Accent),
        ]
        .spacing(space::SM)
        .into()
    }

    pub fn view(&self, ctx: &Ctx) -> El<'_, Msg> {
        let hint = if self.auto { "Остановить автообновление" } else { "Включить автообновление (10 сек)" };
        let auto = ui::hint_on(ui::chip("Авто", self.auto, Msg::ToggleAuto), hint);
        let toolbar = row![
            ui::input("Поиск по ошибке...", &self.search).id(Id::new(SEARCH_ID)).on_input(Msg::Search).width(Fill),
            ui::select(&Status::ALL[..], Some(self.status), Msg::Status).width(160.0),
            ui::check("Только дубликаты", self.duplicates, Msg::Duplicates),
            auto,
            ui::btn_icon(Icon::Refresh, "Обновить", Kind::Secondary, Some(Msg::Refresh)),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);

        let body: El<'_, Msg> = if !self.loaded {
            ui::loading("Загрузка...")
        } else if let (Some(reason), true) = (&self.failed, self.items.is_empty()) {
            container(ui::notice(Tone::Danger, reason.as_str())).padding(space::MD).into()
        } else if self.items.is_empty() {
            ui::empty_state(Icon::Logs, "Нет данных", "Попробуйте изменить фильтры или обновить список.")
        } else {
            grid::grid(Rows { items: &self.items, dark: ctx.dark }, &self.columns, &self.selection, Msg::Grid).row_height(32.0).font_size(13.0).into()
        };

        let mut page = column![self.tiles(), toolbar, container(body).width(Fill).height(Fill).style(theme::card)].spacing(space::MD).height(Fill);
        let pages = page_count(self.total);
        if pages > 1 {
            page = page.push(row![ui::muted(range_label(self.page, self.items.len(), self.total)), ui::fill(), ui::pager(self.page, pages, Msg::Page)].align_y(Alignment::Center));
        }
        container(column![ui::page_header("Логи", Some("Что воркер сделал с каждым сообщением."), Space::new()), page].spacing(space::MD))
            .padding(ui::PAGE_PADDING)
            .width(Fill)
            .height(Fill)
            .style(theme::page)
            .into()
    }

    fn detail_view<'a>(&'a self, entry: &'a LogEntry) -> El<'a, Msg> {
        let line = |caption: &'static str, value: String, mono: bool| -> El<'a, Msg> {
            let value = if mono { text(value).size(size::SM).font(theme::FONT_MONO) } else { text(value).size(size::SM) };
            column![ui::faint(caption), value].spacing(space::XXS).width(Fill).into()
        };
        let pair = |left: El<'a, Msg>, right: El<'a, Msg>| -> El<'a, Msg> { row![left, right].spacing(space::LG).into() };
        let (status, tone) = status_of(entry);
        let status: El<'a, Msg> = column![ui::faint("Статус"), ui::badge(status.into_owned(), tone)].spacing(space::XXS).width(Fill).into();
        let optional = |value: Option<i64>| value.map_or_else(|| EMPTY.to_owned(), |id| id.to_string());
        let print = entry.fingerprint.as_deref().map_or_else(|| EMPTY.to_owned(), short_print);

        let mut body = column![
            ui::dialog_title(format!("Детали записи #{}", entry.id), Some(Msg::CloseDetail)),
            pair(line("Task ID", entry.task_id.clone(), true), status),
            pair(line("Chat ID", entry.chat_id.to_string(), true), line("Message ID", entry.message_id.to_string(), true)),
            pair(line("Transaction ID", optional(entry.transaction_id), true), line("Fingerprint", print, true)),
            pair(line("Создано", with_seconds(entry.created_at.as_deref()), false), line("Обновлено", with_seconds(entry.updated_at.as_deref()), false)),
            pair(line("Оператор", entry.operator_raw.clone().unwrap_or_else(|| EMPTY.to_owned()), false), line("Сумма", amount_of(entry), true)),
        ]
        .spacing(space::MD);
        if let Some(error) = &entry.error {
            body = body.push(column![ui::faint("Ошибка"), ui::notice(Tone::Danger, error.as_str())].spacing(space::XS));
        }
        if let Some(raw) = &entry.raw_message {
            let block = container(scrollable(text(raw.as_str()).size(size::SM).font(theme::FONT_MONO)).style(theme::scroller).width(Fill)).padding(space::SM).max_height(220.0).width(Fill).style(theme::inset);
            body = body.push(column![ui::faint("Исходное сообщение"), block].spacing(space::XS));
        }
        body = body.push(row![ui::fill(), ui::btn("Закрыть", Kind::Secondary, Some(Msg::CloseDetail))]);
        ui::modal(Space::new(), body, 620.0, Some(Msg::CloseDetail))
    }

    pub fn overlay(&self, _ctx: &Ctx) -> Option<El<'_, Msg>> {
        let entry = self.items.iter().find(|entry| Some(entry.id) == self.detail)?;
        Some(self.detail_view(entry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(status: &str) -> LogEntry {
        LogEntry { status: status.to_owned(), ..LogEntry::default() }
    }

    #[test]
    fn a_duplicate_wins_over_the_status() {
        let duplicate = LogEntry { is_duplicate: true, ..entry("failed") };
        assert_eq!(status_of(&duplicate).0, "Дубликат");
    }

    #[test]
    fn dead_receipts_read_as_errors_and_unknown_statuses_stay_as_they_came() {
        assert_eq!(status_of(&entry("dead")).0, "Ошибка");
        assert_eq!(status_of(&entry("weird")).0, "weird");
    }

    #[test]
    fn the_amount_falls_back_to_sum() {
        let priced = LogEntry { amount: Some("1500".to_owned()), ..LogEntry::default() };
        assert!(amount_of(&priced).ends_with("UZS"));
        assert_eq!(amount_of(&LogEntry::default()), EMPTY);
    }

    #[test]
    fn a_long_print_is_cut_a_short_one_is_not() {
        assert_eq!(short_print("abc"), "abc");
        assert_eq!(short_print("0123456789abcdefXYZ"), "0123456789abcdef...");
    }

    #[test]
    fn pages_and_range_follow_the_total() {
        assert_eq!(page_count(0), 1);
        assert_eq!(page_count(50), 1);
        assert_eq!(page_count(51), 2);
        assert_eq!(range_label(2, 1, 51), "Показано 51–51 из 51");
    }
}
