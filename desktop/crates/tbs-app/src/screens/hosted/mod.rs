//! A workspace that is a web page: Time's own interface, the CRM
//! administration panel. The page is compiled in and shown in a web view. It
//! holds no credential: its requests reach one part of the API through the
//! shell, on the session the start window signed in, and the server judges
//! each of them by the account again.

mod assets;
mod host;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::widget::{Space, column, container, row};
use iced::{Alignment, Fill, Subscription, Task, window};
use serde::Deserialize;
use tbs_core::api::is_month;

use crate::ctx::{Ctx, Request, text_error};
use crate::files;
use crate::screens::telegram::host::Area;
use crate::screens::telegram::open_in_system;
use crate::theme::{Tone, space};
use crate::ui::{self, El, Icon, Kind as Look};

pub use assets::Kind;
pub use host::Signal;
use host::Owner;

const PAGE_START_LIMIT: Duration = Duration::from_secs(20);
/// The engine lets go of its files a moment after its view is closed.
const DISCARD_TRIES: u32 = 20;
const DISCARD_PAUSE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq)]
enum Page {
    Absent,
    Starting(Instant),
    Ready,
    Failed(String),
}

#[derive(Clone, Debug)]
pub enum Msg {
    Attached(Owner, Result<(), String>),
    Signal(Signal),
    Tick,
    Retry,
    /// The workbook, for the opening of the page that asked for it.
    Fetched(Owner, Result<(String, Vec<u8>), String>),
    Saved(Result<Option<String>, String>),
    /// The panel as the server itself serves it, in the system's browser.
    Browser,
}

/// What a page tells the shell. It asks for nothing else, and nothing else is read.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type")]
enum PageMessage {
    /// The server refused the page: the session or the access is gone.
    #[serde(rename = "access-lost")]
    AccessLost,
    /// Time: this month's workbook is wanted. The page has no disk to save it to.
    #[serde(rename = "export")]
    Export { ym: String },
}

pub struct Hosted {
    kind: Kind,
    /// The workspace's name, as the start window calls it.
    name: &'static str,
    page: Page,
    /// The opening of the page its web view was made for; 0 before the first.
    owner: Owner,
    shown: bool,
    saving: bool,
    forget: bool,
}

fn page_area(ctx: &Ctx) -> Area {
    Area { x: ctx.rail, y: 0.0, width: ctx.window.width - ctx.rail, height: ctx.window.height }
}

/// A folder of the web view per page, account and server: what one account's
/// page kept is not there for the next account to find.
fn web_dir(ctx: &Ctx, kind: Kind) -> PathBuf {
    ctx.data_dir.join("web").join(format!("{}-{}-{}", kind.scheme(), ctx.user_key(), ctx.host_key()))
}

/// The folder of one page for the account that is signed in.
pub fn folder(ctx: &Ctx, kind: Kind) -> PathBuf {
    web_dir(ctx, kind)
}

/// The folders of every page for the account that is signed in.
pub fn folders(ctx: &Ctx) -> Vec<PathBuf> {
    [Kind::Time, Kind::Admin].into_iter().map(|kind| web_dir(ctx, kind)).collect()
}

/// Removes what the web views kept in these folders: the account signed out
/// or lost the page. A folder the engine still holds is tried again for a few
/// seconds, away from the interface.
pub fn discard(folders: Vec<PathBuf>) {
    let folders: Vec<PathBuf> = folders.into_iter().filter(|folder| folder.exists()).collect();
    if folders.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for folder in folders {
            for tries_left in (0..DISCARD_TRIES).rev() {
                match std::fs::remove_dir_all(&folder) {
                    Ok(()) => break,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) if tries_left == 0 => tracing::warn!("what a page kept was not removed: {error}"),
                    Err(_) => std::thread::sleep(DISCARD_PAUSE),
                }
            }
        }
    });
}

impl Hosted {
    pub fn new(kind: Kind, name: &'static str) -> Self {
        Self { kind, name, page: Page::Absent, owner: 0, shown: false, saving: false, forget: false }
    }

    /// Starts the page once. The work window has to be up: a web view is a
    /// child of the window it was made in.
    pub fn enter(&mut self, ctx: &mut Ctx) -> Task<Msg> {
        if self.page == Page::Absent { self.attach(ctx) } else { Task::none() }
    }

