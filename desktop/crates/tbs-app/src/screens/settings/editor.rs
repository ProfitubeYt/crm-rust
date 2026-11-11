//! The account form: creating a user and changing one.

use iced::widget::{Column, Row, button, column, row};
use iced::{Alignment, Fill, Task};
use serde_json::{Value, json};
use tbs_core::api::{AccessScope, AdminUser, ApiError, NewUser};

use crate::ctx::Ctx;
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Icon, Kind};

use super::avatar;

pub const TAB_OPTIONS: [(&str, &str); 5] = [("dashboard", "Дашборд"), ("reference", "Справочник"), ("userbot", "Telegram Bots"), ("logs", "Логи"), ("admin", "Admin")];

const ROLES: [&str; 2] = ["operator", "admin"];
const DEFAULT_TTL: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Username,
    Password,
    DisplayName,
    Tabs,
    Ttl,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Username(String),
    Password(String),
    DisplayName(String),
    Role(String),
    Active(bool),
    Force(bool),
    Tab(&'static str),
    ToggleMore,
    Ttl(String),
    Sources(bool),
    TelegramId(String),
    TelegramName(String),
    Folder(i64),
    PeriodFrom(usize, String),
    PeriodTo(usize, String),
    AddPeriod,
    RemovePeriod(usize),
    Submit,
    Saved(Result<AdminUser, Option<String>>),
    Close,
}

/// What the editor tells the list when it is done.
pub enum Outcome {
    None,
    Closed,
    Saved(AdminUser, bool),
}

