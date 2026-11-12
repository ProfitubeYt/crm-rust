//! The Telegram section. Chats stay the web page they always were, embedded in
//! a web view; this screen owns who may open it and carries the mini apps.

mod assets;
pub(crate) mod host;

use std::time::{Duration, Instant};

use iced::widget::{Space, column, container, row};
use iced::{Alignment, Fill, Subscription, Task, window};
use serde::Deserialize;
use tbs_core::api::{AccessScope, AccessStatus, ScopeCode, UnlockedScope};
use tbs_core::links;

use crate::ctx::{Ctx, Request, text_error};
use crate::theme::{Tone, space};
use crate::ui::{self, El, Icon, Kind};

use host::{Area, PageSpec};
pub use host::Signal;

/// A page nobody looks at is memory nobody uses.
const IDLE_LIFETIME: Duration = Duration::from_secs(10 * 60);
const PAGE_START_LIMIT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq)]
enum Access {
    Idle,
    Checking,
    Closed { title: &'static str, body: String },
    Gate,
    Open,
}

#[derive(Clone, Debug, PartialEq)]
enum Page {
    Absent,
    Starting(Instant),
    Ready,
    Failed(String),
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

struct Mini {
    generation: u64,
    area: Area,
    wanted: bool,
    attached: bool,
    loaded: bool,
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
    Attached(Result<(), String>),
    MiniAttached(u64, Result<(), String>),
    Signal(Signal),
    Asked(String, bool),
    Sweep,
}

/// What the page sends the shell.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type")]
enum PageMessage {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "open-external")]
    OpenExternal { url: String },
    #[serde(rename = "miniapp:open")]
    MiniOpen { url: String, rect: Area },
    #[serde(rename = "miniapp:bounds")]
    MiniBounds { rect: Area },
    #[serde(rename = "miniapp:visible")]
    MiniVisible { visible: bool },
    #[serde(rename = "miniapp:send")]
    MiniSend {
        #[serde(rename = "eventType")]
        event_type: String,
        #[serde(rename = "eventData", default)]
        event_data: serde_json::Value,
    },
    #[serde(rename = "miniapp:close")]
    MiniClose,
}

pub struct Telegram {
    access: Access,
    page: Page,
    mini: Option<Mini>,
    generation: u64,
    shown: bool,
    hidden_since: Option<Instant>,
    asking: bool,
    otp: Otp,
}

fn page_area(ctx: &Ctx) -> Area {
    Area { x: ctx.rail, y: 0.0, width: ctx.window.width - ctx.rail, height: ctx.window.height }
}

