//! The sign-in window: the brand and the workspace to open on the left, the
//! account on the right. The right half also carries the two-factor steps, the
//! Telegram QR and the few settings that have to work before anyone has signed in.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Column, Id, Space, button, column, container, mouse_area, opaque, operation, qr_code, row, stack, svg, text, text_input};
use iced::{Alignment, Color, Event, Fill, Padding, Size, Subscription, Task, Theme, event, window};
use tbs_core::api::{Api, ApiError, LoginResponse, QrSession, QrStatus, TwoFactorConfirm, TwoFactorSetup};

use crate::ctx::{Ctx, Request, text_error};
use crate::screens::settings::ThemeChoice;
use crate::theme::{self, Tone, control, size, space};
use crate::ui::{self, El, Icon, Kind};

/// The sign-in has a small window of its own, and the two panes fill all of it.
pub const WINDOW: Size = Size::new(880.0, 560.0);
const PANE_WIDTH: f32 = 340.0;
const FORM_WIDTH: f32 = 360.0;
/// Six users; past that the list scrolls. It stays over the rest of the form.
const LIST_HEIGHT: f32 = 212.0;
const QR_SIZE: u32 = 192;
/// Sits beside the keys, so the confirm button stays in the window.
const SETUP_QR_SIZE: u32 = 144;
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The logo is 2.34 times as wide as it is high.
const LOGO_HEIGHT: f32 = 50.0;
const LOGO_WIDTH: f32 = LOGO_HEIGHT * 2.34;
/// Telegram's own blue: the one mark in this window that keeps its colour.
const TELEGRAM_BLUE: Color = iced::color!(0x229ED9);

static LOGO: LazyLock<svg::Handle> = LazyLock::new(|| svg::Handle::from_memory(include_bytes!("../../assets/logo.svg").as_slice()));

const USERNAME_ID: &str = "login-username";
const PASSWORD_ID: &str = "login-password";
const CODE_ID: &str = "login-code";

/// What this client can open. The accounts are shared: one sign-in on one
/// server, whichever subsystem it leads to. A subsystem is listed once it
/// works, never as a placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subsystem {
    Receipts,
}

impl Subsystem {
    const ALL: [Subsystem; 1] = [Subsystem::Receipts];