    fn attach(&mut self, ctx: &Ctx) -> Task<Msg> {
        if self.kind.files().is_empty() {
            self.page = Page::Failed("Страница не встроена в эту сборку".to_owned());
            return Task::none();
        }
        self.page = Page::Starting(Instant::now());
        self.owner = host::claim();
        let owner = self.owner;
        let spec = host::Spec { kind: self.kind, owner, api: ctx.api.clone(), dir: web_dir(ctx, self.kind), dark: ctx.dark, area: page_area(ctx) };
        Task::future(async { tokio::runtime::Handle::current() })
            .then(move |runtime| {
                let spec = spec.clone();
                window::oldest().then(move |id| {
                    let (spec, runtime) = (spec.clone(), runtime.clone());
                    match id {
                        Some(id) => window::run(id, move |w| host::create(w, spec, runtime)),
                        None => Task::done(Err("Окно приложения недоступно".to_owned())),
                    }
                })
            })
            .map(move |made| Msg::Attached(owner, made))
    }

    fn visible(&self) -> bool {
        self.shown && self.page == Page::Ready
    }

    /// The web view floats above everything the app draws, so it follows
    /// every change of screen: the launch gate, the way back to the start window.
    pub fn set_shown(&mut self, shown: bool) {
        if self.shown != shown {
            self.shown = shown;
            host::show(self.owner, self.visible());
        }
    }

    /// The window or the rail changed size: the page keeps to what both leave.
    pub fn resized(&self, ctx: &Ctx) {
        host::place(self.owner, page_area(ctx));
    }

    pub fn theme_changed(&self, ctx: &Ctx) {
        host::run(self.owner, &self.kind.theme_script(ctx.dark));
    }

    /// Shows a toast inside the page when the page covers the app's own.
    pub fn toast(&self, tone: Tone, message: &str) -> bool {
        if !self.visible() {
            return false;
        }
        host::run(self.owner, &self.kind.toast_script(message, matches!(tone, Tone::Danger | Tone::Warning)));
        true
    }

    /// The server took back a refusal the page ran into, as the launch
    /// password does: the page starts over on the session as it is now.
    pub fn reload(&mut self) {
        if self.page != Page::Absent {
            self.page = Page::Starting(Instant::now());
            host::show(self.owner, false);
            host::reload(self.owner);
        }
    }

    /// The account signed out or lost this access: what the web view kept
    /// for its page goes when the page is closed.
    pub fn forget_on_close(&mut self) {
        self.forget = true;
    }

    #[cfg(feature = "devtools")]
    pub fn is_ready(&self) -> bool {
        matches!(self.page, Page::Ready | Page::Failed(_))
    }

    /// The page has been asked for: there is, or was, a web view for it.
    #[cfg(test)]
    pub fn started(&self) -> bool {
        self.page != Page::Absent
    }

