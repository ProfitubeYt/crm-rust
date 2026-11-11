//! Every live token of the team, and the means to end them.

use std::borrow::Cow;
use std::time::Duration;

use iced::widget::{Column, Space, column, container, row, text};
use iced::{Alignment, Fill, Subscription, Task};
use tbs_core::api::{ActiveSession, KilledSessions};
use tbs_core::format::datetime_str;

use crate::ctx::{Ctx, text_error};
use crate::grid::{self, CellStyle, GridColumn, GridData, GridEvent, Selection};
use crate::theme::{self, DARK, LIGHT, Tone, size, space};
use crate::ui::{self, El, Icon, Kind};

use super::panel;

const REFRESH_EVERY: Duration = Duration::from_secs(15);
const COL_ACTION: usize = 7;

#[derive(Clone, Debug)]
pub enum Msg {
    Tick,
    Refresh,
    Loaded(Result<Vec<ActiveSession>, String>),
    Grid(GridEvent),
    CloseDetail,
    Kill(String),
    Killed(String, Result<serde_json::Value, String>),
    KillOthers,
    KilledOthers(Result<KilledSessions, String>),
}

pub struct Sessions {
    items: Option<Vec<ActiveSession>>,
    current: Option<String>,
    columns: Vec<GridColumn>,
    selection: Selection,
    detail: Option<String>,
    killing: bool,
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "access" => "Веб",
        "app" | "app_user" => "Приложение",
        "qr" => "QR-вход",
        "telegram" => "Telegram",
        other => other,
    }
}

fn short(id: &str, keep: usize) -> String {
    id.chars().take(keep).collect()
}

struct Rows<'a> {
    items: &'a [ActiveSession],
    current: Option<&'a str>,
    dark: bool,
}

impl GridData for Rows<'_> {
    fn row_count(&self) -> usize {
        self.items.len()
    }

    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let item = &self.items[row];
        let mine = self.current == Some(item.session_id.as_str());
        match col {
            0 => format!("{}…", short(&item.session_id, 12)).into(),
            1 => kind_label(&item.token_kind).into(),
            2 => match (&item.subject, item.user_id) {
                (Some(subject), Some(id)) => format!("{subject} · #{id}").into(),
                (Some(subject), None) => subject.as_str().into(),
                (None, Some(id)) => format!("#{id}").into(),
                _ => "—".into(),
            },
            3 => item.ip_address.as_deref().unwrap_or("—").into(),
            4 => datetime_str(item.created_at.as_deref()).into(),
            5 => datetime_str(item.last_activity_at.as_deref()).into(),
            6 => datetime_str(item.expires_at.as_deref()).into(),
            _ if mine => "текущая".into(),
            _ => "Завершить".into(),
        }
    }

    fn cell_style(&self, row: usize, col: usize) -> CellStyle {
        let c = if self.dark { &DARK } else { &LIGHT };
        let mine = self.current == Some(self.items[row].session_id.as_str());
        let color = match col {
            COL_ACTION if mine => Some(c.text3),
            COL_ACTION => Some(c.danger),
            _ => None,
        };
        CellStyle { color, ..CellStyle::default() }
    }
}

impl Sessions {
    pub fn new(_ctx: &Ctx) -> Self {
        let columns = vec![
            GridColumn::new("ID сессии", 130.0).mono(),
            GridColumn::new("Тип", 90.0),
            GridColumn::new("Пользователь", 130.0),
            GridColumn::new("IP", 100.0).mono(),
            GridColumn::new("Создана", 115.0),
            GridColumn::new("Активность", 115.0),
            GridColumn::new("Истекает", 115.0),
            GridColumn::new("Действие", 85.0),
        ];
        Self { items: None, current: None, columns, selection: Selection::default(), detail: None, killing: false }
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        iced::time::every(REFRESH_EVERY).map(|_| Msg::Tick)
    }

    pub fn escape(&mut self) {
        self.detail = None;
    }

    pub fn load(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.current = ctx.api.current_session_id();
        let api = ctx.api.clone();
        Task::perform(async move { api.sessions().await.map_err(text_error) }, Msg::Loaded)
    }

