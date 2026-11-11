//! The team list, the row menu and the small prompts its actions need.

use std::borrow::Cow;

use iced::widget::{Column, Id, Space, column, container, row};
use iced::{Alignment, Fill, Length, Point, Size, Task};
use tbs_core::api::{AccessScope, AdminUser, ApiError};

use crate::ctx::{Ctx, text_error};
use crate::grid::{self, CellStyle, GridColumn, GridData, GridEvent, Selection};
use crate::theme::{self, DARK, LIGHT, Tone, space};
use crate::ui::{self, El, Icon, Kind};

use super::editor::{self, Editor, Outcome, strong_password};

const MENU_WIDTH: f32 = 236.0;
const MENU_ROW: f32 = 34.0;
const COL_ACTIONS: usize = 6;
const WEAK_PASSWORD: &str = "Минимум 8 символов: A-Z, a-z, 0-9, спецсимвол";

const SEARCH_ID: &str = "users-search";

pub fn focus_search<M: Send + 'static>() -> Task<M> {
    iced::widget::operation::focus(Id::new(SEARCH_ID))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoleFilter {
    #[default]
    All,
    Admins,
    Operators,
}

impl RoleFilter {
    const ALL: [RoleFilter; 3] = [RoleFilter::All, RoleFilter::Admins, RoleFilter::Operators];

    fn accepts(self, user: &AdminUser) -> bool {
        match self {
            RoleFilter::All => true,
            RoleFilter::Admins => user.role == "admin",
            RoleFilter::Operators => user.role == "operator",
        }
    }
}

impl std::fmt::Display for RoleFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RoleFilter::All => "Все роли",
            RoleFilter::Admins => "Только admin",
            RoleFilter::Operators => "Только operator",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    ResetPassword,
    ResetTwoFactor,
    ToggleForce,
    Unlock,
    Telegram,
    Status,
}

#[derive(Clone, Debug)]
enum Prompt {
    Password { user: i64, name: String, value: String, error: Option<&'static str> },
    TelegramId { user: i64, name: String, value: String, error: Option<&'static str> },
    TelegramName { user: i64, name: String, telegram_id: i64, value: String },
    Deactivate { user: i64, name: String },
}

/// Says how a finished row action reads to the person who ran it.
#[derive(Clone, Copy, Debug)]
pub struct Wording {
    done: &'static str,
    failed: &'static str,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Loaded(Result<Vec<AdminUser>, String>),
    Scopes(Result<Vec<AccessScope>, String>),
    Search(String),
    Role(RoleFilter),
    Grid(GridEvent),
    CloseMenu,
    Row(RowAction),
    OpenAdd,
    Editor(editor::Msg),
    PromptInput(String),
    PromptSubmit,
    PromptClose,
    Done(Wording, Result<AdminUser, Option<String>>),
}

pub struct Users {
    items: Vec<AdminUser>,
    scopes: Vec<AccessScope>,
    loaded: bool,
    search: String,
    role: RoleFilter,
    selection: Selection,
    columns: Vec<GridColumn>,
    menu: Option<(i64, Point)>,
    prompt: Option<Prompt>,
    editor: Option<Editor>,
}

struct Rows<'a> {
    items: Vec<&'a AdminUser>,
    dark: bool,
}

fn two_factor_label(user: &AdminUser) -> &'static str {
    match (user.totp_enabled, user.force_2fa) {
        (true, _) => "включена",
        (false, true) => "принудит.",
        _ => "—",
    }
}

fn telegram_label(user: &AdminUser) -> String {
    match (user.telegram_id, user.telegram_username.as_deref()) {
        (Some(id), Some(name)) if !name.is_empty() => format!("{id} · @{name}"),
        (Some(id), _) => id.to_string(),
        _ => "—".to_owned(),
    }
}

impl GridData for Rows<'_> {
    fn row_count(&self) -> usize {
        self.items.len()
    }

    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let user = self.items[row];
        match col {
            0 => format!("{} · @{}", user.title(), user.username).into(),
            1 => user.role.as_str().into(),
            2 if user.is_admin() => format!("все ({})", editor::TAB_OPTIONS.len()).into(),
            2 => user.allowed_tabs.len().to_string().into(),
            3 => two_factor_label(user).into(),
            4 => telegram_label(user).into(),
            5 if user.is_active => "активен".into(),
            5 => "отключён".into(),
            _ => "···".into(),
        }
    }

    fn cell_style(&self, row: usize, col: usize) -> CellStyle {
        let c = if self.dark { &DARK } else { &LIGHT };
        let user = self.items[row];
        let color = match col {
            1 if user.is_admin() => Some(c.accent),
            3 if user.totp_enabled => Some(c.success),
            3 if user.force_2fa => Some(c.warning),
            5 if user.is_active => Some(c.success),
            5 => Some(c.danger),
            6 => Some(c.text2),
            _ => None,
        };
        CellStyle { color, ..CellStyle::default() }
    }
}

