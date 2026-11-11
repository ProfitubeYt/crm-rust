//! Who did what to access: read only, for administrators.

use std::borrow::Cow;
use std::time::Duration;

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Space, column, container, row, scrollable, text};
use iced::{Alignment, Event, Fill, Subscription, Task, event, window};
use tbs_core::api::AuditRecord;
use tbs_core::format::{self, EMPTY};

use crate::ctx::{Ctx, text_error};
use crate::grid::{self, CellStyle, GridColumn, GridData, GridEvent, Selection};
use crate::theme::{self, DARK, LIGHT, Tone, size, space};
use crate::ui::{self, El, Icon, Kind};

const LIMIT: i64 = 200;
const DEBOUNCE: Duration = Duration::from_millis(300);
const RESULT: usize = 2;

fn details_line(record: &AuditRecord) -> String {
    record.details.as_ref().map_or_else(|| EMPTY.to_owned(), ToString::to_string)
}

fn details_block(record: &AuditRecord) -> String {
    record.details.as_ref().and_then(|value| serde_json::to_string_pretty(value).ok()).unwrap_or_else(|| EMPTY.to_owned())
}

/// A date the server can parse, or nothing while it is still being typed.
fn date_ok(raw: &str) -> bool {
    raw.trim().is_empty() || format::parse_date_input(raw).is_some()
}

/// The date as the server takes it. The table shows ДД.ММ.ГГГГ, so that is
/// what gets typed here.
fn iso_date(raw: &str) -> Option<String> {
    format::parse_date_input(raw).map(|date| date.format("%Y-%m-%d").to_string())
}

#[derive(Clone, Debug)]
pub enum Field {
    Action(String),
    User(String),
    From(String),
    To(String),
}

#[derive(Clone, Debug)]
pub enum Msg {
    Field(Field),
    Due(u64),
    Refresh,
    Loaded(u64, Result<Vec<AuditRecord>, String>),
    Page(i64),
    Grid(GridEvent),
    CloseDetail,
    Escape,
}

#[cfg(test)]
impl Audit {
    pub fn actions(&self) -> Vec<String> {
        self.rows.iter().map(|row| row.action.clone()).collect()
    }
}

pub struct Audit {
    rows: Vec<AuditRecord>,
    action: String,
    user: String,
    from: String,
    to: String,
    page: i64,
    loaded: bool,
    busy: bool,
    failed: Option<String>,
    token: u64,
    typed: u64,
    selection: Selection,
    columns: Vec<GridColumn>,
    detail: Option<i64>,
}

struct Rows<'a> {
    rows: &'a [AuditRecord],
    dark: bool,
}

impl GridData for Rows<'_> {
    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let Some(record) = self.rows.get(row) else { return Cow::Borrowed("") };
        match col {
            0 => Cow::Owned(format::datetime_str(record.created_at.as_deref())),
            1 => Cow::Borrowed(&record.action),
            RESULT => Cow::Borrowed(if record.success { "OK" } else { "Ошибка" }),
            3 => record.user_id.map_or(Cow::Borrowed(EMPTY), |id| Cow::Owned(id.to_string())),
            4 => Cow::Borrowed(record.ip_address.as_deref().unwrap_or(EMPTY)),
            _ => Cow::Owned(details_line(record)),
        }
    }

    fn cell_style(&self, row: usize, col: usize) -> CellStyle {
        let palette = if self.dark { &DARK } else { &LIGHT };
        let Some(record) = self.rows.get(row) else { return CellStyle::default() };
        let color = (col == RESULT).then_some(if record.success { palette.success } else { palette.danger });
        CellStyle { color, ..CellStyle::default() }
    }
}

fn columns() -> Vec<GridColumn> {
    vec![
        GridColumn::new("Время", 136.0),
        GridColumn::new("Действие", 240.0).medium(),
        GridColumn::new("Результат", 92.0),
        GridColumn::new("Пользователь", 110.0).mono(),
        GridColumn::new("IP", 130.0).mono(),
        GridColumn::new("Детали", 520.0).mono(),
    ]
}