pub struct Editor {
    original: Option<AdminUser>,
    username: String,
    password: String,
    display_name: String,
    role: String,
    is_active: bool,
    force_2fa: bool,
    tabs: Vec<String>,
    more: bool,
    ttl: String,
    can_toggle_sources: bool,
    telegram_id: String,
    telegram_name: String,
    folders: Vec<i64>,
    periods: Vec<(String, String)>,
    errors: Vec<(Field, &'static str)>,
    failure: Option<String>,
    saving: bool,
}

/// At least 8 characters with a capital, a small letter, a digit and a symbol.
pub fn strong_password(value: &str) -> bool {
    value.chars().count() >= 8
        && value.chars().any(|c| c.is_ascii_lowercase())
        && value.chars().any(|c| c.is_ascii_uppercase())
        && value.chars().any(|c| c.is_ascii_digit())
        && value.chars().any(|c| !c.is_alphanumeric())
}

fn valid_username(value: &str) -> bool {
    (3..=50).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn period_pair(raw: &Value) -> (String, String) {
    let read = |key: &str| raw[key].as_str().unwrap_or_default().to_owned();
    (read("from"), read("to"))
}

impl Editor {
    pub fn create() -> Self {
        Self {
            original: None,
            username: String::new(),
            password: String::new(),
            display_name: String::new(),
            role: "operator".into(),
            is_active: true,
            force_2fa: false,
            tabs: vec!["dashboard".into()],
            more: false,
            ttl: DEFAULT_TTL.to_string(),
            can_toggle_sources: false,
            telegram_id: String::new(),
            telegram_name: String::new(),
            folders: Vec::new(),
            periods: Vec::new(),
            errors: Vec::new(),
            failure: None,
            saving: false,
        }
    }

    pub fn edit(user: &AdminUser) -> Self {
        Self {
            original: Some(user.clone()),
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            role: user.role.clone(),
            is_active: user.is_active,
            force_2fa: user.force_2fa,
            tabs: user.allowed_tabs.clone(),
            ttl: user.session_ttl_days.to_string(),
            can_toggle_sources: user.can_toggle_sources,
            folders: user.allowed_folders.clone(),
            periods: user.forbidden_periods.iter().map(period_pair).collect(),
            ..Self::create()
        }
    }

    fn is_new(&self) -> bool {
        self.original.is_none()
    }

    fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    fn set_role(&mut self, role: String) {
        self.role = role;
        if self.is_admin() {
            self.tabs = TAB_OPTIONS.iter().map(|(id, _)| (*id).to_owned()).collect();
        } else {
            self.tabs.retain(|tab| tab != "admin");
            if !self.tabs.iter().any(|tab| tab == "dashboard") {
                self.tabs.insert(0, "dashboard".into());
            }
        }
    }

    fn toggle_tab(&mut self, tab: &'static str) {
        if self.is_admin() {
            return;
        }
        match self.tabs.iter().position(|t| t == tab) {
            Some(at) => {
                self.tabs.remove(at);
            }
            None => self.tabs.push(tab.to_owned()),
        }
    }

    fn validate(&self) -> Vec<(Field, &'static str)> {
        let mut errors = Vec::new();
        if self.is_new() {
            let name = self.username.trim();
            if name.is_empty() {
                errors.push((Field::Username, "Логин обязателен"));
            } else if !valid_username(name) {
                errors.push((Field::Username, "Логин: 3-50 символов, только a-z A-Z 0-9 . _ -"));
            }
            if self.password.is_empty() {
                errors.push((Field::Password, "Пароль обязателен"));
            } else if !strong_password(&self.password) {
                errors.push((Field::Password, "Минимум 8 символов: A-Z, a-z, 0-9 и спецсимвол"));
            }
        }
        if self.display_name.trim().is_empty() {
            errors.push((Field::DisplayName, "Отображаемое имя обязательно"));
        }
        if self.tabs.is_empty() {
            errors.push((Field::Tabs, "Выберите хотя бы одну вкладку"));
        }
        if !self.ttl.trim().parse::<i64>().is_ok_and(|days| (1..=90).contains(&days)) {
            errors.push((Field::Ttl, "TTL сессии должен быть от 1 до 90 дней"));
        }
        errors
    }

    fn forbidden(&self) -> Vec<Value> {
        self.periods
            .iter()
            .filter(|(from, to)| !from.trim().is_empty() && !to.trim().is_empty())
            .map(|(from, to)| json!({ "from": from.trim(), "to": to.trim() }))
            .collect()
    }

    fn new_user(&self) -> NewUser {
        NewUser {
            username: self.username.trim().to_owned(),
            password: self.password.clone(),
            role: self.role.clone(),
            display_name: self.display_name.trim().to_owned(),
            is_active: self.is_active,
            allowed_tabs: self.tabs.clone(),
            allowed_folders: self.folders.clone(),
            forbidden_periods: self.forbidden(),
            allowed_sources: Vec::new(),
            can_toggle_sources: self.can_toggle_sources,
            force_2fa: self.force_2fa,
            session_ttl_days: self.ttl.trim().parse().unwrap_or(DEFAULT_TTL),
            telegram_id: self.telegram_id.trim().parse().ok(),
            telegram_username: Some(self.telegram_name.trim().trim_start_matches('@').to_owned()).filter(|name| !name.is_empty()),
        }
    }

    fn changes(&self) -> Value {
        json!({
            "display_name": self.display_name.trim(),
            "role": self.role,
            "is_active": self.is_active,
            "allowed_tabs": self.tabs,
            "allowed_folders": self.folders,
            "forbidden_periods": self.forbidden(),
            "allowed_sources": [],
            "can_toggle_sources": self.can_toggle_sources,
            "force_2fa": self.force_2fa,
            "session_ttl_days": self.ttl.trim().parse::<i64>().unwrap_or(DEFAULT_TTL),
        })
    }

    pub fn update(&mut self, message: Msg, ctx: &Ctx) -> (Task<Msg>, Outcome) {
        match message {
            Msg::Username(value) => self.username = value,
            Msg::Password(value) => self.password = value,
            Msg::DisplayName(value) => self.display_name = value,
            Msg::Role(value) => self.set_role(value),
            Msg::Active(value) => self.is_active = value,
            Msg::Force(value) => self.force_2fa = value,
            Msg::Tab(tab) => self.toggle_tab(tab),
            Msg::ToggleMore => self.more = !self.more,
            Msg::Ttl(value) => self.ttl = value.chars().filter(char::is_ascii_digit).collect(),
            Msg::Sources(value) => self.can_toggle_sources = value,
            Msg::TelegramId(value) => self.telegram_id = value.chars().filter(char::is_ascii_digit).collect(),
            Msg::TelegramName(value) => self.telegram_name = value,
            Msg::Folder(id) => match self.folders.iter().position(|folder| *folder == id) {
                Some(at) => {
                    self.folders.remove(at);
                }
                None => self.folders.push(id),
            },
            Msg::PeriodFrom(at, value) => {
                if let Some(period) = self.periods.get_mut(at) {
                    period.0 = value;
                }
            }
            Msg::PeriodTo(at, value) => {
                if let Some(period) = self.periods.get_mut(at) {
                    period.1 = value;
                }
            }
            Msg::AddPeriod => self.periods.push(Default::default()),
            Msg::RemovePeriod(at) => {
                if at < self.periods.len() {
                    self.periods.remove(at);
                }
            }
            Msg::Submit => {
                if self.saving {
                    return (Task::none(), Outcome::None);
                }
                self.errors = self.validate();
                self.failure = None;
                if self.errors.iter().any(|(field, _)| *field == Field::Ttl) {
                    self.more = true;
                }
                if !self.errors.is_empty() {
                    return (Task::none(), Outcome::None);
                }
                self.saving = true;
                let api = ctx.api.clone();
                let task = match &self.original {
                    None => {
                        let user = self.new_user();
                        Task::perform(async move { api.create_user(&user).await.map_err(|error| explain(&error)) }, Msg::Saved)
                    }
                    Some(user) => {
                        let (id, changes) = (user.id, self.changes());
                        Task::perform(async move { api.update_user(id, &changes).await.map_err(|error| explain(&error)) }, Msg::Saved)
                    }
                };
                return (task, Outcome::None);
            }
            Msg::Saved(Ok(user)) => {
                self.saving = false;
                return (Task::none(), Outcome::Saved(user, self.is_new()));
            }
            Msg::Saved(Err(reason)) => {
                self.saving = false;
                self.failure = Some(reason.unwrap_or_else(|| "Ошибка сохранения".to_owned()));
            }
            Msg::Close => return (Task::none(), Outcome::Closed),
        }
        (Task::none(), Outcome::None)
    }

    fn error_of(&self, field: Field) -> Option<&'static str> {
        self.errors.iter().find(|(f, _)| *f == field).map(|(_, message)| *message)
    }

    fn with_error<'a>(&self, control: impl Into<El<'a, Msg>>, field: Field) -> El<'a, Msg> {
        match self.error_of(field) {
            Some(message) => column![control.into(), ui::error_line(message)].spacing(space::XS).into(),
            None => control.into(),
        }
    }

    fn text_field<'a>(&'a self, caption: &'a str, placeholder: &str, value: &str, field: Field, on_input: fn(String) -> Msg, secret: bool) -> El<'a, Msg> {
        let mut control = if secret { ui::secret_input(placeholder, value) } else { ui::input(placeholder, value) }.on_input(on_input).on_submit(Msg::Submit);
        if self.error_of(field).is_some() {
            control = control.style(theme::input_error);
        }
        self.with_error(ui::field(caption, control), field)
    }

    fn identity(&self, user: &AdminUser) -> El<'_, Msg> {
        let tone = if user.is_admin() { Tone::Accent } else { Tone::Neutral };
        let status = if user.is_active { ui::badge("активен", Tone::Success) } else { ui::badge("отключён", Tone::Danger) };
        let name = if user.display_name.trim().is_empty() { "Без имени".to_owned() } else { user.display_name.clone() };
        let badges = row![ui::badge(user.role.clone(), tone), status].spacing(space::XS);
        row![avatar(user.title(), 40.0), column![ui::strong(name), badges].spacing(space::XS)].spacing(space::MD).align_y(Alignment::Center).into()
    }

    fn tabs_view(&self) -> El<'_, Msg> {
        let admin = self.is_admin();
        let mut chips = Row::new().spacing(space::XS);
        for (id, caption) in TAB_OPTIONS {
            let active = self.tabs.iter().any(|tab| tab == id);
            let visible = admin || id != "admin";
            if visible {
                chips = chips.push(ui::chip(caption, active, Msg::Tab(id)));
            }
        }
        let mut block = column![chips.wrap()].spacing(space::XS);
        if admin {
            block = block.push(ui::faint("admin — полный доступ"));
        }
        self.with_error(block, Field::Tabs)
    }

    fn more_view<'a>(&'a self, scopes: &'a [AccessScope]) -> El<'a, Msg> {
        let glyph = if self.more { Icon::ChevronUp } else { Icon::ChevronDown };
        let head = button(row![ui::icon(glyph, 14.0), ui::label("Дополнительные настройки")].spacing(space::XS).align_y(Alignment::Center)).padding(0).style(theme::btn_link).on_press(Msg::ToggleMore);
        if !self.more {
            return head.into();
        }
        let ttl = self.text_field("TTL сессии (дней)", "7", &self.ttl, Field::Ttl, Msg::Ttl, false);
        let mut block = column![head, ttl, row![ui::switch(self.can_toggle_sources, Msg::Sources), ui::label("Переключение источников")].spacing(space::SM).align_y(Alignment::Center)].spacing(space::MD);
        if self.is_new() {
            block = block.push(
                row![
                    ui::field("Telegram ID", ui::input("123456789", &self.telegram_id).on_input(Msg::TelegramId)).width(Fill),
                    ui::field("@username", ui::input("username", &self.telegram_name).on_input(Msg::TelegramName)).width(Fill),
                ]
                .spacing(space::MD),
            );
        }
        if !scopes.is_empty() {
            let mut chips = Row::new().spacing(space::XS);
            for scope in scopes {
                chips = chips.push(ui::chip(scope.name.as_str(), self.folders.contains(&scope.id), Msg::Folder(scope.id)));
            }
            block = block.push(ui::section("Разрешённые папки", chips.wrap()));
        }
        let mut rows = Column::new().spacing(space::XS);
        for (at, (from, to)) in self.periods.iter().enumerate() {
            rows = rows.push(
                row![
                    ui::input("from (YYYY-MM-DD)", from).on_input(move |value| Msg::PeriodFrom(at, value)),
                    ui::input("to (YYYY-MM-DD)", to).on_input(move |value| Msg::PeriodTo(at, value)),
                    ui::btn("Удалить", Kind::Ghost, Some(Msg::RemovePeriod(at))),
                ]
                .spacing(space::SM)
                .align_y(Alignment::Center),
            );
        }
        rows = rows.push(ui::btn_icon(Icon::Plus, "Добавить период", Kind::Secondary, Some(Msg::AddPeriod)));
        block.push(ui::section("Запрещённые периоды", rows)).into()
    }

    pub fn view<'a>(&'a self, scopes: &'a [AccessScope]) -> El<'a, Msg> {
        let mut body = Column::new().spacing(space::MD);
        match &self.original {
            Some(user) => body = body.push(self.identity(user)),
            None => {
                body = body
                    .push(self.text_field("Логин", "andrey_b", &self.username, Field::Username, Msg::Username, false))
                    .push(self.text_field("Пароль", "••••••••", &self.password, Field::Password, Msg::Password, true))
                    .push(ui::faint("8+ символов, A-Z, a-z, 0-9, спецсимвол"));
            }
        }
        let roles: Vec<String> = ROLES.iter().map(|role| (*role).to_owned()).collect();
        body = body
            .push(self.text_field("Отображаемое имя", "Андрей Бачинин", &self.display_name, Field::DisplayName, Msg::DisplayName, false))
            .push(ui::field("Роль", ui::select(roles, Some(self.role.clone()), Msg::Role).width(Fill)))
            .push(
                row![
                    row![ui::switch(self.is_active, Msg::Active), ui::label("Активен")].spacing(space::SM).align_y(Alignment::Center),
                    row![ui::switch(self.force_2fa, Msg::Force), ui::label("Обязательная 2FA")].spacing(space::SM).align_y(Alignment::Center),
                ]
                .spacing(space::XL),
            )
            .push(ui::section("Разрешённые вкладки", self.tabs_view()))
            .push(self.more_view(scopes));
        if !self.is_new() {
            body = body.push(ui::faint("Пароль и логин нельзя редактировать. Используйте действия в строке таблицы."));
        }
        if let Some(failure) = &self.failure {
            body = body.push(ui::error_line(failure.as_str()));
        }
        let (name, hint) = if self.is_new() { ("Новый пользователь", "Логин и пароль фиксируются при создании.".to_owned()) } else { ("Редактирование", format!("#{} · {}", self.original.as_ref().map_or(0, |u| u.id), self.username)) };
        let action = match (self.is_new(), self.saving) {
            (_, true) => "Сохраняем",
            (true, false) => "Создать",
            (false, false) => "Сохранить",
        };
        let footer = row![ui::fill(), ui::btn("Отмена", Kind::Secondary, Some(Msg::Close)), ui::btn(action, Kind::Primary, (!self.saving).then_some(Msg::Submit))].spacing(space::SM);
        let panel = ui::sheet_panel("Пользователь", name, hint, Msg::Close, body, Some(footer.into()));
        ui::sheet(iced::widget::Space::new(), panel, 560.0, Some(Msg::Close))
    }
}