/// The page measures from its own corner, the window from the rail's.
fn in_window(rect: Area, ctx: &Ctx) -> Area {
    Area { x: rect.x + ctx.rail, ..rect }
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn deliver_script(event_type: &str, event_data: &serde_json::Value) -> String {
    format!(
        "try{{window.Telegram&&window.Telegram.WebView&&window.Telegram.WebView.receiveEvent({},{})}}catch(e){{}}",
        js_string(event_type),
        serde_json::to_string(event_data).unwrap_or_else(|_| "null".to_owned()),
    )
}

fn page_toast_kind(tone: Tone) -> &'static str {
    match tone {
        Tone::Success => "success",
        Tone::Danger => "error",
        Tone::Warning => "warning",
        _ => "info",
    }
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

impl Default for Telegram {
    fn default() -> Self {
        Self::new()
    }
}

impl Telegram {
    pub fn new() -> Self {
        Self { access: Access::Idle, page: Page::Absent, mini: None, generation: 0, shown: false, hidden_since: None, asking: false, otp: Otp::default() }
    }

    /// Called on every visit: resumes whatever the last one left unfinished.
    pub fn enter(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        match self.access {
            Access::Idle | Access::Closed { .. } => self.check(ctx),
            Access::Open if self.page == Page::Absent => self.attach(ctx),
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
        if self.page == Page::Absent { self.attach(ctx) } else { Task::none() }
    }

    fn attach(&mut self, ctx: &Ctx) -> Task<Msg> {
        if assets::ASSETS.is_empty() {
            self.page = Page::Failed("Веб-часть не встроена в эту сборку".to_owned());
            return Task::none();
        }
        self.page = Page::Starting(Instant::now());
        let spec = PageSpec { api: ctx.api.clone(), dir: ctx.data_dir.clone(), dark: ctx.dark, area: page_area(ctx) };
        Task::future(async { tokio::runtime::Handle::current() })
            .then(move |runtime| {
                let spec = spec.clone();
                window::oldest().then(move |id| {
                    let (spec, runtime) = (spec.clone(), runtime.clone());
                    match id {
                        Some(id) => window::run(id, move |w| host::create_page(w, spec, runtime)),
                        None => Task::done(Err("Окно приложения недоступно".to_owned())),
                    }
                })
            })
            .map(Msg::Attached)
    }

    fn visible(&self) -> bool {
        self.shown && self.access == Access::Open && self.page == Page::Ready
    }

    fn sync_visibility(&self) {
        let visible = self.visible();
        host::show_page(visible);
        host::show_mini(visible && self.mini.as_ref().is_some_and(|mini| mini.wanted && mini.loaded));
    }

    /// Follows the app's route. Web views float above everything drawn by the
    /// app, so one left visible would cover the next screen.
    pub fn set_shown(&mut self, shown: bool) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        self.hidden_since = (!shown).then(Instant::now);
        self.sync_visibility();
    }

    /// The window or the rail changed size: the page keeps to what both leave.
    pub fn resized(&self, ctx: &Ctx) {
        host::place_page(page_area(ctx));
        // The mini app is placed from the page's corner, which moves with the rail.
        if let Some(mini) = self.mini.as_ref().filter(|mini| mini.attached) {
            host::place_mini(in_window(mini.area, ctx));
        }
    }

    pub fn theme_changed(&self, ctx: &Ctx) {
        let theme = if ctx.dark { "dark" } else { "light" };
        host::run_in_page(&format!("window.__tbsShell&&window.__tbsShell.theme('{theme}')"));
    }

    /// Shows a toast inside the page when the page covers the app's own.
    pub fn toast(&self, tone: Tone, message: &str) -> bool {
        if !self.visible() {
            return false;
        }
        host::run_in_page(&format!("window.__tbsShell&&window.__tbsShell.toast('{}',{})", page_toast_kind(tone), js_string(message)));
        true
    }

    /// The shared access token ran out; the section closes until it is renewed.
    pub fn scope_expired(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        self.close_mini();
        self.access = Access::Idle;
        self.sync_visibility();
        if self.shown { self.check(ctx) } else { Task::none() }
    }

    fn close_mini(&mut self) {
        self.mini = None;
        host::close_mini();
    }

    fn drop_page(&mut self) {
        self.mini = None;
        self.page = Page::Absent;
        host::close_all();
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
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
                if let Page::Starting(since) = self.page
                    && since.elapsed() > PAGE_START_LIMIT
                {
                    self.drop_page();
                    self.page = Page::Failed("Страница Telegram не ответила".to_owned());
                }
            }
            Msg::Retry => {
                if matches!(self.page, Page::Failed(_)) {
                    self.drop_page();
                }
                return match self.access {
                    Access::Open => self.attach(ctx),
                    _ => self.check(ctx),
                };
            }
            Msg::Attached(Ok(())) => {
                // The window may have changed while the view was being created.
                self.resized(ctx);
            }
            Msg::Attached(Err(reason)) => {
                tracing::error!("telegram web view failed: {reason}");
                self.page = Page::Failed(format!("Не удалось запустить WebView2: {reason}"));
            }
            Msg::MiniAttached(generation, outcome) => {
                let current = self.mini.as_mut().filter(|mini| mini.generation == generation);
                match (current, outcome) {
                    (Some(mini), Ok(())) => {
                        mini.attached = true;
                        host::place_mini(in_window(mini.area, ctx));
                    }
                    (Some(_), Err(reason)) => {
                        tracing::warn!("mini app failed: {reason}");
                        self.close_mini();
                        ctx.toast(Tone::Danger, "Не удалось открыть мини-приложение");
                    }
                    (None, _) => {
                        if self.mini.is_none() {
                            host::close_mini();
                        }
                    }
                }
            }
            Msg::Signal(signal) => return self.on_signal(signal, ctx),
            Msg::Asked(url, yes) => {
                self.asking = false;
                if yes {
                    open_in_system(&url);
                }
            }
            Msg::Sweep => {
                let idle = self.hidden_since.is_some_and(|since| since.elapsed() > IDLE_LIFETIME);
                if idle && self.mini.is_none() && self.page != Page::Absent {
                    self.drop_page();
                }
            }
        }
        Task::none()
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

    fn on_signal(&mut self, signal: Signal, ctx: &mut Ctx) -> Task<Msg> {
        match signal {
            Signal::Page(raw) => match serde_json::from_str::<PageMessage>(&raw) {
                Ok(message) => return self.on_page_message(message, ctx),
                Err(error) => tracing::warn!("unreadable page message: {error}"),
            },
            Signal::Mini(raw) => host::run_in_page(&format!("window.__tbsMiniApp&&window.__tbsMiniApp.event({})", js_string(&raw))),
            Signal::MiniStarted => host::run_in_page("window.__tbsMiniApp&&window.__tbsMiniApp.navigated()"),
            Signal::MiniFinished => {
                if let Some(mini) = &mut self.mini {
                    mini.loaded = true;
                }
                self.sync_visibility();
                host::run_in_page("window.__tbsMiniApp&&window.__tbsMiniApp.loaded()");
            }
            Signal::External(url) => return self.ask_external(&url),
        }
        Task::none()
    }

    fn on_page_message(&mut self, message: PageMessage, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            PageMessage::Ready => {
                self.page = Page::Ready;
                self.sync_visibility();
                self.theme_changed(ctx);
            }
            PageMessage::OpenExternal { url } => return self.ask_external(&url),
            PageMessage::MiniOpen { url, rect } => return self.open_mini(url, rect, ctx),
            PageMessage::MiniBounds { rect } => {
                if let Some(mini) = &mut self.mini {
                    mini.area = rect;
                    host::place_mini(in_window(rect, ctx));
                }
            }
            PageMessage::MiniVisible { visible } => {
                if let Some(mini) = &mut self.mini {
                    mini.wanted = visible;
                }
                self.sync_visibility();
            }
            PageMessage::MiniSend { event_type, event_data } => {
                if self.mini.as_ref().is_some_and(|mini| mini.attached) {
                    host::run_in_mini(&deliver_script(&event_type, &event_data));
                }
            }
            PageMessage::MiniClose => self.close_mini(),
        }
        Task::none()
    }

    fn open_mini(&mut self, url: String, rect: Area, ctx: &mut Ctx) -> Task<Msg> {
        if !links::is_safe_mini_app_url(&url) {
            ctx.toast(Tone::Danger, "Адрес мини-приложения отклонён");
            return Task::none();
        }
        self.generation += 1;
        let generation = self.generation;
        self.mini = Some(Mini { generation, area: rect, wanted: true, attached: false, loaded: false });
        let (area, dir) = (in_window(rect, ctx), ctx.data_dir.clone());
        window::oldest()
            .then(move |id| {
                let (url, dir) = (url.clone(), dir.clone());
                match id {
                    Some(id) => window::run(id, move |w| host::create_mini(w, &url, area, dir)),
                    None => Task::done(Err("Окно приложения недоступно".to_owned())),
                }
            })
            .map(move |outcome| Msg::MiniAttached(generation, outcome))
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

    pub fn subscription(&self) -> Subscription<Msg> {
        let waiting = self.access == Access::Gate || matches!(self.page, Page::Starting(_));
        if waiting { iced::time::every(Duration::from_secs(1)).map(|_| Msg::Tick) } else { Subscription::none() }
    }

    /// Web views keep talking whichever section is open.
    pub fn background(&self) -> Subscription<Msg> {
        let signals = host::signals().map(Msg::Signal);
        if self.hidden_since.is_some() && self.page != Page::Absent {
            Subscription::batch([signals, iced::time::every(Duration::from_secs(60)).map(|_| Msg::Sweep)])
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

    pub fn view(&self, _ctx: &Ctx) -> El<'_, Msg> {
        match &self.access {
            Access::Idle | Access::Checking => ui::page(ui::loading("Проверяем доступ...")),
            Access::Closed { title, body } => self.closed_view(title, body),
            Access::Gate => self.gate_view(),
            Access::Open => match &self.page {
                Page::Absent | Page::Starting(_) => ui::page(ui::loading("Загрузка Telegram…")),
                Page::Failed(reason) => self.closed_view("Telegram недоступен", reason),
                Page::Ready => ui::page(Space::new().width(Fill).height(Fill)),
            },
        }
    }
}

fn centered(content: El<'_, Msg>) -> El<'_, Msg> {
    ui::page(container(content).width(Fill).height(Fill).center_x(Fill).center_y(Fill))
}

impl Drop for Telegram {
    fn drop(&mut self) {
        host::close_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tbs_core::model::UserInfo;

    fn ctx(dir: &tempfile::TempDir, admin: bool) -> Ctx {
        let mut ctx = Ctx::for_tests(dir.path());
        let role = if admin { "admin" } else { "user" };
        ctx.user = serde_json::from_value::<UserInfo>(serde_json::json!({ "id": 1, "username": "u", "role": role })).ok();
        ctx
    }

    fn scope(id: i64, active: bool, sources: bool) -> AccessScope {
        AccessScope { id, is_active: active, allow_sources: sources, ..AccessScope::default() }
    }

    fn ready() -> Telegram {
        let mut screen = Telegram::new();
        screen.access = Access::Open;
        screen.page = Page::Ready;
        screen.set_shown(true);
        screen
    }

    #[test]
    fn page_messages_read_as_the_page_writes_them() {
        let read = |raw: &str| serde_json::from_str::<PageMessage>(raw).unwrap();
        assert_eq!(read(r#"{"type":"ready"}"#), PageMessage::Ready);
        assert_eq!(read(r#"{"type":"open-external","url":"https://t.me/a"}"#), PageMessage::OpenExternal { url: "https://t.me/a".into() });
        assert_eq!(
            read(r#"{"type":"miniapp:open","url":"https://a.b/","rect":{"x":1,"y":2,"width":3,"height":4}}"#),
            PageMessage::MiniOpen { url: "https://a.b/".into(), rect: Area { x: 1.0, y: 2.0, width: 3.0, height: 4.0 } },
        );
        assert_eq!(
            read(r#"{"type":"miniapp:send","eventType":"theme_changed","eventData":{"a":1}}"#),
            PageMessage::MiniSend { event_type: "theme_changed".into(), event_data: serde_json::json!({ "a": 1 }) },
        );
        assert!(serde_json::from_str::<PageMessage>(r#"{"type":"run-anything"}"#).is_err());
    }

    #[test]
    fn scripts_cannot_be_broken_out_of() {
        let script = deliver_script("x');alert(1);//", &serde_json::json!({ "a": "\"</script>" }));
        assert!(script.contains(r#"receiveEvent("x');alert(1);//","#));
        assert!(script.contains(r#"{"a":"\"</script>"}"#));
        assert_eq!(js_string("a\"\nb"), r#""a\"\nb""#);
    }

    #[test]
    fn an_admin_goes_straight_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, true);
        let mut screen = Telegram::new();
        let _ = screen.enter(&mut ctx);
        assert_eq!(screen.access, Access::Open);
    }

    #[test]
    fn a_user_is_checked_first_and_stays_out_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, false);
        let mut screen = Telegram::new();
        let _ = screen.enter(&mut ctx);
        assert_eq!(screen.access, Access::Checking);
        let _ = screen.update(Msg::Status(Err("нет сети".into())), &mut ctx);
        assert!(matches!(screen.access, Access::Closed { title: "Доступ временно ограничен", .. }));
        assert!(!screen.visible());
    }

    #[test]
    fn no_scopes_means_nothing_to_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, false);
        let mut screen = Telegram::new();
        let _ = screen.update(Msg::Scopes(Ok(vec![scope(1, false, true), scope(2, true, false)])), &mut ctx);
        assert!(matches!(screen.access, Access::Closed { title: "Доступ закрыт", .. }));
    }

    #[test]
    fn the_first_scope_with_sources_is_unlocked_and_asks_for_a_code_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, false);
        let mut screen = Telegram::new();
        let _ = screen.update(Msg::Scopes(Ok(vec![scope(1, true, false), scope(7, true, true)])), &mut ctx);
        assert_eq!(screen.access, Access::Gate);
        assert_eq!(screen.otp.scope, 7);
        assert!(screen.otp.requesting);
        let _ = screen.update(Msg::Requested(Ok(ScopeCode { expires_in: 120, code_length: 6, ..ScopeCode::default() })), &mut ctx);
        assert!(!screen.otp.requesting);
        assert_eq!(screen.otp.seconds_left, 120);
    }

    #[test]
    fn a_code_takes_digits_only_and_a_short_one_is_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, false);
        let mut screen = Telegram::new();
        screen.access = Access::Gate;
        let _ = screen.update(Msg::Code("12a3".into()), &mut ctx);
        assert_eq!(screen.otp.code, "123");
        let _ = screen.update(Msg::Verify, &mut ctx);
        assert!(!screen.otp.verifying);
        let _ = screen.update(Msg::Code("12345678".into()), &mut ctx);
        assert_eq!(screen.otp.code, "123456");
        let _ = screen.update(Msg::Verify, &mut ctx);
        assert!(screen.otp.verifying);
        let _ = screen.update(Msg::Verified(Err("неверный код".into())), &mut ctx);
        assert_eq!(screen.access, Access::Gate);
        assert_eq!(screen.otp.error.as_deref(), Some("неверный код"));
    }

    #[test]
    fn a_verified_code_opens_the_section_and_refreshes_the_table() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, false);
        let mut screen = Telegram::new();
        screen.access = Access::Gate;
        let _ = screen.update(Msg::Verified(Ok(UnlockedScope::default())), &mut ctx);
        assert_eq!(screen.access, Access::Open);
        assert!(ctx.requests.iter().any(|r| matches!(r, Request::TransactionsChanged)));
    }

    #[test]
    fn a_skipped_otp_opens_without_a_form() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, false);
        let mut screen = Telegram::new();
        screen.access = Access::Gate;
        let code = ScopeCode { otp_skipped: true, token: Some("t".into()), ..ScopeCode::default() };
        let _ = screen.update(Msg::Requested(Ok(code)), &mut ctx);
        assert_eq!(screen.access, Access::Open);
    }

    #[test]
    fn an_expired_scope_closes_the_section_while_it_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, false);
        let mut screen = ready();
        assert!(screen.visible());
        let _ = screen.scope_expired(&mut ctx);
        assert_eq!(screen.access, Access::Checking);
        assert!(!screen.visible());
    }

    #[test]
    fn leaving_the_section_hides_it_and_toasts_stay_with_the_app() {
        let mut screen = ready();
        assert!(screen.toast(Tone::Success, "ok"));
        screen.set_shown(false);
        assert!(!screen.visible());
        assert!(!screen.toast(Tone::Success, "ok"));
        assert!(screen.hidden_since.is_some());
        screen.set_shown(true);
        assert!(screen.hidden_since.is_none());
    }

    #[test]
    fn a_long_hidden_page_is_released_unless_a_mini_app_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, true);
        let mut screen = ready();
        screen.set_shown(false);
        let long_ago = Instant::now().checked_sub(IDLE_LIFETIME + Duration::from_secs(1));
        screen.hidden_since = long_ago;
        screen.mini = Some(Mini { generation: 1, area: Area::default(), wanted: true, attached: true, loaded: true });
        let _ = screen.update(Msg::Sweep, &mut ctx);
        assert_eq!(screen.page, Page::Ready);
        screen.mini = None;
        let _ = screen.update(Msg::Sweep, &mut ctx);
        assert_eq!(screen.page, Page::Absent);
        let _ = screen.enter(&mut ctx);
        assert!(!assets::ASSETS.is_empty() && matches!(screen.page, Page::Starting(_)) || matches!(screen.page, Page::Failed(_)));
    }

    #[test]
    fn a_stalled_page_is_reported_not_waited_on_forever() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, true);
        let mut screen = Telegram::new();
        screen.access = Access::Open;
        screen.page = Page::Starting(Instant::now().checked_sub(PAGE_START_LIMIT + Duration::from_secs(1)).unwrap());
        let _ = screen.update(Msg::Tick, &mut ctx);
        assert!(matches!(screen.page, Page::Failed(_)));
    }

    #[test]
    fn unsafe_mini_apps_never_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, true);
        let mut screen = ready();
        let message = serde_json::json!({ "type": "miniapp:open", "url": "http://evil.example.com/", "rect": { "x": 0, "y": 0, "width": 9, "height": 9 } });
        let _ = screen.update(Msg::Signal(Signal::Page(message.to_string())), &mut ctx);
        assert!(screen.mini.is_none());
        assert_eq!(ctx.toasts.len(), 1);
    }

    #[test]
    fn a_stale_mini_app_does_not_take_over_the_current_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, true);
        let mut screen = ready();
        screen.mini = Some(Mini { generation: 2, area: Area::default(), wanted: true, attached: false, loaded: false });
        let _ = screen.update(Msg::MiniAttached(1, Ok(())), &mut ctx);
        assert!(!screen.mini.as_ref().unwrap().attached);
        let _ = screen.update(Msg::MiniAttached(2, Ok(())), &mut ctx);
        assert!(screen.mini.as_ref().unwrap().attached);
    }

    #[test]
    fn only_allowed_links_raise_a_question() {
        let mut screen = ready();
        let _ = screen.ask_external("https://evil.example.com/");
        assert!(!screen.asking);
        let _ = screen.ask_external("https://t.me/x");
        assert!(screen.asking);
    }
}