    fn name(self) -> &'static str {
        match self {
            Subsystem::Receipts => "Чеки",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Subsystem::Receipts => "Транзакции и учёт чеков",
        }
    }

    fn glyph(self) -> Icon {
        match self {
            Subsystem::Receipts => Icon::Receipt24Filled,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Credentials,
    TwoFactor { temp_token: String, setup: bool },
}

struct Setup {
    secret: String,
    uri: qr_code::Data,
    backup: Vec<String>,
}

enum Qr {
    Idle,
    Loading,
    Active { session_id: String, code: qr_code::Data, expires_at: Instant },
    Expired,
    Failed(String),
}

impl Qr {
    /// Past the button: the code, or what went wrong with it, needs the room.
    fn takes_the_pane(&self) -> bool {
        !matches!(self, Qr::Idle | Qr::Loading)
    }
}

pub struct Login {
    system: Subsystem,
    settings: bool,
    /// `None` until the first answer.
    server: Option<bool>,
    step: Step,
    username: String,
    users: Vec<String>,
    users_open: bool,
    marked: usize,
    password: String,
    reveal: bool,
    remember: bool,
    code: String,
    busy: bool,
    error: Option<String>,
    hint: Option<String>,
    status: Option<String>,
    setup: Option<Setup>,
    setup_code: String,
    setup_error: Option<String>,
    setup_busy: bool,
    qr: Qr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKey {
    Toggle,
    Next,
    Previous,
    Close,
}

#[derive(Clone, Debug)]
pub enum Msg {
    System(Subsystem),
    OpenSettings,
    CloseSettings,
    Theme(ThemeChoice),
    Username(String),
    ToggleUsers,
    CloseUsers,
    PickUser(String),
    ForgetUser(String),
    ListKey(ListKey),
    Password(String),
    ToggleReveal,
    Remember(bool),
    /// Enter in a field: picks from an open list, signs in otherwise.
    Enter,
    Submit,
    Answered(Result<LoginResponse, String>),
    Code(String),
    SubmitCode,
    Back,
    StartSetup,
    SetupReady(Result<TwoFactorSetup, String>),
    SetupCode(String),
    ConfirmSetup,
    SetupConfirmed(Result<TwoFactorConfirm, String>),
    QrStart,
    QrCancel,
    QrReady(Result<QrSession, String>),
    QrPoll,
    QrAnswer(Option<QrStatus>),
    QrTick,
    #[cfg(feature = "devtools")]
    QrDemo,
}

impl Msg {
    /// The list's own controls and the QR timers leave it open; anything else
    /// closes it.
    fn keeps_the_list(&self) -> bool {
        matches!(self, Msg::ToggleUsers | Msg::ForgetUser(_) | Msg::ListKey(_) | Msg::Enter | Msg::QrPoll | Msg::QrAnswer(_) | Msg::QrTick)
    }
}

fn list_key(event: Event, _status: event::Status, _window: window::Id) -> Option<Msg> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else { return None };
    let key = match key {
        Key::Named(Named::F4) if !modifiers.alt() => ListKey::Toggle,
        Key::Named(Named::ArrowDown) if modifiers.alt() => ListKey::Toggle,
        Key::Named(Named::ArrowDown) => ListKey::Next,
        Key::Named(Named::ArrowUp) => ListKey::Previous,
        Key::Named(Named::Escape) => ListKey::Close,
        _ => return None,
    };
    Some(Msg::ListKey(key))
}

fn focus(id: &'static str) -> Task<Msg> {
    operation::focus(Id::new(id))
}

impl Login {
    pub fn new(ctx: &Ctx) -> Self {
        let users = ctx.prefs.users();
        Self {
            system: Subsystem::Receipts,
            settings: false,
            server: None,
            step: Step::Credentials,
            remember: !users.is_empty(),
            username: users.first().cloned().unwrap_or_default(),
            users,
            users_open: false,
            marked: 0,
            password: String::new(),
            reveal: false,
            code: String::new(),
            busy: false,
            error: None,
            hint: None,
            status: None,
            setup: None,
            setup_code: String::new(),
            setup_error: None,
            setup_busy: false,
            qr: Qr::Idle,
        }
    }

    /// Where typing starts: a known user only has the password left to enter.
    pub fn focus_first(&self) -> Task<Msg> {
        focus(if self.username.is_empty() { USERNAME_ID } else { PASSWORD_ID })
    }

    pub fn server_answered(&mut self, online: bool) {
        self.server = Some(online);
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        let mut all = Vec::new();
        if matches!(self.qr, Qr::Active { .. }) {
            all.push(iced::time::every(Duration::from_secs(2)).map(|_| Msg::QrPoll));
            all.push(iced::time::every(Duration::from_secs(1)).map(|_| Msg::QrTick));
        }
        if self.form_shown() && !self.users.is_empty() {
            all.push(event::listen_with(list_key));
        }
        Subscription::batch(all)
    }

    fn form_shown(&self) -> bool {
        self.step == Step::Credentials && !self.settings && !self.qr.takes_the_pane()
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        if !message.keeps_the_list() {
            self.users_open = false;
        }
        match message {
            Msg::System(system) => {
                self.system = system;
                self.settings = false;
                if self.form_shown() {
                    return self.focus_first();
                }
            }
            Msg::OpenSettings => self.settings = true,
            Msg::CloseSettings => {
                self.settings = false;
                if self.form_shown() {
                    return self.focus_first();
                }
            }
            Msg::Theme(choice) => {
                ctx.prefs.theme = choice.saved();
                ctx.request(Request::ThemeChanged);
            }
            Msg::Username(value) => self.username = value,
            Msg::ToggleUsers => return self.toggle_users(),
            Msg::CloseUsers => {}
            Msg::PickUser(name) => return self.pick(name),
            Msg::ForgetUser(name) => return self.forget(&name, ctx),
            Msg::ListKey(key) => return self.walk_list(key),
            Msg::Password(value) => self.password = value,
            Msg::ToggleReveal => self.reveal = !self.reveal,
            Msg::Remember(value) => self.remember = value,
            Msg::Enter => {
                if self.users_open {
                    return match self.users.get(self.marked).cloned() {
                        Some(name) => self.pick(name),
                        None => Task::none(),
                    };
                }
                return self.submit(ctx);
            }
            Msg::Submit => return self.submit(ctx),
            Msg::Answered(answer) => return self.answered(answer, ctx),
            Msg::Code(value) => {
                self.code = value.chars().filter(char::is_ascii_alphanumeric).take(16).collect();
                if self.code.len() == 6 && self.code.chars().all(|c| c.is_ascii_digit()) && !self.busy {
                    return self.submit_code(ctx);
                }
            }
            Msg::SubmitCode => return self.submit_code(ctx),
            Msg::Back => {
                self.step = Step::Credentials;
                self.code.clear();
                self.error = None;
                self.hint = None;
                self.status = None;
                self.setup = None;
                self.setup_error = None;
                return self.focus_first();
            }
            Msg::StartSetup => {
                let Step::TwoFactor { temp_token, .. } = &self.step else { return Task::none() };
                self.setup_busy = true;
                self.setup_error = None;
                let (api, token) = (ctx.api.clone(), temp_token.clone());
                return Task::perform(
                    async move { api.two_factor_setup(Some(&token)).await.map_err(text_error) },
                    Msg::SetupReady,
                );
            }
            Msg::SetupReady(answer) => {
                self.setup_busy = false;
                match answer.and_then(|s| {
                    qr_code::Data::new(&s.otpauth_uri)
                        .map(|uri| Setup { secret: s.secret, uri, backup: s.backup_codes })
                        .map_err(text_error)
                }) {
                    Ok(setup) => self.setup = Some(setup),
                    Err(_) => self.setup_error = Some("Не удалось начать настройку 2FA".into()),
                }
            }
            Msg::SetupCode(value) => {
                self.setup_code = value.chars().filter(char::is_ascii_digit).take(6).collect();
            }
            Msg::ConfirmSetup => {
                let Step::TwoFactor { temp_token, .. } = &self.step else { return Task::none() };
                if self.setup_code.len() != 6 || self.setup_busy {
                    return Task::none();
                }
                self.setup_busy = true;
                self.setup_error = None;
                let (api, token, code) = (ctx.api.clone(), temp_token.clone(), self.setup_code.clone());
                return Task::perform(
                    async move { api.two_factor_confirm(&code, Some(&token)).await.map_err(text_error) },
                    Msg::SetupConfirmed,
                );
            }
            Msg::SetupConfirmed(answer) => {
                self.setup_busy = false;
                match answer {
                    Ok(reply) if reply.ok && reply.enabled => {
                        if let Step::TwoFactor { setup, .. } = &mut self.step {
                            *setup = false;
                        }
                        self.setup = None;
                        self.setup_code.clear();
                        self.status = Some("2FA успешно включена. Теперь введите код для входа.".into());
                        return focus(CODE_ID);
                    }
                    Ok(_) => self.setup_error = Some("Неверный код подтверждения".into()),
                    Err(_) => self.setup_error = Some("Не удалось подтвердить 2FA".into()),
                }
            }
            Msg::QrStart => {
                self.qr = Qr::Loading;
                let api = ctx.api.clone();
                return Task::perform(async move { api.qr_generate().await.map_err(text_error) }, Msg::QrReady);
            }
            Msg::QrCancel => {
                self.qr = Qr::Idle;
                return self.focus_first();
            }
            Msg::QrReady(answer) => {
                // Cancelled while the server was still answering.
                if !matches!(self.qr, Qr::Loading) {
                    return Task::none();
                }
                self.qr = match answer {
                    Ok(session) => match qr_code::Data::new(&session.qr_url) {
                        Ok(code) => Qr::Active {
                            session_id: session.session_id,
                            code,
                            expires_at: Instant::now() + Duration::from_secs(session.expires_in.max(0) as u64),
                        },
                        Err(_) => Qr::Failed("Не удалось создать QR".into()),
                    },
                    Err(reason) => Qr::Failed(qr_failure(reason)),
                };
            }
            Msg::QrPoll => {
                let Qr::Active { session_id, .. } = &self.qr else { return Task::none() };
                let (api, id) = (ctx.api.clone(), session_id.clone());
                // A failed poll is ignored: the next tick asks again.
                return Task::perform(async move { api.qr_status(&id).await.ok() }, Msg::QrAnswer);
            }
            Msg::QrAnswer(Some(status)) => match status.status.as_str() {
                "confirmed" if status.user.is_some() => {
                    self.qr = Qr::Idle;
                    self.status = Some("Готово, выполняется переход".into());
                    ctx.request(Request::SignedIn(status.user.unwrap()));
                }
                "expired" => self.qr = Qr::Expired,
                _ => {}
            },
            Msg::QrAnswer(None) => {}
            Msg::QrTick => {
                if let Qr::Active { expires_at, .. } = &self.qr {
                    if Instant::now() >= *expires_at {
                        self.qr = Qr::Expired;
                    }
                }
            }
            #[cfg(feature = "devtools")]
            Msg::QrDemo => {
                self.qr = Qr::Loading;
                let session = QrSession { session_id: "dev".into(), qr_url: "tg://login?token=dev".into(), expires_in: 120 };
                return Task::done(Msg::QrReady(Ok(session)));
            }
        }
        Task::none()
    }

    fn toggle_users(&mut self) -> Task<Msg> {
        if self.users_open {
            self.users_open = false;
            return focus(USERNAME_ID);
        }
        if !self.users.is_empty() {
            self.users_open = true;
            self.marked = self.users.iter().position(|name| *name == self.username).unwrap_or(0);
        }
        Task::none()
    }

    fn walk_list(&mut self, key: ListKey) -> Task<Msg> {
        let total = self.users.len();
        match key {
            ListKey::Toggle if self.form_shown() => return self.toggle_users(),
            ListKey::Close if self.users_open => return self.toggle_users(),
            ListKey::Next if self.users_open => self.marked = (self.marked + 1) % total,
            ListKey::Previous if self.users_open => self.marked = (self.marked + total - 1) % total,
            _ => {}
        }
        Task::none()
    }

    fn pick(&mut self, name: String) -> Task<Msg> {
        self.users_open = false;
        if self.username != name {
            self.password.clear();
            self.error = None;
            self.hint = None;
        }
        self.username = name;
        self.remember = true;
        focus(PASSWORD_ID)
    }

    fn forget(&mut self, name: &str, ctx: &mut Ctx) -> Task<Msg> {
        ctx.prefs.forget_user(name);
        ctx.save_prefs();
        self.users = ctx.prefs.users();
        self.marked = self.marked.min(self.users.len().saturating_sub(1));
        if self.username == name {
            self.username.clear();
            self.password.clear();
        }
        if self.users.is_empty() {
            self.users_open = false;
            self.remember = false;
            return focus(USERNAME_ID);
        }
        Task::none()
    }

    fn submit(&mut self, ctx: &Ctx) -> Task<Msg> {
        if self.busy || self.username.trim().is_empty() || self.password.is_empty() {
            return Task::none();
        }
        self.busy = true;
        self.error = None;
        self.hint = None;
        let (api, user, pass) = (ctx.api.clone(), self.username.trim().to_owned(), self.password.clone());
        Task::perform(async move { login_call(api, user, pass).await }, Msg::Answered)
    }

    fn submit_code(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Step::TwoFactor { temp_token, .. } = &self.step else { return Task::none() };
        if self.busy || self.code.is_empty() {
            return Task::none();
        }
        self.busy = true;
        self.error = None;
        let (api, token, code) = (ctx.api.clone(), temp_token.clone(), self.code.clone());
        Task::perform(
            async move { api.login_2fa(&token, &code).await.map_err(text_error) },
            Msg::Answered,
        )
    }

    fn answered(&mut self, answer: Result<LoginResponse, String>, ctx: &mut Ctx) -> Task<Msg> {
        self.busy = false;
        let reply = match answer {
            Ok(reply) => reply,
            Err(reason) => {
                self.error = Some(reason);
                return Task::none();
            }
        };
        if reply.requires_2fa {
            if let Some(temp_token) = reply.temp_token.clone() {
                self.step = Step::TwoFactor { temp_token, setup: reply.requires_2fa_setup };
                self.password.clear();
                self.code.clear();
                return focus(CODE_ID);
            }
        }
        if reply.ok {
            if let Some(user) = reply.user {
                self.password.clear();
                self.status = Some("Готово, выполняется переход".into());
                let name = self.username.trim();
                if self.remember {
                    ctx.prefs.remember_user(name);
                } else {
                    ctx.prefs.forget_user(name);
                }
                ctx.save_prefs();
                ctx.request(Request::SignedIn(user));
                return Task::none();
            }
        }
        let two_factor = matches!(self.step, Step::TwoFactor { .. });
        self.code.clear();
        self.error = Some(explain(&reply, two_factor));
        self.hint = reply.attempts_left.map(|left| {
            if left > 0 { format!("Осталось попыток: {left}") } else { "Лимит попыток исчерпан".into() }
        });
        Task::none()
    }

    pub fn view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let panes = container(row![self.systems(), self.account(ctx)]).width(Fill).height(Fill).style(theme::surface);
        let area = mouse_area(panes);
        if self.users_open { area.on_press(Msg::CloseUsers).into() } else { area.into() }
    }

    // ---- the brand half --------------------------------------------------

    fn systems(&self) -> El<'_, Msg> {
        let logo = svg(LOGO.clone()).width(LOGO_WIDTH).height(LOGO_HEIGHT).style(|theme: &Theme, _| svg::Style { color: Some(theme::p(theme).on_pane) });
        let line = container(Space::new()).width(1).height(LOGO_HEIGHT - 12.0).style(theme::pane_line);
        let brand = row![logo, line, text("CRM").size(size::XXL).font(theme::FONT_BOLD)].spacing(space::LG).align_y(Alignment::Center);
        let list = Subsystem::ALL.into_iter().fold(Column::new().spacing(space::SM), |list, system| list.push(self.system_row(system)));

        let (tone, state) = match self.server {
            None => (Tone::Neutral, "Проверяем сервер…"),
            Some(true) => (Tone::Success, "Сервер подключён"),
            Some(false) => (Tone::Danger, "Сервер недоступен"),
        };
        let dot = container(Space::new()).width(8).height(8).style(theme::pane_dot(tone));
        let footer = row![dot, quiet(state, size::SM), Space::new().width(Fill), quiet(format!("Версия {VERSION}"), size::SM)].spacing(space::SM).align_y(Alignment::Center);
        let settings = button(row![pane_icon(Icon::Settings20, 18.0), text("Настройки").size(size::BASE).font(theme::FONT_MEDIUM)].spacing(space::SM).align_y(Alignment::Center))
            .padding([space::SM - 1.0, space::MD - 2.0])
            .style(theme::btn_pane)
            .on_press(Msg::OpenSettings);

        let body = column![
            brand,
            Space::new().height(44),
            container(quiet("РАБОЧЕЕ ПРОСТРАНСТВО", size::XS).font(theme::FONT_BOLD)).padding(Padding { bottom: space::SM + 2.0, left: space::XXS, ..Padding::ZERO }),
            list,
            Space::new().height(Fill),
            settings,
            container(footer).padding(Padding { top: space::MD, left: space::MD - 2.0, right: space::XXS, ..Padding::ZERO }),
        ];
        container(body).width(PANE_WIDTH).height(Fill).padding(Padding { top: 32.0, bottom: 22.0, left: 26.0, right: 26.0 }).style(theme::sign_in_pane).into()
    }

    fn system_row(&self, system: Subsystem) -> El<'_, Msg> {
        let active = system == self.system;
        let tile = container(pane_icon(system.glyph(), 22.0)).center(40).style(theme::pane_tile);
        let caption = column![text(system.name()).size(size::LG - 1.0).font(theme::FONT_BOLD), quiet(system.hint(), size::SM)].spacing(1).width(Fill);
        let mut line = row![tile, caption].spacing(space::MD).align_y(Alignment::Center);
        if active {
            line = line.push(pane_icon(Icon::CheckCircle20Filled, 20.0));
        }
        button(line)
            .width(Fill)
            .padding(Padding { top: space::SM + 2.0, bottom: space::SM + 2.0, left: space::SM + 2.0, right: space::MD + 2.0 })
            .style(theme::btn_system(active))
            .on_press(Msg::System(system))
            .into()
    }

    // ---- the right half --------------------------------------------------

    fn account<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let body = if self.settings {
            self.settings_view(ctx)
        } else if self.qr.takes_the_pane() {
            self.qr_pane()
        } else {
            match &self.step {
                Step::Credentials => self.credentials(),
                Step::TwoFactor { setup: true, .. } => self.setup_view(),
                Step::TwoFactor { .. } => self.two_factor(),
            }
        };
        let form = container(container(body).max_width(FORM_WIDTH)).center_x(Fill).padding(Padding { top: 56.0, bottom: space::LG, left: space::XL, right: space::XL });
        ui::scroll(form).width(Fill).height(Fill).into()
    }

    fn feedback(&self) -> Column<'_, Msg> {
        let mut out = Column::new().spacing(space::XS);
        if let Some(status) = &self.status {
            out = out.push(ui::notice(Tone::Success, status.as_str()));
        }
        if let Some(error) = &self.error {
            out = out.push(ui::error_line(error.as_str()));
        }
        if let Some(hint) = &self.hint {
            out = out.push(ui::muted(hint.as_str()));
        }
        out
    }

    fn credentials(&self) -> El<'_, Msg> {
        let ready = !self.busy && !self.username.trim().is_empty() && !self.password.is_empty();
        let submit = primary(if self.busy { "Вход…" } else { "Войти" }, ready.then_some(Msg::Submit));
        let form = column![
            labeled("Пароль", self.password_field()),
            ui::check("Запомнить пользователя", self.remember, Msg::Remember),
            self.feedback(),
            submit,
            or_line(),
            self.telegram_button(),
        ]
        .spacing(space::MD + 2.0)
        .padding(Padding { top: space::MD + 2.0, ..Padding::ZERO });

        // The list opens over the form instead of pushing it down. The layer is
        // there even when empty: a tree that changes shape loses the focus.
        let list: El<'_, Msg> = if self.users_open { self.user_list() } else { Space::new().into() };
        column![
            heading(format!("Вход в {}", self.system.name()), "Введите данные вашей учётной записи"),
            Space::new().height(space::XL),
            labeled("Пользователь", self.user_field()),
            stack![form, list],
        ]
        .into()
    }

    fn user_field(&self) -> El<'_, Msg> {
        let input = roomy(ui::input("Логин", &self.username), !self.users.is_empty()).id(Id::new(USERNAME_ID)).on_input(Msg::Username).on_submit(Msg::Enter);
        let open: El<'_, Msg> = if self.users.is_empty() {
            Space::new().into()
        } else {
            inside_field(ui::icon_btn(Icon::ChevronDown, "Показать пользователей этого компьютера", Some(Msg::ToggleUsers)))
        };
        stack![input, open].into()
    }

    fn password_field(&self) -> El<'_, Msg> {
        let input = roomy(ui::input("Введите пароль", &self.password), true).id(Id::new(PASSWORD_ID)).secure(!self.reveal).on_input(Msg::Password).on_submit(Msg::Enter);
        let (glyph, hint) = if self.reveal { (Icon::EyeOff, "Скрыть пароль") } else { (Icon::Eye, "Показать пароль") };
        stack![input, inside_field(ui::icon_btn(glyph, hint, Some(Msg::ToggleReveal)))].into()
    }

    fn user_list(&self) -> El<'_, Msg> {
        let lines = self.users.iter().enumerate().fold(Column::new().spacing(space::XXS), |lines, (index, name)| {
            let pick = button(text(name.as_str()).size(size::BASE))
                .width(Fill)
                .padding([space::XS + 3.0, space::SM])
                .style(theme::btn_option(index == self.marked))
                .on_press(Msg::PickUser(name.clone()));
            lines.push(row![pick, ui::icon_btn(Icon::Close, "Убрать из списка", Some(Msg::ForgetUser(name.clone())))].spacing(space::XXS).align_y(Alignment::Center))
        });
        let panel = container(ui::scroll(lines)).padding(space::XS).max_height(LIST_HEIGHT).width(Fill).style(theme::popover);
        container(opaque(panel)).padding(Padding { top: space::XS, ..Padding::ZERO }).width(Fill).into()
    }

    /// The second way in: quiet beside the filled button, with Telegram's own mark.
    fn telegram_button(&self) -> El<'_, Msg> {
        let (caption, press) = match self.qr {
            Qr::Loading => ("Готовим QR…", None),
            _ => ("Войти через Telegram", Some(Msg::QrStart)),
        };
        let mark = svg(ui::icon_handle(Icon::Telegram)).width(18).height(18).style(|_: &Theme, _| svg::Style { color: Some(TELEGRAM_BLUE) });
        let line = row![mark, text(caption).size(size::MD).font(theme::FONT_MEDIUM)].spacing(space::SM).align_y(Alignment::Center);
        button(container(line).center_x(Fill).center_y(Fill)).width(Fill).height(control::LG).padding(0).style(theme::btn_secondary).on_press_maybe(press).into()
    }

    fn qr_pane(&self) -> El<'_, Msg> {
        let body: El<'_, Msg> = match &self.qr {
            Qr::Active { code, expires_at, .. } => {
                let left = expires_at.saturating_duration_since(Instant::now()).as_secs();
                column![
                    container(iced::widget::qr_code(code).total_size(QR_SIZE)).padding(space::SM).style(qr_backing),
                    ui::faint(format!("Действует: {}:{:02}", left / 60, left % 60)),
                ]
                .spacing(space::SM)
                .width(Fill)
                .align_x(Alignment::Center)
                .into()
            }
            Qr::Expired => primary("Обновить", Some(Msg::QrStart)),
            Qr::Failed(reason) => column![ui::error_line(reason.as_str()), primary("Повторить", Some(Msg::QrStart))].spacing(space::MD).into(),
            Qr::Idle | Qr::Loading => Space::new().into(),
        };
        let note = match self.qr {
            Qr::Expired => "Срок действия QR-кода истёк",
            Qr::Failed(_) => "QR-код получить не удалось",
            _ => "Отсканируйте QR-код в приложении Telegram",
        };
        column![heading("Вход через Telegram".to_owned(), note), body, back("Назад ко входу", Msg::QrCancel)]
            .spacing(space::LG + space::XS)
            .into()
    }

    fn two_factor(&self) -> El<'_, Msg> {
        let ready = !self.busy && !self.code.is_empty();
        column![
            heading("Подтверждение входа".to_owned(), "Введите шестизначный код из приложения или резервный код."),
            labeled("Код 2FA", roomy(ui::input("000000", &self.code), false).id(Id::new(CODE_ID)).on_input(Msg::Code).on_submit(Msg::SubmitCode)),
            self.feedback(),
            primary(if self.busy { "Проверка…" } else { "Подтвердить" }, ready.then_some(Msg::SubmitCode)),
            back("Назад ко входу", Msg::Back),
        ]
        .spacing(space::LG)
        .into()
    }

    fn setup_view(&self) -> El<'_, Msg> {
        let mut out = column![heading("Настройка 2FA".to_owned(), "Администратор требует двухфакторную защиту для этой учётной записи.")].spacing(space::MD);

        match &self.setup {
            None => {
                out = out.push(primary(if self.setup_busy { "Готовим QR…" } else { "Начать настройку" }, (!self.setup_busy).then_some(Msg::StartSetup)));
            }
            Some(setup) => {
                let code = container(qr_code(&setup.uri).total_size(SETUP_QR_SIZE)).padding(space::SM).style(qr_backing);
                let keys = column![
                    ui::muted("Секретный ключ"),
                    text(setup.secret.as_str()).font(theme::FONT_MONO).size(size::SM),
                    Space::new().height(space::XS),
                    ui::muted("Резервные коды"),
                    text(setup.backup.join(", ")).font(theme::FONT_MONO).size(size::SM),
                ]
                .spacing(space::XS)
                .width(Fill);
                out = out
                    .push(row![code, keys].spacing(space::MD))
                    .push(labeled(
                        "Код из приложения",
                        roomy(ui::input("000000", &self.setup_code), false).on_input(Msg::SetupCode).on_submit(Msg::ConfirmSetup),
                    ))
                    .push(primary(
                        if self.setup_busy { "Проверка…" } else { "Подтвердить" },
                        (!self.setup_busy && self.setup_code.len() == 6).then_some(Msg::ConfirmSetup),
                    ));
            }
        }
        if let Some(error) = &self.setup_error {
            out = out.push(ui::error_line(error.as_str()));
        }
        out.push(back("Назад ко входу", Msg::Back)).into()
    }

    fn settings_view<'a>(&'a self, ctx: &'a Ctx) -> El<'a, Msg> {
        let choice = ThemeChoice::from_saved(ctx.prefs.theme.as_deref());
        let pick = |glyph: Icon, caption: &'static str, value: ThemeChoice| {
            let kind = if choice == value { Kind::Primary } else { Kind::Secondary };
            ui::btn_icon(glyph, caption, kind, Some(Msg::Theme(value))).width(Fill)
        };
        let themes = row![pick(Icon::Sun, "Светлая", ThemeChoice::Light), pick(Icon::Moon, "Тёмная", ThemeChoice::Dark), pick(Icon::Monitor, "Системная", ThemeChoice::System)].spacing(space::SM);
        let address = container(text(ctx.api.base()).font(theme::FONT_MONO).size(size::SM)).width(Fill).padding([space::SM + 1.0, space::MD]).style(theme::inset);
        column![
            heading("Настройки".to_owned(), "Действуют на этом компьютере"),
            labeled("Тема", themes),
            labeled("Сервер", column![address, ui::faint("Адрес задаётся при установке клиента.")].spacing(space::XS)),
            back("Назад", Msg::CloseSettings),
        ]
        .spacing(space::LG + space::XS)
        .into()
    }

    #[cfg(feature = "devtools")]
    pub fn probe(name: &str) -> Option<Msg> {
        let two_factor = |setup: bool| LoginResponse { requires_2fa: true, requires_2fa_setup: setup, temp_token: Some("dev".into()), ..Default::default() };
        Some(match name {
            "l-users" => Msg::ToggleUsers,
            "l-next" => Msg::ListKey(ListKey::Next),
            "l-receipts" => Msg::System(Subsystem::Receipts),
            "l-settings" => Msg::OpenSettings,
            "l-settings-off" => Msg::CloseSettings,
            "l-qr" => Msg::QrStart,
            "l-qr-demo" => Msg::QrDemo,
            "l-qr-off" => Msg::QrCancel,
            "l-2fa" => Msg::Answered(Ok(two_factor(false))),
            "l-setup" => Msg::Answered(Ok(two_factor(true))),
            "l-setup-qr" => Msg::SetupReady(Ok(TwoFactorSetup {
                secret: "JBSWY3DPEHPK3PXP".into(),
                otpauth_uri: "otpauth://totp/dev".into(),
                backup_codes: (1..=8).map(|n| format!("dev-{n:04}")).collect(),
                ..Default::default()
            })),
            "l-back" => Msg::Back,
            "l-wrong" => Msg::Answered(Ok(LoginResponse { attempts_left: Some(3), ..Default::default() })),
            "l-password" => Msg::Password("dev-only".into()),
            _ => return None,
        })
    }
}