fn matches(user: &AdminUser, needle: &str) -> bool {
    needle.is_empty()
        || user.username.to_lowercase().contains(needle)
        || user.display_name.to_lowercase().contains(needle)
        || user.telegram_username.as_deref().is_some_and(|name| name.to_lowercase().contains(needle))
}

fn parse_telegram_id(raw: &str) -> Option<i64> {
    raw.trim().parse::<i64>().ok().filter(|id| *id > 0)
}

impl Users {
    pub fn new(_ctx: &Ctx) -> Self {
        let columns = vec![
            GridColumn::new("Пользователь", 230.0).medium(),
            GridColumn::new("Роль", 100.0),
            GridColumn::new("Вкладки", 90.0),
            GridColumn::new("2FA", 100.0),
            GridColumn::new("Telegram", 130.0),
            GridColumn::new("Статус", 100.0),
            GridColumn::new("", 48.0).align(grid::Align::Center),
        ];
        Self { items: Vec::new(), scopes: Vec::new(), loaded: false, search: String::new(), role: RoleFilter::All, selection: Selection::default(), columns, menu: None, prompt: None, editor: None }
    }

    pub fn count(&self) -> Option<usize> {
        self.loaded.then_some(self.items.len())
    }

    pub fn escape(&mut self) {
        if self.menu.take().is_none() && self.prompt.take().is_none() {
            self.editor = None;
        }
    }

    pub fn load(&self, ctx: &Ctx) -> Task<Msg> {
        let (api, scopes) = (ctx.api.clone(), ctx.api.clone());
        Task::batch([
            Task::perform(async move { api.admin_users().await.map_err(text_error) }, Msg::Loaded),
            Task::perform(async move { scopes.access_scopes().await.map_err(text_error) }, Msg::Scopes),
        ])
    }

    fn visible(&self) -> Vec<&AdminUser> {
        let needle = self.search.trim().to_lowercase();
        self.items.iter().filter(|user| self.role.accepts(user) && matches(user, &needle)).collect()
    }

    fn user(&self, id: i64) -> Option<&AdminUser> {
        self.items.iter().find(|user| user.id == id)
    }

    fn run(&self, wording: Wording, call: impl Future<Output = Result<AdminUser, ApiError>> + Send + 'static) -> Task<Msg> {
        Task::perform(async move { call.await.map_err(|error| editor::explain(&error)) }, move |reply| Msg::Done(wording, reply))
    }

    fn row_action(&mut self, action: RowAction, ctx: &Ctx) -> Task<Msg> {
        let Some((id, _)) = self.menu.take() else { return Task::none() };
        let Some(user) = self.user(id).cloned() else { return Task::none() };
        let api = ctx.api.clone();
        let name = user.username.clone();
        match action {
            RowAction::ResetPassword => self.prompt = Some(Prompt::Password { user: id, name, value: String::new(), error: None }),
            RowAction::ResetTwoFactor => {
                return self.run(Wording { done: "2FA сброшена", failed: "Не удалось сбросить 2FA" }, async move { api.reset_two_factor(id).await });
            }
            RowAction::ToggleForce => {
                let force = !user.force_2fa;
                return self.run(Wording { done: "Флаг force_2fa обновлён", failed: "Не удалось обновить" }, async move { api.force_two_factor(id, force).await });
            }
            RowAction::Unlock => {
                return self.run(Wording { done: "Пользователь разблокирован", failed: "Ошибка разблокировки" }, async move { api.unlock_user(id).await });
            }
            RowAction::Telegram if user.telegram_id.is_some() => {
                return self.run(Wording { done: "Telegram отвязан", failed: "Ошибка отвязки" }, async move { api.unlink_telegram(id).await });
            }
            RowAction::Telegram => self.prompt = Some(Prompt::TelegramId { user: id, name, value: String::new(), error: None }),
            RowAction::Status if user.is_active => self.prompt = Some(Prompt::Deactivate { user: id, name }),
            RowAction::Status => {
                let changes = serde_json::json!({ "is_active": true });
                return self.run(Wording { done: "Пользователь активирован", failed: "Ошибка" }, async move { api.update_user(id, &changes).await });
            }
        }
        Task::none()
    }

