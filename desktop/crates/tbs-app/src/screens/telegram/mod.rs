//! The Telegram section, drawn by the client itself: who may open it, the
//! server's Telegram sign-in, the chats and their messages. Only a mini app's
//! own page is a web page, in a web view the section frames.

mod auth;
mod bridge;
mod client;
mod dates;
pub(crate) mod host;
mod layers;
mod list;
mod miniapp;
mod pane;
mod pointer;
mod style;

use std::time::{Duration, Instant};

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{column, container, row};
use iced::{Alignment, Event, Fill, Subscription, Task, event, window};
use tbs_core::api::{AccessScope, AccessStatus, ScopeCode, UnlockedScope};
use tbs_core::links;

use crate::ctx::{Ctx, Request, text_error};
use crate::theme::{self, Tone, space};
use crate::ui::{self, El, Icon, Kind};

use client::{Client, Want};
pub use host::Signal;
use miniapp::MiniApp;

/// Narrower than this the list and the open chat take turns, as on a phone.
const TWO_PANES: f32 = 768.0;

#[derive(Clone, Debug, PartialEq)]
enum Access {
    Idle,
    Checking,
    Closed { title: &'static str, body: String },
    Gate,
    Open,
}

#[derive(Default)]
struct Otp {
    scope: i64,
    code: String,
    requesting: bool,
    verifying: bool,
    expires_at: Option<Instant>,
    seconds_left: u64,
    error: Option<String>,
}

/// Keys the section hears wherever the pointer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Escape,
    Enter,
    SelectAll,
    SelectNone,
    Command(bool),
}

#[derive(Clone, Debug)]
pub enum Msg {
    Status(Result<AccessStatus, String>),
    Scopes(Result<Vec<AccessScope>, String>),
    Code(String),
    Request,
    Requested(Result<ScopeCode, String>),
    Verify,
    Verified(Result<UnlockedScope, String>),
    Tick,
    Retry,
    Asked(String, bool),
    Signal(Signal),
    Chats(client::Msg),
    Mini(miniapp::Msg),
    Hotkey(Hotkey),
    Pump,
}

pub struct Telegram {
    access: Access,
    shown: bool,
    asking: bool,
    otp: Otp,
    client: Client,
    mini: MiniApp,
}

async fn confirm_external(url: String, host: String) -> bool {
    let answer = rfd::AsyncMessageDialog::new()
        .set_title("Открыть внешнюю ссылку?")
        .set_description(format!("Открыть {host} вне {}?\n{url}", crate::app::PRODUCT))
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show()
        .await;
    answer == rfd::MessageDialogResult::Ok
}

pub(crate) fn open_in_system(url: &str) {
    if let Err(error) = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn() {
        tracing::warn!("could not open an external link: {error}");
    }
}