fn heading<'a>(title: String, note: &'a str) -> El<'a, Msg> {
    column![text(title).size(size::XXL).font(theme::FONT_BOLD), ui::muted(note).size(size::MD)].spacing(space::XS).into()
}

fn labeled<'a>(caption: &'a str, control: impl Into<El<'a, Msg>>) -> El<'a, Msg> {
    column![text(caption).size(size::BASE).font(theme::FONT_BOLD), control.into()].spacing(space::XS + 2.0).into()
}

/// The taller field of this window, with room on the right for a button inside.
fn roomy(input: text_input::TextInput<'_, Msg>, button_inside: bool) -> text_input::TextInput<'_, Msg> {
    let right = if button_inside { control::MD + space::SM } else { space::MD };
    input.size(size::MD).padding(Padding { top: space::MD - 2.0, bottom: space::MD - 2.0, left: space::MD, right })
}

fn inside_field(control: El<'_, Msg>) -> El<'_, Msg> {
    container(control).align_right(Fill).center_y(Fill).padding(Padding { right: space::XXS + 1.0, ..Padding::ZERO }).into()
}

fn primary<'a>(caption: &'a str, press: Option<Msg>) -> El<'a, Msg> {
    let body = container(text(caption).size(size::MD).font(theme::FONT_MEDIUM)).center_x(Fill).center_y(Fill);
    button(body).width(Fill).height(control::LG).padding(0).style(theme::btn_primary).on_press_maybe(press).into()
}