    fn submit_prompt(&mut self, ctx: &Ctx) -> Task<Msg> {
        let api = ctx.api.clone();
        match self.prompt.take() {
            Some(Prompt::Password { user, name, value, .. }) => {
                let error = if value.is_empty() {
                    Some("Пароль обязателен")
                } else if !strong_password(&value) {
                    Some(WEAK_PASSWORD)
                } else {
                    None
                };
                if error.is_some() {
                    self.prompt = Some(Prompt::Password { user, name, value, error });
                    return Task::none();
                }
                self.run(Wording { done: "Пароль сброшен", failed: "Не удалось сбросить пароль" }, async move { api.reset_password(user, &value).await })
            }
            Some(Prompt::TelegramId { user, name, value, .. }) => {
                match parse_telegram_id(&value) {
                    Some(telegram_id) => self.prompt = Some(Prompt::TelegramName { user, name, telegram_id, value: String::new() }),
                    None => self.prompt = Some(Prompt::TelegramId { user, name, value, error: Some("Telegram ID должен быть положительным числом") }),
                }
                Task::none()
            }
            Some(Prompt::TelegramName { user, telegram_id, value, .. }) => {
                let handle = value.trim().trim_start_matches('@').to_owned();
                self.run(Wording { done: "Telegram привязан", failed: "Ошибка привязки Telegram" }, async move {
                    api.link_telegram(user, telegram_id, Some(handle.as_str()).filter(|name| !name.is_empty())).await
                })
            }
            Some(Prompt::Deactivate { user, .. }) => self.run(Wording { done: "Пользователь деактивирован", failed: "Ошибка" }, async move { api.deactivate_user(user).await }),
            None => Task::none(),
        }
    }