/// Escape always; the rest only when no field took the key for itself.
fn hotkey(event: Event, status: event::Status, _window: window::Id) -> Option<Msg> {
    let key = match event {
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => Hotkey::Command(modifiers.command()),
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, physical_key, .. }) => {
            if key == Key::Named(Named::Escape) {
                Hotkey::Escape
            } else if status != event::Status::Ignored || modifiers.shift() || modifiers.alt() {
                return None;
            } else if key == Key::Named(Named::Enter) && !modifiers.command() {
                Hotkey::Enter
            } else if modifiers.command() {
                match key.to_latin(physical_key) {
                    Some('a') => Hotkey::SelectAll,
                    Some('d') => Hotkey::SelectNone,
                    _ => return None,
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(Msg::Hotkey(key))
}

impl Default for Telegram {
    fn default() -> Self {
        Self::new()
    }
}

impl Telegram {
    pub fn new() -> Self {
        Self { access: Access::Idle, shown: false, asking: false, otp: Otp::default(), client: Client::new(), mini: MiniApp::default() }
    }

    /// Called on every visit: resumes whatever the last one left unfinished.
    pub fn enter(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        match self.access {
            Access::Idle | Access::Closed { .. } => self.check(ctx),
            Access::Open => self.client.resume(ctx).map(Msg::Chats),
            _ => Task::none(),
        }
    }

    fn check(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        if ctx.is_admin() {
            return self.open(ctx);
        }
        self.access = Access::Checking;
        let api = ctx.api.clone();
        Task::perform(async move { api.access_status().await.map_err(|e| text_error(&e)) }, Msg::Status)
    }

    fn open(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.access = Access::Open;
        self.otp = Otp::default();
        self.client.resume(ctx).map(Msg::Chats)
    }

    fn visible(&self) -> bool {
        self.shown && self.access == Access::Open
    }

    /// Follows the app's route. A mini app's web view floats above everything
    /// drawn by the app, so one left visible would cover the next screen.
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
        self.mini.show(self.visible());
    }

    /// The window or the rail changed size: a mini app keeps to its frame.
    pub fn resized(&self, ctx: &Ctx) {
        self.mini.resized(ctx);
    }

    pub fn theme_changed(&self, ctx: &Ctx) {
        self.mini.theme_changed(ctx);
    }

    /// The shared access token ran out; the section closes until it is renewed.
    pub fn scope_expired(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let closing = self.mini.close(ctx).map(Msg::Mini);
        self.access = Access::Idle;
        if self.shown { Task::batch([closing, self.check(ctx)]) } else { closing }
    }

    /// Carries out what the chats and a mini app left to the section, and
    /// keeps the web view where and as it should be.
    fn settle(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        let mut tasks = Vec::new();
        for want in std::mem::take(&mut self.client.wants) {
            match want {
                Want::App(request) => tasks.push(self.mini.open(request, ctx).map(Msg::Mini)),
                Want::Link(url) => {
                    if links::classify_external(&url).is_none() {
                        ctx.toast(Tone::Warning, "Ссылка не открыта: вне приложения открываются только ссылки Telegram");
                    }
                    tasks.push(self.ask_external(&url));
                }
            }
        }
        for url in std::mem::take(&mut self.mini.links) {
            tasks.push(self.ask_external(&url));
        }
        self.mini.place(ctx);
        self.mini.show(self.visible());
        Task::batch(tasks)
    }

    fn request_code(&mut self, ctx: &Ctx) -> Task<Msg> {
        if self.otp.requesting {
            return Task::none();
        }
        self.otp.requesting = true;
        self.otp.error = None;
        let (api, id) = (ctx.api.clone(), self.otp.scope);
        Task::perform(async move { api.request_scope_code(id).await.map_err(|e| text_error(&e)) }, Msg::Requested)
    }

    /// One question at a time: a page cannot stack dialogs by firing links.
    fn ask_external(&mut self, raw: &str) -> Task<Msg> {
        let Some(link) = links::classify_external(raw) else {
            tracing::info!("external link dropped by policy");
            return Task::none();
        };
        if self.asking {
            return Task::none();
        }
        self.asking = true;
        let url = link.url.clone();
        Task::perform(confirm_external(link.url, link.host), move |yes| Msg::Asked(url.clone(), yes))
    }

    fn on_hotkey(&mut self, key: Hotkey, ctx: &mut Ctx) -> Task<Msg> {
        if self.access != Access::Open {
            return Task::none();
        }
        // An open mini app is all there is to act on; what is under it waits.
        let inner = match key {
            Hotkey::Command(held) => client::Msg::Command(held),
            Hotkey::Escape if self.mini.is_open() => return self.mini.escape(ctx).map(Msg::Mini),
            Hotkey::Escape => {
                self.client.escape();
                return Task::none();
            }
            _ if self.mini.is_open() || self.client.layered() || !self.client.selecting => return Task::none(),
            Hotkey::Enter => client::Msg::BulkStart,
            Hotkey::SelectAll => client::Msg::SelectAll,
            Hotkey::SelectNone => client::Msg::SelectNone,
        };
        self.client.update(inner, ctx).map(Msg::Chats)
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        let task = self.route(message, ctx);
        let follow = self.settle(ctx);
        Task::batch([task, follow])
    }

    fn route(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Status(Ok(status)) => {
                let allowed = !status.scopes_enabled || status.current_scope.is_some_and(|scope| scope.allow_sources);
                if allowed {
                    return self.open(ctx);
                }
                let api = ctx.api.clone();
                return Task::perform(async move { api.access_scopes().await.map_err(|e| text_error(&e)) }, Msg::Scopes);
            }
            Msg::Status(Err(reason)) => {
                tracing::warn!("access status failed: {reason}");
                self.access = Access::Closed {
                    title: "Доступ временно ограничен",
                    body: "Не удалось проверить статус безопасности. До успешной проверки раздел остаётся закрыт.".to_owned(),
                };
            }
            Msg::Scopes(Ok(scopes)) => match scopes.into_iter().find(|scope| scope.is_active && scope.allow_sources) {
                Some(scope) => {
                    self.otp = Otp { scope: scope.id, ..Otp::default() };
                    self.access = Access::Gate;
                    return self.request_code(ctx);
                }
                None => {
                    self.access = Access::Closed {
                        title: "Доступ закрыт",
                        body: "Нет активной папки доступа с разрешением на источники. Включите доступ к источникам в настройках безопасности.".to_owned(),
                    };
                }
            },
            Msg::Scopes(Err(reason)) => self.access = Access::Closed { title: "Доступ закрыт", body: reason },
            Msg::Code(value) => self.otp.code = value.chars().filter(char::is_ascii_digit).take(6).collect(),
            Msg::Request => return self.request_code(ctx),
            Msg::Requested(Ok(code)) => {
                self.otp.requesting = false;
                if code.otp_skipped && code.token.is_some() {
                    ctx.request(Request::TransactionsChanged);
                    return self.open(ctx);
                }
                let lifetime = u64::try_from(code.expires_in).unwrap_or(0);
                self.otp.expires_at = Some(Instant::now() + Duration::from_secs(lifetime));
                self.otp.seconds_left = lifetime;
            }
            Msg::Requested(Err(reason)) => {
                self.otp.requesting = false;
                self.otp.error = Some(reason);
            }
            Msg::Verify => {
                if self.otp.verifying || self.otp.code.len() < 4 {
                    return Task::none();
                }
                self.otp.verifying = true;
                self.otp.error = None;
                let (api, id, code) = (ctx.api.clone(), self.otp.scope, self.otp.code.clone());
                return Task::perform(async move { api.verify_scope_code(id, &code).await.map_err(|e| text_error(&e)) }, Msg::Verified);
            }
            Msg::Verified(Ok(_)) => {
                ctx.request(Request::TransactionsChanged);
                return self.open(ctx);
            }
            Msg::Verified(Err(reason)) => {
                self.otp.verifying = false;
                self.otp.error = Some(reason);
            }
            Msg::Tick => {
                if let Some(deadline) = self.otp.expires_at {
                    self.otp.seconds_left = deadline.saturating_duration_since(Instant::now()).as_secs();
                }
                if self.visible() {
                    return self.client.poll(ctx, Instant::now()).map(Msg::Chats);
                }
            }
            Msg::Retry => return self.check(ctx),
            Msg::Asked(url, yes) => {
                self.asking = false;
                if yes {
                    open_in_system(&url);
                }
            }
            Msg::Signal(signal) => return self.mini.update(miniapp::Msg::Signal(signal), ctx).map(Msg::Mini),
            Msg::Chats(inner) => return self.client.update(inner, ctx).map(Msg::Chats),
            Msg::Mini(inner) => return self.mini.update(inner, ctx).map(Msg::Mini),
            Msg::Hotkey(key) => return self.on_hotkey(key, ctx),
            Msg::Pump => crate::screens::hosted::host::pump(),
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        let clock = iced::time::every(Duration::from_secs(1)).map(|_| Msg::Tick);
        match self.access {
            Access::Gate => clock,
            Access::Open => Subscription::batch([clock, event::listen_with(hotkey)]),
            _ => Subscription::none(),
        }
    }

    /// A mini app keeps talking whichever section is open.
    pub fn background(&self) -> Subscription<Msg> {
        let signals = host::signals().map(Msg::Signal);
        // On Linux the web view is WebKitGTK, whose toolkit nobody else runs.
        if cfg!(target_os = "linux") && self.mini.is_open() {
            Subscription::batch([signals, iced::time::every(crate::screens::hosted::host::PUMP_EVERY).map(|_| Msg::Pump)])
        } else {
            signals
        }
    }

    fn closed_view<'a>(&'a self, title: &'a str, body: &'a str) -> El<'a, Msg> {
        let content = column![
            row![ui::icon_tone(Icon::Lock, 16.0, Tone::Warning), ui::strong(title)].spacing(space::SM).align_y(Alignment::Center),
            ui::muted(body),
            ui::btn("Повторить", Kind::Secondary, Some(Msg::Retry)),
        ]
        .spacing(space::MD);
        centered(ui::card(content).max_width(460).into())
    }

    fn gate_view(&self) -> El<'_, Msg> {
        let otp = &self.otp;
        let can_verify = !otp.verifying && otp.code.len() >= 4;
        let resend = if otp.requesting { "Запрос..." } else if otp.expires_at.is_some() { "Отправить повторно" } else { "Запросить OTP" };
        let form = row![
            ui::input("Введите код", &otp.code).on_input(Msg::Code).on_submit(Msg::Verify).width(160),
            ui::btn(if otp.verifying { "Проверка..." } else { "Подтвердить код" }, Kind::Primary, can_verify.then_some(Msg::Verify)),
            ui::btn(resend, Kind::Secondary, (!otp.requesting).then_some(Msg::Request)),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);
        let mut content = column![
            row![ui::icon_tone(Icon::Lock, 16.0, Tone::Warning), ui::strong("Доступ ограничен")].spacing(space::SM).align_y(Alignment::Center),
            ui::muted("Для открытия раздела нужен OTP-код. Код отправляется в AuthBot после запроса."),
            form,
        ]
        .spacing(space::MD);
        if otp.seconds_left > 0 {
            content = content.push(ui::muted(format!("Код действителен {} с", otp.seconds_left)));
        } else if otp.expires_at.is_some() {
            content = content.push(ui::toned("Код истек, запросите новый", Tone::Warning));
        }
        if let Some(reason) = &otp.error {
            content = content.push(ui::error_line(reason.as_str()));
        }
        centered(ui::card(content).max_width(520).into())
    }

    /// Telegram's sign-in until it stands, then the chats.
    fn chats_view(&self, ctx: &Ctx) -> El<'_, Msg> {
        let client = &self.client;
        let content: El<'_, client::Msg> = match auth::view(client) {
            Some(board) => board,
            None => {
                let now = dates::now();
                let narrow = ctx.window.width - ctx.rail < TWO_PANES;
                let open = client.active_chat().map(|chat| pane::view(client, chat, narrow, now.date()));
                match (narrow, open) {
                    (true, Some(open)) => open,
                    (true, None) => list::view(client, now),
                    (false, open) => row![container(list::view(client, now)).width(list::WIDTH), ui::divider_v(), open.unwrap_or_else(pane::empty)].into(),
                }
            }
        };
        let page = container(content).width(Fill).height(Fill).style(theme::page);
        let tracked: El<'_, client::Msg> = pointer::pointer(page, client::Msg::Pointer).into();
        tracked.map(Msg::Chats)
    }

    pub fn view(&self, ctx: &Ctx) -> El<'_, Msg> {
        match &self.access {
            Access::Idle | Access::Checking => ui::page(ui::loading("Проверяем доступ...")),
            Access::Closed { title, body } => self.closed_view(title, body),
            Access::Gate => self.gate_view(),
            Access::Open => self.chats_view(ctx),
        }
    }

    /// What is drawn over the whole window: a mini app's frame, or the
    /// topmost menu or dialog of the chats.
    pub fn overlay(&self, ctx: &Ctx) -> Option<El<'_, Msg>> {
        if self.access != Access::Open {
            return None;
        }
        if let Some(frame) = self.mini.view(ctx) {
            return Some(frame.map(Msg::Mini));
        }
        layers::view(&self.client, ctx).map(|layer| layer.map(Msg::Chats))
    }

    /// Opens the first chat for a screenshot, the way a press on it would.
    #[cfg(feature = "devtools")]
    pub fn probe(name: &str) -> Option<Msg> {
        let inner = match name {
            "tg-select" => client::Msg::ToggleSelecting,
            "tg-period" => client::Msg::TogglePeriod,
            "tg-all" => client::Msg::Filter(client::Filter::All),
            _ => return name.strip_prefix("tg-chat:").and_then(|id| id.parse().ok()).map(|id| Msg::Chats(client::Msg::Open(id))),
        };
        Some(Msg::Chats(inner))
    }
}

fn centered(content: El<'_, Msg>) -> El<'_, Msg> {
    ui::page(container(content).width(Fill).height(Fill).center_x(Fill).center_y(Fill))
}

impl Drop for Telegram {
    fn drop(&mut self) {
        host::close_mini();
    }
}

#[cfg(test)]
mod tests;