    fn others(&self) -> usize {
        self.items.as_ref().map_or(0, |items| items.iter().filter(|item| self.current.as_deref() != Some(item.session_id.as_str())).count())
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Tick | Msg::Refresh => return self.load(ctx),
            Msg::Loaded(Ok(items)) => {
                self.items = Some(items);
                self.selection = Selection::default();
            }
            Msg::Loaded(Err(reason)) => {
                self.items.get_or_insert_default();
                ctx.toast(Tone::Danger, reason);
            }
            Msg::Grid(event) => match event {
                GridEvent::Select(selection) => self.selection = selection,
                GridEvent::Click { row, col, .. } => {
                    let Some(item) = self.items.as_ref().and_then(|items| items.get(row)) else { return Task::none() };
                    let id = item.session_id.clone();
                    if col == COL_ACTION {
                        if self.current.as_deref() != Some(id.as_str()) {
                            return Task::done(Msg::Kill(id));
                        }
                    } else {
                        self.detail = Some(id);
                    }
                }
                GridEvent::Resize { col, width } => {
                    if let Some(column) = self.columns.get_mut(col) {
                        column.width = width;
                    }
                }
                _ => {}
            },
            Msg::CloseDetail => self.detail = None,
            Msg::Kill(id) => {
                let api = ctx.api.clone();
                let target = id.clone();
                return Task::perform(async move { api.kill_session(&target).await.map_err(text_error) }, move |reply| Msg::Killed(id.clone(), reply));
            }
            Msg::Killed(id, Ok(_)) => {
                ctx.toast(Tone::Success, format!("Сессия {}… завершена", short(&id, 8)));
                self.detail = None;
                return self.load(ctx);
            }
            Msg::Killed(_, Err(_)) => ctx.toast(Tone::Danger, "Не удалось завершить сессию"),
            Msg::KillOthers => {
                let Some(current) = self.current.clone() else {
                    ctx.toast(Tone::Danger, "Не удалось определить текущую сессию");
                    return Task::none();
                };
                self.killing = true;
                let api = ctx.api.clone();
                return Task::perform(async move { api.kill_other_sessions(&current).await.map_err(text_error) }, Msg::KilledOthers);
            }
            Msg::KilledOthers(reply) => {
                self.killing = false;
                match reply {
                    Ok(answer) => {
                        ctx.toast(Tone::Success, format!("Завершено сессий: {}", answer.killed));
                        return self.load(ctx);
                    }
                    Err(_) => ctx.toast(Tone::Danger, "Ошибка завершения сессий"),
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, ctx: &Ctx) -> El<'a, Msg> {
        let others = self.others();
        let mut hint = "Все живые токены команды. Авто-обновление каждые 15с.".to_owned();
        if let Some(items) = &self.items {
            hint = format!("{hint} · всего {}, чужих {others}", items.len());
        }
        let mut actions = row![].spacing(space::SM);
        if others > 0 {
            let caption = if self.killing { "Завершаем".to_owned() } else { format!("Завершить все ({others})") };
            actions = actions.push(ui::btn(caption, Kind::Danger, (!self.killing).then_some(Msg::KillOthers)));
        }
        actions = actions.push(ui::btn_icon(Icon::Refresh, "Обновить", Kind::Secondary, Some(Msg::Refresh)));
        let body: El<'a, Msg> = match &self.items {
            None => ui::empty_state(Icon::Monitor, "Загружаем сессии…", "Опрашиваем активные токены команды."),
            Some(items) if items.is_empty() => ui::empty_state(Icon::Monitor, "Нет активных сессий", "Если вы вошли в систему — это странно. Попробуйте обновить."),
            Some(items) => grid::grid(Rows { items, current: self.current.as_deref(), dark: ctx.dark }, &self.columns, &self.selection, Msg::Grid).row_height(34.0).font_size(13.0).into(),
        };
        let table = container(body).width(Fill).height(Fill).style(theme::card);
        column![panel("Сессии команды", hint, actions, Space::new()), table].spacing(space::MD).height(Fill).into()
    }

    pub fn overlay<'a>(&'a self, _ctx: &Ctx) -> Vec<El<'a, Msg>> {
        let Some(id) = &self.detail else { return Vec::new() };
        let Some(item) = self.items.iter().flatten().find(|item| &item.session_id == id) else { return Vec::new() };
        let line = |caption: &'static str, value: String| -> El<'a, Msg> {
            row![container(ui::muted(caption)).width(130.0), text(value).size(size::BASE).font(theme::FONT_MONO).width(Fill)].spacing(space::SM).align_y(Alignment::Start).into()
        };
        let optional = |value: &Option<String>| value.clone().unwrap_or_else(|| "—".to_owned());
        let lines = Column::with_children(vec![
            line("Session ID", item.session_id.clone()),
            line("Тип", kind_label(&item.token_kind).to_owned()),
            line("Пользователь", optional(&item.subject)),
            line("User ID", item.user_id.map_or_else(|| "—".to_owned(), |id| id.to_string())),
            line("IP-адрес", optional(&item.ip_address)),
            line("Создана", datetime_str(item.created_at.as_deref())),
            line("Активность", datetime_str(item.last_activity_at.as_deref())),
            line("Истекает", datetime_str(item.expires_at.as_deref())),
        ])
        .spacing(space::SM);
        let mut actions = row![ui::fill(), ui::btn("Закрыть", Kind::Secondary, Some(Msg::CloseDetail))].spacing(space::SM);
        if self.current.as_deref() != Some(id.as_str()) {
            actions = actions.push(ui::btn("Завершить сессию", Kind::Danger, Some(Msg::Kill(id.clone()))));
        }
        let body = column![
            ui::dialog_title(format!("Сессия {}…", short(id, 8)), Some(Msg::CloseDetail)),
            ui::muted("Полная информация о сессии. Все поля доступны в одну строку без обрезания."),
            lines,
            actions,
        ]
        .spacing(space::MD);
        vec![ui::modal(Space::new(), body, 560.0, Some(Msg::CloseDetail))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> ActiveSession {
        ActiveSession { session_id: id.into(), token_kind: "app".into(), ..Default::default() }
    }

    #[test]
    fn token_kinds_read_as_words_and_unknown_ones_stay() {
        assert_eq!(kind_label("qr"), "QR-вход");
        assert_eq!(kind_label("mystery"), "mystery");
    }

    #[test]
    fn the_current_session_cannot_be_ended_from_its_row() {
        let items = [session("aaaaaaaaaaaaaaaa"), session("bbbbbbbbbbbbbbbb")];
        let rows = Rows { items: &items, current: Some("aaaaaaaaaaaaaaaa"), dark: false };
        assert_eq!(rows.cell(0, COL_ACTION), "текущая");
        assert_eq!(rows.cell(1, COL_ACTION), "Завершить");
        assert_eq!(rows.cell(0, 0), "aaaaaaaaaaaa…");
    }
}