    fn put(&mut self, user: AdminUser, ctx: &mut Ctx) {
        if ctx.user.as_ref().is_some_and(|me| me.id == user.id) {
            refresh_self(&user, ctx);
        }
        match self.items.iter_mut().find(|item| item.id == user.id) {
            Some(item) => *item = user,
            None => self.items.push(user),
        }
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Loaded(Ok(items)) => {
                self.items = items;
                self.loaded = true;
            }
            Msg::Loaded(Err(reason)) => {
                self.loaded = true;
                ctx.toast(Tone::Danger, reason);
            }
            Msg::Scopes(Ok(scopes)) => self.scopes = scopes,
            Msg::Scopes(Err(reason)) => tracing::warn!("folders not loaded: {reason}"),
            Msg::Search(value) => {
                self.search = value;
                self.selection = Selection::default();
            }
            Msg::Role(role) => {
                self.role = role;
                self.selection = Selection::default();
            }
            Msg::Grid(event) => match event {
                GridEvent::Select(selection) => self.selection = selection,
                GridEvent::Click { row, col, at } if col == COL_ACTIONS => self.menu = self.visible().get(row).map(|user| (user.id, at)),
                GridEvent::Click { row, .. } | GridEvent::Activate { row, .. } => self.editor = self.visible().get(row).map(|user| Editor::edit(user)),
                GridEvent::Context { row, at, .. } => self.menu = self.visible().get(row).map(|user| (user.id, at)),
                GridEvent::Resize { col, width } => {
                    if let Some(column) = self.columns.get_mut(col) {
                        column.width = width;
                    }
                }
                _ => {}
            },
            Msg::CloseMenu => self.menu = None,
            Msg::Row(action) => return self.row_action(action, ctx),
            Msg::OpenAdd => self.editor = Some(Editor::create()),
            Msg::Editor(inner) => {
                let Some(form) = &mut self.editor else { return Task::none() };
                let (task, outcome) = form.update(inner, ctx);
                match outcome {
                    Outcome::None => {}
                    Outcome::Closed => self.editor = None,
                    Outcome::Saved(user, created) => {
                        self.editor = None;
                        ctx.toast(Tone::Success, if created { "Пользователь создан" } else { "Пользователь обновлён" });
                        self.put(user, ctx);
                    }
                }
                return task.map(Msg::Editor);
            }
            Msg::PromptInput(input) => match &mut self.prompt {
                Some(Prompt::Password { value, error, .. } | Prompt::TelegramId { value, error, .. }) => {
                    *value = input;
                    *error = None;
                }
                Some(Prompt::TelegramName { value, .. }) => *value = input,
                _ => {}
            },
            Msg::PromptSubmit => return self.submit_prompt(ctx),
            Msg::PromptClose => self.prompt = None,
            Msg::Done(wording, Ok(user)) => {
                ctx.toast(Tone::Success, wording.done);
                self.put(user, ctx);
            }
            Msg::Done(wording, Err(reason)) => ctx.toast(Tone::Danger, reason.unwrap_or_else(|| wording.failed.to_owned())),
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, ctx: &Ctx) -> El<'a, Msg> {
        let shown = self.visible();
        let tools = row![
            ui::input("Поиск по логину или имени", &self.search).id(Id::new(SEARCH_ID)).on_input(Msg::Search).width(Length::Fixed(300.0)),
            ui::select(RoleFilter::ALL.to_vec(), Some(self.role), Msg::Role).width(Length::Fixed(170.0)),
            ui::fill(),
            ui::faint(format!("{} из {}", shown.len(), self.items.len())),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);
        let body: El<'a, Msg> = if !self.loaded {
            ui::loading("Загружаем пользователей…")
        } else if self.items.is_empty() {
            ui::empty_state(Icon::Users, "Пока нет пользователей", "Создайте первую учётную запись через кнопку «Добавить».")
        } else if shown.is_empty() {
            ui::empty_state(Icon::Search, "Никого не найдено", "Попробуйте изменить запрос или фильтр.")
        } else {
            grid::grid(Rows { items: shown, dark: ctx.dark }, &self.columns, &self.selection, Msg::Grid).row_height(36.0).font_size(13.0).into()
        };
        let table = container(body).width(Fill).height(Fill).style(theme::card);
        column![tools, ui::faint("Клик по строке открывает карточку, правый клик или «···» — действия."), table].spacing(space::SM).height(Fill).into()
    }

    fn menu_view(&self, id: i64, at: Point, ctx: &Ctx) -> Option<El<'_, Msg>> {
        let user = self.user(id)?;
        let status = if user.is_active {
            ui::menu_item(Icon::Power, "Деактивировать", true, Msg::Row(RowAction::Status))
        } else {
            ui::menu_item(Icon::Check, "Активировать", false, Msg::Row(RowAction::Status))
        };
        let items: Vec<El<'_, Msg>> = vec![
            ui::menu_item(Icon::Key, "Сбросить пароль", false, Msg::Row(RowAction::ResetPassword)),
            ui::menu_item(Icon::Shield, "Сбросить 2FA", false, Msg::Row(RowAction::ResetTwoFactor)),
            ui::menu_item(Icon::Lock, if user.force_2fa { "Снять обязательную 2FA" } else { "Сделать 2FA обязательной" }, false, Msg::Row(RowAction::ToggleForce)),
            ui::menu_item(Icon::Unlock, "Разблокировать", false, Msg::Row(RowAction::Unlock)),
            ui::divider(),
            ui::menu_item(Icon::Send, if user.telegram_id.is_some() { "Отвязать Telegram" } else { "Привязать Telegram" }, false, Msg::Row(RowAction::Telegram)),
            ui::divider(),
            status,
        ];
        let panel_size = Size::new(MENU_WIDTH, items.len() as f32 * MENU_ROW);
        Some(ui::floating(ui::menu_panel(Column::with_children(items), MENU_WIDTH), at, panel_size, ctx.window, Msg::CloseMenu))
    }

    fn prompt_view<'a>(&'a self, prompt: &'a Prompt) -> El<'a, Msg> {
        if let Prompt::Deactivate { name, .. } = prompt {
            let body = format!("Учётная запись {name} перестанет работать, все её сессии завершатся.");
            return ui::confirm(iced::widget::Space::new(), "Деактивировать пользователя?", body, "Деактивировать", Msg::PromptSubmit, Msg::PromptClose);
        }
        let (name, hint, caption, placeholder, value, action, error, secret) = match prompt {
            Prompt::Password { name, value, error, .. } => (format!("Сбросить пароль: {name}"), "Новый пароль будет передан пользователю вне приложения.", "Новый пароль", "", value, "Сбросить", *error, true),
            Prompt::TelegramId { name, value, error, .. } => (format!("Привязать Telegram: {name}"), "Шаг 1 из 2 — Telegram ID пользователя.", "Telegram ID", "123456789", value, "Далее", *error, false),
            Prompt::TelegramName { name, value, .. } => (format!("Привязать Telegram: {name}"), "Шаг 2 из 2 — @username (необязательно, можно оставить пустым).", "Telegram @username", "", value, "Привязать", None, false),
            Prompt::Deactivate { .. } => unreachable!("handled above"),
        };
        let control = if secret { ui::secret_input(placeholder, value) } else { ui::input(placeholder, value) }.on_input(Msg::PromptInput).on_submit(Msg::PromptSubmit);
        let mut body = column![ui::dialog_title(name, Some(Msg::PromptClose)), ui::muted(hint), ui::field(caption, control)].spacing(space::MD);
        if let Some(error) = error {
            body = body.push(ui::error_line(error));
        }
        let actions = row![ui::fill(), ui::btn("Отмена", Kind::Secondary, Some(Msg::PromptClose)), ui::btn(action, Kind::Primary, Some(Msg::PromptSubmit))].spacing(space::SM);
        ui::modal(Space::new(), body.push(actions), 420.0, Some(Msg::PromptClose))
    }

    pub fn overlay<'a>(&'a self, ctx: &Ctx) -> Vec<El<'a, Msg>> {
        let mut layers = Vec::new();
        if let Some(form) = &self.editor {
            layers.push(form.view(&self.scopes).map(Msg::Editor));
        }
        if let Some((id, at)) = self.menu {
            layers.extend(self.menu_view(id, at, ctx));
        }
        if let Some(prompt) = &self.prompt {
            layers.push(self.prompt_view(prompt));
        }
        layers
    }
}