fn back<'a>(caption: &'a str, message: Msg) -> El<'a, Msg> {
    let line = row![ui::icon(Icon::ArrowLeft, 16.0), ui::muted(caption).size(size::BASE)].spacing(space::XS).align_y(Alignment::Center);
    button(line).style(theme::btn_link).padding(0).on_press(message).into()
}

/// Between the two ways in.
fn or_line<'a>() -> El<'a, Msg> {
    row![ui::divider(), ui::faint("или"), ui::divider()].spacing(space::MD).align_y(Alignment::Center).into()
}

/// An icon in the ink of the navy pane.
fn pane_icon<'a>(name: Icon, px: f32) -> svg::Svg<'a> {
    svg(ui::icon_handle(name)).width(px).height(px).style(|theme: &Theme, _| svg::Style { color: Some(theme::p(theme).on_pane) })
}

/// The quieter text of the navy pane.
fn quiet<'a>(value: impl text::IntoFragment<'a>, px: f32) -> text::Text<'a> {
    text(value).size(px).style(|theme: &Theme| text::Style { color: Some(theme::p(theme).on_pane2) })
}

/// A QR code reads only dark on light, whatever the theme.
fn qr_backing(_: &Theme) -> container::Style {
    container::Style { background: Some(Color::WHITE.into()), border: iced::border::rounded(theme::radius::MD), ..Default::default() }
}