    #[cfg(test)]
    pub fn owner(&self) -> Owner {
        self.owner
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            // The window may have changed while the view was being created.
            Msg::Attached(owner, Ok(())) if owner == self.owner => self.resized(ctx),
            Msg::Attached(owner, Err(reason)) if owner == self.owner => {
                tracing::error!("hosted page web view failed: {reason}");
                self.page = Page::Failed(format!("Не удалось запустить WebView2: {reason}"));
            }
            // Of an opening that has since been closed or tried again.
            Msg::Attached(..) => {}
            Msg::Signal(Signal::Loaded(owner)) if owner == self.owner => {
                if matches!(self.page, Page::Starting(_)) {
                    self.page = Page::Ready;
                    host::show(self.owner, self.visible());
                    self.theme_changed(ctx);
                }
            }
            Msg::Signal(Signal::Said(owner, raw)) if owner == self.owner => match serde_json::from_str::<PageMessage>(&raw) {
                // Whatever was refused, the server's own answer has told the
                // shell already; this asks what the account may still open.
                Ok(PageMessage::AccessLost) => ctx.request(Request::AccessChanged),
                Ok(PageMessage::Export { ym }) => return self.export(ym, ctx),
                Err(error) => tracing::warn!("unreadable page message: {error}"),
            },
            // From a page that has since been closed.
            Msg::Signal(_) => {}
            Msg::Tick => {
                if let Page::Starting(since) = self.page
                    && since.elapsed() > PAGE_START_LIMIT
                {
                    host::close(self.owner, false);
                    self.page = Page::Failed("Страница не ответила".to_owned());
                }
            }
            Msg::Retry => {
                host::close(self.owner, false);
                self.page = Page::Absent;
                return self.attach(ctx);
            }
            // Offered to whoever asked: a page closed or opened again since
            // then belongs to another moment, perhaps to another account.
            Msg::Fetched(owner, fetched) if owner == self.owner => match fetched {
                Ok((name, bytes)) => return Task::perform(files::save_as(name, bytes), Msg::Saved),
                Err(reason) => {
                    self.saving = false;
                    ctx.toast(Tone::Danger, reason);
                }
            },
            Msg::Fetched(..) => {}
            Msg::Saved(outcome) => {
                self.saving = false;
                match outcome {
                    Ok(Some(path)) => ctx.toast(Tone::Success, format!("Файл сохранён: {path}")),
                    Ok(None) => {}
                    Err(reason) => ctx.toast(Tone::Danger, reason),
                }
            }
            Msg::Browser => open_in_system(&format!("{}/admin/", ctx.api.base())),
        }
        Task::none()
    }

    /// The workbook is fetched on the shell's session and saved where the
    /// person points. Only Time has one, and only a month names it.
    fn export(&mut self, month: String, ctx: &Ctx) -> Task<Msg> {
        if self.kind != Kind::Time || self.saving || !is_month(&month) {
            return Task::none();
        }
        self.saving = true;
        let (api, owner) = (ctx.api.clone(), self.owner);
        Task::perform(
            async move {
                let (name, bytes) = api.time_export(&month).await.map_err(text_error)?;
                Ok((name.unwrap_or_else(|| format!("TimeList-{month}.xlsx")), bytes))
            },
            move |fetched| Msg::Fetched(owner, fetched),
        )
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        if matches!(self.page, Page::Starting(_)) { iced::time::every(Duration::from_secs(1)).map(|_| Msg::Tick) } else { Subscription::none() }
    }

    /// The page keeps talking whatever stands in front of it.
    pub fn background(&self) -> Subscription<Msg> {
        host::signals().map(Msg::Signal)
    }

    fn failed_view<'a>(&'a self, reason: &'a str) -> El<'a, Msg> {
        let mut buttons = row![ui::btn("Повторить", Look::Primary, Some(Msg::Retry))].spacing(space::SM);
        if self.kind == Kind::Admin {
            // The same panel, as the server serves it; it asks for the sign-in itself.
            buttons = buttons.push(ui::btn("Открыть в браузере", Look::Secondary, Some(Msg::Browser)));
        }
        let content = column![
            row![ui::icon_tone(Icon::Warning, 16.0, Tone::Warning), ui::strong(format!("Раздел «{}» не открылся", self.name))].spacing(space::SM).align_y(Alignment::Center),
            ui::muted(reason),
            buttons,
        ]
        .spacing(space::MD);
        ui::page(container(ui::card(content).max_width(480)).width(Fill).height(Fill).center_x(Fill).center_y(Fill))
    }

    pub fn view(&self, _ctx: &Ctx) -> El<'_, Msg> {
        match &self.page {
            Page::Absent | Page::Starting(_) => ui::page(ui::loading("Загрузка…")),
            Page::Failed(reason) => self.failed_view(reason),
            // The web view covers this part of the window.
            Page::Ready => ui::page(Space::new().width(Fill).height(Fill)),
        }
    }
}

impl Drop for Hosted {
    fn drop(&mut self) {
        host::close(self.owner, self.forget);
    }
}

#[cfg(test)]
mod tests {
    use iced_test::Simulator;
    use iced_test::core::Settings;
    use tbs_core::model::UserInfo;

    use super::*;

    fn ctx(dir: &tempfile::TempDir, id: i64) -> Ctx {
        let mut ctx = Ctx::for_tests(dir.path());
        ctx.user = Some(UserInfo { id, username: "anna".into(), ..Default::default() });
        ctx
    }

    fn said(owner: Owner, raw: &str) -> Msg {
        Msg::Signal(Signal::Said(owner, raw.to_owned()))
    }