/// Someone changed their own account: the rail must follow right away.
fn refresh_self(user: &AdminUser, ctx: &mut Ctx) {
    if let Some(me) = &mut ctx.user {
        me.display_name = Some(user.display_name.clone()).filter(|name| !name.is_empty());
        me.role = user.role.clone();
        me.allowed_tabs = user.allowed_tabs.clone();
        me.force_2fa = user.force_2fa;
        me.totp_enabled = user.totp_enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: i64, name: &str, role: &str) -> AdminUser {
        AdminUser { id, username: name.into(), display_name: format!("{name} display"), role: role.into(), is_active: true, ..Default::default() }
    }

    #[test]
    fn search_looks_at_login_name_and_telegram() {
        let mut user = person(1, "anna", "operator");
        user.telegram_username = Some("Tele_Anna".into());
        assert!(matches(&user, "ann"));
        assert!(matches(&user, "display"));
        assert!(matches(&user, "tele_"));
        assert!(!matches(&user, "boris"));
    }

    #[test]
    fn the_role_filter_splits_the_team() {
        let admin = person(1, "a", "admin");
        let operator = person(2, "b", "operator");
        assert!(RoleFilter::All.accepts(&admin) && RoleFilter::All.accepts(&operator));
        assert!(RoleFilter::Admins.accepts(&admin) && !RoleFilter::Admins.accepts(&operator));
        assert!(RoleFilter::Operators.accepts(&operator) && !RoleFilter::Operators.accepts(&admin));
    }

    #[test]
    fn telegram_ids_must_be_positive_numbers() {
        assert_eq!(parse_telegram_id(" 123 "), Some(123));
        assert_eq!(parse_telegram_id("-5"), None);
        assert_eq!(parse_telegram_id("abc"), None);
    }

    #[test]
    fn the_two_factor_column_says_what_matters() {
        let mut user = person(1, "a", "operator");
        assert_eq!(two_factor_label(&user), "—");
        user.force_2fa = true;
        assert_eq!(two_factor_label(&user), "принудит.");
        user.totp_enabled = true;
        assert_eq!(two_factor_label(&user), "включена");
    }

    #[test]
    fn the_telegram_column_shows_what_is_known() {
        let mut user = person(1, "a", "operator");
        assert_eq!(telegram_label(&user), "—");
        user.telegram_id = Some(7);
        assert_eq!(telegram_label(&user), "7");
        user.telegram_username = Some("x".into());
        assert_eq!(telegram_label(&user), "7 · @x");
    }
}