/// The server answers a refused login with a status, not always a body we can
/// parse as a reply; both shapes reach the form the same way.
async fn login_call(api: Api, username: String, password: String) -> Result<LoginResponse, String> {
    match api.login(&username, &password).await {
        Ok(reply) => Ok(reply),
        Err(ApiError::Http { code, message, .. }) => {
            let reason = code.unwrap_or(message);
            Ok(LoginResponse { error: Some(reason), ..Default::default() })
        }
        Err(error) if error.is_network() => Err("Сервер недоступен. Проверьте подключение.".into()),
        Err(error) => Err(error.to_string()),
    }
}

/// The one refusal a person can act on comes from the server in English.
fn qr_failure(reason: String) -> String {
    match reason.as_str() {
        "" => "Не удалось создать QR".into(),
        "Telegram bot not configured" => "Вход через Telegram на этом сервере не настроен".into(),
        _ => reason,
    }
}

fn explain(reply: &LoginResponse, two_factor: bool) -> String {
    let code = reply.error.as_deref().unwrap_or("");
    match code {
        "locked" => match reply.locked_seconds {
            Some(seconds) => format!("Пользователь временно заблокирован ({seconds} сек)"),
            None => "Пользователь временно заблокирован".into(),
        },
        "inactive_user" => "Пользователь отключен администратором".into(),
        "rate_limited" => "Слишком много попыток. Подождите минуту.".into(),
        "2fa_setup_required" => "Сначала завершите настройку 2FA".into(),
        _ if two_factor => "Неверный 2FA код".into(),
        _ => "Неверный логин или пароль".into(),
    }
}