    fn gone(folder: &std::path::Path) -> bool {
        for _ in 0..100 {
            if !folder.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn sees(screen: &Hosted, ctx: &Ctx, wanted: &str) -> bool {
        let mut sim = Simulator::with_size(Settings::default(), iced::Size::new(1100.0, 700.0), screen.view(ctx));
        sim.find(wanted).is_ok()
    }

    #[test]
    fn page_messages_read_as_the_pages_write_them() {
        let read = |raw: &str| serde_json::from_str::<PageMessage>(raw);
        assert_eq!(read(r#"{"type":"access-lost"}"#).unwrap(), PageMessage::AccessLost);
        assert_eq!(read(r#"{"type":"export","ym":"2026-11"}"#).unwrap(), PageMessage::Export { ym: "2026-11".into() });
        // Nothing general-purpose: a page cannot ask the shell to run, open or fetch anything.
        for other in [r#"{"type":"run-anything"}"#, r#"{"type":"open-external","url":"https://example.com"}"#, r#"{"type":"fetch","path":"/api/auth/me"}"#, r#"{"type":"export"}"#, "export"] {
            assert!(read(other).is_err(), "{other}");
        }
    }

    #[test]
    fn the_page_starts_once_and_shows_only_when_it_has_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, 7);
        let mut screen = Hosted::new(Kind::Time, "Учёт времени");
        assert!(sees(&screen, &ctx, "Загрузка…"));
        let _ = screen.enter(&mut ctx);
        assert!(matches!(screen.page, Page::Starting(_)));
        screen.set_shown(true);
        assert!(!screen.visible(), "a page that has not loaded is not put over the window");

        // News of another opening, a page closed a moment ago, is not this one's.
        let _ = screen.update(Msg::Signal(Signal::Loaded(screen.owner + 1)), &mut ctx);
        let _ = screen.update(Msg::Signal(Signal::Loaded(0)), &mut ctx);
        let _ = screen.update(Msg::Attached(screen.owner + 1, Err("поздно".into())), &mut ctx);
        assert!(matches!(screen.page, Page::Starting(_)));
        let _ = screen.update(Msg::Signal(Signal::Loaded(screen.owner)), &mut ctx);
        assert_eq!(screen.page, Page::Ready);
        assert!(screen.visible());
        let _ = screen.enter(&mut ctx);
        assert_eq!(screen.page, Page::Ready, "coming back to it does not start it again");

        screen.set_shown(false);
        assert!(!screen.visible() && !screen.toast(Tone::Success, "ok"));
        screen.set_shown(true);
        assert!(screen.toast(Tone::Success, "ok"));
    }

    #[test]
    fn a_page_that_does_not_start_is_reported_and_can_be_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, 7);
        let mut screen = Hosted::new(Kind::Admin, "Администрирование CRM");
        let _ = screen.update(Msg::Attached(screen.owner, Err("нет среды выполнения".into())), &mut ctx);
        assert!(sees(&screen, &ctx, "Раздел «Администрирование CRM» не открылся"));
        assert!(sees(&screen, &ctx, "Повторить") && sees(&screen, &ctx, "Открыть в браузере"));
        let before = screen.owner;
        let _ = screen.update(Msg::Retry, &mut ctx);
        assert!(matches!(screen.page, Page::Starting(_)));
        // The new try is a new opening: the first one's late load is not its own.
        assert_ne!(screen.owner, before);
        let _ = screen.update(Msg::Signal(Signal::Loaded(before)), &mut ctx);
        assert!(matches!(screen.page, Page::Starting(_)));

        screen.page = Page::Starting(Instant::now().checked_sub(PAGE_START_LIMIT + Duration::from_secs(1)).unwrap());
        let _ = screen.update(Msg::Tick, &mut ctx);
        assert!(matches!(screen.page, Page::Failed(_)));

        // Time has no page on the server to fall back to: it is never published.
        let mut time = Hosted::new(Kind::Time, "Учёт времени");
        let _ = time.update(Msg::Attached(time.owner, Err("x".into())), &mut ctx);
        assert!(sees(&time, &ctx, "Повторить") && !sees(&time, &ctx, "Открыть в браузере"));
    }

    #[test]
    fn a_refused_page_asks_the_shell_what_the_account_may_still_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, 7);
        let mut screen = Hosted::new(Kind::Time, "Учёт времени");
        let _ = screen.update(said(screen.owner, r#"{"type":"access-lost"}"#), &mut ctx);
        assert!(matches!(ctx.requests.as_slice(), [Request::AccessChanged]), "{:?}", ctx.requests);
        // The page decides nothing here: the workspace is closed by the shell, on the server's word.
        assert!(ctx.user.is_some());
    }

    #[test]
    fn only_time_saves_a_workbook_and_only_for_a_month() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir, 7);
        let mut time = Hosted::new(Kind::Time, "Учёт времени");
        for wrong in [r#"{"type":"export","ym":"../../auth/me"}"#, r#"{"type":"export","ym":"2026-11/../x"}"#, r#"{"type":"export","ym":""}"#] {
            let _ = time.update(said(time.owner, wrong), &mut ctx);
            assert!(!time.saving, "{wrong}");
        }
        let _ = time.update(said(time.owner, r#"{"type":"export","ym":"2026-11"}"#), &mut ctx);
        assert!(time.saving);
        // A workbook fetched for a page that has been closed since is offered to nobody.
        let _ = time.update(Msg::Fetched(time.owner + 1, Ok(("TimeList-2026-11.xlsx".into(), vec![1]))), &mut ctx);
        let _ = time.update(Msg::Fetched(time.owner + 1, Err("чужой ответ".into())), &mut ctx);
        assert!(time.saving && ctx.toasts.is_empty());
        // One at a time; the answer frees the next.
        let _ = time.update(Msg::Saved(Ok(Some("C:/x/TimeList-2026-11.xlsx".into()))), &mut ctx);
        assert!(!time.saving);
        assert!(ctx.toasts.iter().any(|(_, text)| text.contains("TimeList-2026-11.xlsx")));
        let _ = time.update(Msg::Saved(Err("Сервис учёта времени не отвечает".into())), &mut ctx);
        assert!(ctx.toasts.iter().any(|(tone, _)| *tone == Tone::Danger));
        // A workbook the server refused frees the next as well.
        let _ = time.update(said(time.owner, r#"{"type":"export","ym":"2026-12"}"#), &mut ctx);
        let _ = time.update(Msg::Fetched(time.owner, Err("Нет доступа".into())), &mut ctx);
        assert!(!time.saving && ctx.toasts.iter().any(|(_, text)| text == "Нет доступа"));

        let mut admin = Hosted::new(Kind::Admin, "Администрирование CRM");
        let _ = admin.update(said(admin.owner, r#"{"type":"export","ym":"2026-11"}"#), &mut ctx);
        assert!(!admin.saving, "the panel has nothing to save");
        // A message of another opening is not acted on.
        let _ = admin.update(said(admin.owner + 1, r#"{"type":"access-lost"}"#), &mut ctx);
        assert!(ctx.requests.is_empty());
    }

    #[test]
    fn each_account_and_each_page_keeps_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (anna, boris) = (ctx(&dir, 7), ctx(&dir, 8));
        let folders = [web_dir(&anna, Kind::Time), web_dir(&anna, Kind::Admin), web_dir(&boris, Kind::Time), web_dir(&boris, Kind::Admin)];
        for (at, folder) in folders.iter().enumerate() {
            assert!(folder.starts_with(dir.path().join("web")));
            assert!(folders.iter().skip(at + 1).all(|other| other != folder), "{folder:?} is shared");
        }
        // Nor is it the folder the Telegram page keeps.
        assert!(folders.iter().all(|folder| *folder != dir.path().join("webview")));
    }

    #[test]
    fn what_the_pages_kept_is_removed_for_the_account_and_for_nobody_else() {
        let dir = tempfile::tempdir().unwrap();
        let (anna, boris) = (ctx(&dir, 7), ctx(&dir, 8));
        for folder in folders(&anna).into_iter().chain(folders(&boris)) {
            std::fs::create_dir_all(folder.join("Default")).unwrap();
            std::fs::write(folder.join("Default").join("Preferences"), b"{}").unwrap();
        }
        assert_eq!(folders(&anna), [folder(&anna, Kind::Time), folder(&anna, Kind::Admin)]);
        discard(folders(&anna));
        assert!(folders(&anna).iter().all(|folder| gone(folder)));
        assert!(folders(&boris).iter().all(|folder| folder.join("Default").join("Preferences").exists()));
        // Nothing there: nothing to do, and no harm in asking.
        discard(folders(&anna));
        discard(Vec::new());
    }
}