fn hotkey(event: Event, _status: event::Status, _window: window::Id) -> Option<Msg> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event else { return None };
    (key == Key::Named(Named::Escape)).then_some(Msg::Escape)
}

impl Audit {
    pub fn new(ctx: &Ctx) -> (Self, Task<Msg>) {
        let mut screen = Self {
            rows: Vec::new(),
            action: String::new(),
            user: String::new(),
            from: String::new(),
            to: String::new(),
            page: 0,
            loaded: false,
            busy: false,
            failed: None,
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
        event::listen_with(hotkey)
    }

    #[cfg(feature = "devtools")]
    pub fn probe(name: &str) -> Option<Msg> {
        (name == "audit-detail").then_some(Msg::Grid(GridEvent::Click { row: 0, col: 0, at: iced::Point::ORIGIN }))
    }

    fn query(&self) -> Vec<(&'static str, String)> {
        let mut query = vec![("limit", LIMIT.to_string()), ("offset", (self.page * LIMIT).to_string())];
        let action = self.action.trim();
        if !action.is_empty() {
            query.push(("action", action.to_owned()));
        }
        if let Ok(id) = self.user.trim().parse::<i64>() {
            query.push(("user_id", id.to_string()));
        }
        if let Some(date) = iso_date(&self.from) {
            query.push(("date_from", date));
        }
        if let Some(date) = iso_date(&self.to) {
            query.push(("date_to", date));
        }
        query
    }

    fn reload(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.token += 1;
        self.busy = true;
        let token = self.token;
        let query = self.query();
        let api = ctx.api.clone();
        Task::perform(
            async move {
                let query: Vec<(&str, String)> = query.iter().map(|(key, value)| (*key, value.clone())).collect();
                api.audit(&query).await.map_err(text_error)
            },
            move |reply| Msg::Loaded(token, reply),
        )
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Field(field) => {
                match field {
                    Field::Action(value) => self.action = value,
                    Field::User(value) => self.user = value.chars().filter(char::is_ascii_digit).collect(),
                    Field::From(value) => self.from = value,
                    Field::To(value) => self.to = value,
                }
                self.typed += 1;
                let typed = self.typed;
                return Task::perform(tokio::time::sleep(DEBOUNCE), move |()| Msg::Due(typed));
            }
            Msg::Due(typed) => {
                if typed == self.typed {
                    self.page = 0;
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
                self.selection = Selection::default();
                match reply {
                    Ok(rows) => {
                        self.rows = rows;
                        self.failed = None;
                    }
                    Err(reason) => self.failed = Some(reason),
                }
            }
            Msg::Page(page) => {
                self.page = page.max(0);
                return self.reload(ctx);
            }
            Msg::Grid(event) => match event {
                GridEvent::Select(selection) => self.selection = selection,
                GridEvent::Click { row, .. } | GridEvent::Activate { row, .. } => self.detail = self.rows.get(row).map(|record| record.id),
                GridEvent::Resize { col, width } => {
                    if let Some(column) = self.columns.get_mut(col) {
                        column.width = width;
                    }
                }
                _ => {}
            },
            Msg::CloseDetail | Msg::Escape => self.detail = None,
        }
        Task::none()
    }

    pub fn view(&self, ctx: &Ctx) -> El<'_, Msg> {
        let date = |hint: &str, value: &str, wrap: fn(String) -> Field| {
            let input = ui::input(hint, value).on_input(move |typed| Msg::Field(wrap(typed))).width(Fill);
            if date_ok(value) { input } else { input.style(theme::input_error) }
        };
        let filters = row![
            ui::input("Действие", &self.action).on_input(|value| Msg::Field(Field::Action(value))).width(Fill),
            ui::input("ID пользователя", &self.user).on_input(|value| Msg::Field(Field::User(value))).width(Fill),
            date("Дата от (ДД.ММ.ГГГГ)", &self.from, Field::From),
            date("Дата до (ДД.ММ.ГГГГ)", &self.to, Field::To),
            ui::btn_icon(Icon::Refresh, "Обновить", Kind::Secondary, Some(Msg::Refresh)),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);

        let body: El<'_, Msg> = if !self.loaded {
            ui::loading("Загрузка...")
        } else if let Some(reason) = &self.failed {
            container(ui::notice(Tone::Danger, reason.as_str())).padding(space::MD).into()
        } else if self.rows.is_empty() {
            ui::empty_state(Icon::Shield, "Записей нет", "Нет записей по текущим фильтрам.")
        } else {
            grid::grid(Rows { rows: &self.rows, dark: ctx.dark }, &self.columns, &self.selection, Msg::Grid).row_height(32.0).font_size(13.0).into()
        };