#[cfg(test)]
mod tests {
    use iced_test::Simulator;
    use iced_test::core::Settings;
    use tbs_core::model::UserInfo;

    use super::*;

    fn with_users(dir: &tempfile::TempDir, names: &[&str]) -> Ctx {
        let mut ctx = Ctx::for_tests(dir.path());
        for name in names.iter().rev() {
            ctx.prefs.remember_user(name);
        }
        ctx
    }

    fn signed_in() -> Msg {
        Msg::Answered(Ok(LoginResponse { ok: true, user: Some(UserInfo::default()), ..Default::default() }))
    }

    /// Everything the window shows at the one size it comes in.
    fn sees(screen: &Login, ctx: &Ctx, wanted: &str) -> bool {
        let mut sim = Simulator::with_size(Settings::default(), WINDOW, screen.view(ctx));
        sim.find(wanted).is_ok()
    }

    fn click(screen: &Login, ctx: &Ctx, caption: &str) -> Vec<Msg> {
        let mut sim = Simulator::with_size(Settings::default(), WINDOW, screen.view(ctx));
        sim.click(caption).unwrap_or_else(|_| panic!("no «{caption}» on the screen"));
        sim.into_messages().collect()
    }

    #[test]
    fn explains_lockout_with_seconds() {
        let reply = LoginResponse { error: Some("locked".into()), locked_seconds: Some(42), ..Default::default() };
        assert_eq!(explain(&reply, false), "Пользователь временно заблокирован (42 сек)");
    }