/// The server's refusal in words, or `None` when the caller's fallback fits.
pub fn explain(error: &ApiError) -> Option<String> {
    let ApiError::Http { code, message, .. } = error else { return None };
    let text = match code.as_deref().unwrap_or(message.as_str()) {
        "weak_password" => "Слабый пароль: мин. 8 символов, заглавная + строчная + цифра + спецсимвол",
        "username_exists" => "Пользователь с таким логином уже существует",
        "invalid_username" => "Некорректный логин",
        "invalid_password" => "Пароль не задан",
        "invalid_role" => "Некорректная роль",
        "telegram_id_already_linked" => "Этот Telegram ID уже привязан к другому пользователю",
        _ => return None,
    };
    Some(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> Editor {
        Editor { username: "anna_k".into(), password: "Str0ng!pass".into(), display_name: "Анна".into(), ..Editor::create() }
    }

    #[test]
    fn passwords_need_all_four_kinds_of_character() {
        assert!(strong_password("Str0ng!pass"));
        assert!(!strong_password("Sh0rt!"));
        assert!(!strong_password("nouppercase1!"));
        assert!(!strong_password("NoDigits!!"));
        assert!(!strong_password("NoSymbol12"));
    }

    #[test]
    fn usernames_are_limited_to_a_safe_alphabet() {
        assert!(valid_username("andrey_b.1-x"));
        assert!(!valid_username("ab"));
        assert!(!valid_username("имя"));
        assert!(!valid_username("with space"));
    }

    #[test]
    fn a_filled_form_passes() {
        assert!(filled().validate().is_empty());
    }

    #[test]
    fn every_empty_field_is_reported() {
        let form = Editor { ttl: "0".into(), tabs: Vec::new(), ..Editor::create() };
        let fields: Vec<Field> = form.validate().into_iter().map(|(field, _)| field).collect();
        assert_eq!(fields, vec![Field::Username, Field::Password, Field::DisplayName, Field::Tabs, Field::Ttl]);
    }

    #[test]
    fn editing_does_not_ask_for_login_or_password() {
        let user = AdminUser { id: 1, username: "a".into(), display_name: "A".into(), role: "operator".into(), allowed_tabs: vec!["logs".into()], session_ttl_days: 7, ..Default::default() };
        assert!(Editor::edit(&user).validate().is_empty());
    }

    #[test]
    fn an_admin_gets_every_tab_and_an_operator_keeps_the_table() {
        let mut form = filled();
        form.set_role("admin".into());
        assert_eq!(form.tabs.len(), TAB_OPTIONS.len());
        form.toggle_tab("logs");
        assert_eq!(form.tabs.len(), TAB_OPTIONS.len());
        form.tabs = vec!["admin".into()];
        form.set_role("operator".into());
        assert_eq!(form.tabs, vec!["dashboard".to_owned()]);
    }

    #[test]
    fn half_filled_period_rows_are_dropped() {
        let form = Editor { periods: vec![("2026-01-01".into(), "2026-02-01".into()), ("2026-03-01".into(), String::new())], ..filled() };
        assert_eq!(form.forbidden(), vec![json!({ "from": "2026-01-01", "to": "2026-02-01" })]);
    }

    #[test]
    fn a_telegram_handle_loses_its_at_sign() {
        let form = Editor { telegram_id: "42".into(), telegram_name: "@anna".into(), ..filled() };
        let user = form.new_user();
        assert_eq!(user.telegram_id, Some(42));
        assert_eq!(user.telegram_username.as_deref(), Some("anna"));
    }

    #[test]
    fn server_refusals_read_as_sentences() {
        let error = ApiError::Http { status: 409, code: None, message: "username_exists".into() };
        assert_eq!(explain(&error).as_deref(), Some("Пользователь с таким логином уже существует"));
        let other = ApiError::Http { status: 500, code: None, message: "boom".into() };
        assert!(explain(&other).is_none());
    }
}