        let more = self.rows.len() as i64 >= LIMIT && !self.busy;
        let footer = row![
            ui::muted(format!("Страница: {}", self.page + 1)),
            ui::fill(),
            ui::btn("Назад", Kind::Secondary, (self.page > 0 && !self.busy).then(|| Msg::Page(self.page - 1))),
            ui::btn("Далее", Kind::Secondary, more.then(|| Msg::Page(self.page + 1))),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);

        let page = column![ui::page_header("Аудит безопасности", None, Space::new()), filters, container(body).width(Fill).height(Fill).style(theme::card), footer].spacing(space::MD).height(Fill);
        ui::page(page)
    }

    fn detail_view<'a>(&'a self, record: &'a AuditRecord) -> El<'a, Msg> {
        let line = |caption: &'static str, value: String| -> El<'a, Msg> { column![ui::faint(caption), text(value).size(size::SM)].spacing(space::XXS).width(Fill).into() };
        let tone = if record.success { Tone::Success } else { Tone::Danger };
        let result: El<'a, Msg> = column![ui::faint("Результат"), ui::badge(if record.success { "OK" } else { "Ошибка" }, tone)].spacing(space::XXS).width(Fill).into();
        let optional = |value: Option<i64>| value.map_or_else(|| EMPTY.to_owned(), |id| id.to_string());
        let block = container(scrollable(text(details_block(record)).size(size::SM).font(theme::FONT_MONO)).style(theme::scroller).width(Fill)).padding(space::SM).max_height(280.0).width(Fill).style(theme::inset);
        let body = column![
            ui::dialog_title(record.action.clone(), Some(Msg::CloseDetail)),
            row![line("Время", format::datetime_str(record.created_at.as_deref())), result].spacing(space::LG),
            row![line("Пользователь", optional(record.user_id)), line("IP", record.ip_address.clone().unwrap_or_else(|| EMPTY.to_owned()))].spacing(space::LG),
            column![ui::faint("Детали"), block].spacing(space::XS),
            row![ui::fill(), ui::btn("Закрыть", Kind::Secondary, Some(Msg::CloseDetail))],
        ]
        .spacing(space::MD);
        ui::modal(Space::new(), body, 560.0, Some(Msg::CloseDetail))
    }

    pub fn overlay(&self, _ctx: &Ctx) -> Option<El<'_, Msg>> {
        let record = self.rows.iter().find(|record| Some(record.id) == self.detail)?;
        Some(self.detail_view(record))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_typed_dates_stay_out_of_the_query() {
        assert!(date_ok(""));
        assert!(date_ok("2026-10-19"));
        assert!(date_ok("19.10.2026"));
        assert_eq!(iso_date("19.10.2026").as_deref(), Some("2026-10-19"));
        assert_eq!(iso_date("2026-10-19").as_deref(), Some("2026-10-19"));
        assert_eq!(iso_date(""), None);
        assert!(!date_ok("2026-1"));
    }

    #[test]
    fn details_read_as_json_and_absent_ones_as_a_dash() {
        let record = AuditRecord { details: Some(serde_json::json!({"a": 1})), ..AuditRecord::default() };
        assert_eq!(details_line(&record), r#"{"a":1}"#);
        assert_eq!(details_line(&AuditRecord::default()), EMPTY);
    }
}