    #[test]
    fn wrong_second_factor_is_not_called_a_wrong_password() {
        let reply = LoginResponse { error: Some("invalid_code".into()), ..Default::default() };
        assert_eq!(explain(&reply, true), "Неверный 2FA код");
        assert_eq!(explain(&reply, false), "Неверный логин или пароль");
    }

    #[test]
    fn a_first_launch_knows_nobody() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = with_users(&dir, &[]);
        let screen = Login::new(&ctx);
        assert_eq!(screen.username, "");
        assert!(!screen.remember);
        assert!(sees(&screen, &ctx, "Вход в Чеки"));
        assert!(sees(&screen, &ctx, "Проверяем сервер…"));
    }

    #[test]
    fn the_last_user_is_ready_and_the_password_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = with_users(&dir, &["anna", "boris"]);
        let screen = Login::new(&ctx);
        assert_eq!(screen.username, "anna");
        assert!(screen.remember);
        assert_eq!(screen.password, "");
    }

    #[test]
    fn choosing_from_the_list_moves_on_to_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &["anna", "boris"]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::Password("typed for anna".into()), &mut ctx);
        let _ = screen.update(Msg::ToggleUsers, &mut ctx);
        assert!(screen.users_open);
        assert!(sees(&screen, &ctx, "boris"));

        let messages = click(&screen, &ctx, "boris");
        assert!(matches!(messages.as_slice(), [Msg::PickUser(name)] if name == "boris"), "{messages:?}");
        let _ = screen.update(Msg::PickUser("boris".into()), &mut ctx);
        assert_eq!(screen.username, "boris");
        assert_eq!(screen.password, "", "one person's password must not stay for another");
        assert!(!screen.users_open);
    }

    #[test]
    fn the_keyboard_walks_the_list_and_enter_picks() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &["anna", "boris", "vera"]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::ListKey(ListKey::Next), &mut ctx);
        assert_eq!(screen.marked, 0, "a closed list has nothing to walk");

        let _ = screen.update(Msg::ListKey(ListKey::Toggle), &mut ctx);
        assert!(screen.users_open);
        let _ = screen.update(Msg::ListKey(ListKey::Previous), &mut ctx);
        assert_eq!(screen.marked, 2, "up from the first line wraps to the last");
        let _ = screen.update(Msg::ListKey(ListKey::Next), &mut ctx);
        let _ = screen.update(Msg::ListKey(ListKey::Next), &mut ctx);
        assert_eq!(screen.marked, 1);

        let _ = screen.update(Msg::Enter, &mut ctx);
        assert_eq!(screen.username, "boris");
        assert!(!screen.busy, "Enter on the list must not send the form");
    }

    #[test]
    fn enter_signs_in_once_the_list_is_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &["anna"]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::Password("secret".into()), &mut ctx);
        let _ = screen.update(Msg::ToggleUsers, &mut ctx);
        let _ = screen.update(Msg::ListKey(ListKey::Close), &mut ctx);
        assert!(!screen.users_open);
        let _ = screen.update(Msg::Enter, &mut ctx);
        assert!(screen.busy);
    }

    #[test]
    fn the_sign_in_button_works_past_an_open_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &["anna", "boris"]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::Password("secret".into()), &mut ctx);
        let _ = screen.update(Msg::ToggleUsers, &mut ctx);
        let _ = screen.update(Msg::Submit, &mut ctx);
        assert_eq!(screen.username, "anna");
        assert!(screen.busy);
        assert!(!screen.users_open);
    }

    #[test]
    fn typing_a_new_login_closes_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &["anna"]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::ToggleUsers, &mut ctx);
        let _ = screen.update(Msg::Username("newcomer".into()), &mut ctx);
        assert!(!screen.users_open);
        assert_eq!(screen.username, "newcomer");
    }

    #[test]
    fn removing_a_user_is_saved_and_leaves_the_list_usable() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &["anna", "boris"]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::ToggleUsers, &mut ctx);
        let _ = screen.update(Msg::ListKey(ListKey::Next), &mut ctx);
        let _ = screen.update(Msg::ForgetUser("boris".into()), &mut ctx);
        assert_eq!(ctx.prefs.users(), ["anna"]);
        assert_eq!(screen.users, ["anna"]);
        assert_eq!(screen.marked, 0);
        assert!(screen.users_open);
        assert_eq!(screen.username, "anna");

        let _ = screen.update(Msg::ForgetUser("anna".into()), &mut ctx);
        assert!(!screen.users_open);
        assert_eq!(screen.username, "");
        assert!(!screen.remember);
    }

    #[test]
    fn a_successful_sign_in_decides_who_stays_on_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &["anna"]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::Username(" boris ".into()), &mut ctx);
        let _ = screen.update(Msg::Password("secret".into()), &mut ctx);
        let _ = screen.update(signed_in(), &mut ctx);
        assert_eq!(ctx.prefs.users(), ["boris", "anna"]);
        assert_eq!(screen.password, "");

        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::Remember(false), &mut ctx);
        let _ = screen.update(signed_in(), &mut ctx);
        assert_eq!(ctx.prefs.users(), ["anna"]);
    }

    #[test]
    fn a_refused_password_remembers_nobody() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &[]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::Username("typo".into()), &mut ctx);
        let _ = screen.update(Msg::Remember(true), &mut ctx);
        let _ = screen.update(Msg::Answered(Ok(LoginResponse::default())), &mut ctx);
        assert!(ctx.prefs.users().is_empty());
        assert!(sees(&screen, &ctx, "Неверный логин или пароль"));
    }

    #[test]
    fn the_second_factor_keeps_the_login_for_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &[]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::Username("anna".into()), &mut ctx);
        let _ = screen.update(Msg::Remember(true), &mut ctx);
        let _ = screen.update(Msg::Answered(Ok(LoginResponse { requires_2fa: true, temp_token: Some("t".into()), ..Default::default() })), &mut ctx);
        assert!(sees(&screen, &ctx, "Подтверждение входа"));
        let _ = screen.update(signed_in(), &mut ctx);
        assert_eq!(ctx.prefs.users(), ["anna"]);
    }

    #[test]
    fn only_working_subsystems_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &[]);
        let mut screen = Login::new(&ctx);
        for placeholder in ["LST", "Клиенты и сделки", "Скоро"] {
            assert!(!sees(&screen, &ctx, placeholder), "{placeholder} is still listed");
        }
        let messages = click(&screen, &ctx, "Транзакции и учёт чеков");
        assert!(matches!(messages.as_slice(), [Msg::System(Subsystem::Receipts)]), "{messages:?}");

        let _ = screen.update(Msg::System(Subsystem::Receipts), &mut ctx);
        assert!(sees(&screen, &ctx, "Вход в Чеки"));
        assert!(sees(&screen, &ctx, "Войти"));
        assert!(sees(&screen, &ctx, "Войти через Telegram"));
    }

    #[test]
    fn the_window_carries_the_product_and_not_the_old_name() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = with_users(&dir, &[]);
        let screen = Login::new(&ctx);
        // The logo reads «TBS inform»; the text beside it completes the name.
        assert!(sees(&screen, &ctx, "CRM"));
        assert!(sees(&screen, &ctx, "РАБОЧЕЕ ПРОСТРАНСТВО"));
        assert!(!sees(&screen, &ctx, "TBSparcer"));
        assert!(crate::app::PRODUCT.ends_with("CRM"));
    }

    #[test]
    fn the_server_line_follows_the_ping() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = with_users(&dir, &[]);
        let mut screen = Login::new(&ctx);
        screen.server_answered(true);
        assert!(sees(&screen, &ctx, "Сервер подключён"));
        screen.server_answered(false);
        assert!(sees(&screen, &ctx, "Сервер недоступен"));
    }

    #[test]
    fn the_theme_can_be_changed_before_signing_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &[]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::OpenSettings, &mut ctx);
        assert!(sees(&screen, &ctx, "Тема"));
        assert!(sees(&screen, &ctx, ctx.api.base().as_str()));

        let _ = screen.update(Msg::Theme(ThemeChoice::Dark), &mut ctx);
        assert_eq!(ctx.prefs.theme.as_deref(), Some("dark"));
        assert!(matches!(ctx.requests.as_slice(), [Request::ThemeChanged]));

        let _ = screen.update(Msg::CloseSettings, &mut ctx);
        assert!(sees(&screen, &ctx, "Вход в Чеки"));
    }

    #[test]
    fn an_abandoned_qr_answer_does_not_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &[]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::QrStart, &mut ctx);
        let _ = screen.update(Msg::QrCancel, &mut ctx);
        let session = QrSession { session_id: "s".into(), qr_url: "tg://login".into(), expires_in: 60 };
        let _ = screen.update(Msg::QrReady(Ok(session)), &mut ctx);
        assert!(matches!(screen.qr, Qr::Idle));
    }

    #[test]
    fn a_server_without_a_bot_is_explained_in_russian() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &[]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::QrStart, &mut ctx);
        let _ = screen.update(Msg::QrReady(Err("Telegram bot not configured".into())), &mut ctx);
        assert!(sees(&screen, &ctx, "Вход через Telegram на этом сервере не настроен"));
        assert!(!sees(&screen, &ctx, "Отсканируйте QR-код в приложении Telegram"));
    }

    #[test]
    fn the_code_takes_the_whole_pane_and_goes_back_to_the_form() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = with_users(&dir, &[]);
        let mut screen = Login::new(&ctx);
        let _ = screen.update(Msg::QrStart, &mut ctx);
        let session = QrSession { session_id: "s".into(), qr_url: "tg://login".into(), expires_in: 60 };
        let _ = screen.update(Msg::QrReady(Ok(session)), &mut ctx);
        assert!(sees(&screen, &ctx, "Вход через Telegram"));
        assert!(!sees(&screen, &ctx, "Войти"));
        let _ = screen.update(Msg::QrCancel, &mut ctx);
        assert!(sees(&screen, &ctx, "Войти"));
    }
}
