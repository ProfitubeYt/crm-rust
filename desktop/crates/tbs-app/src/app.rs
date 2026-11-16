//! The application shell: the start window every launch opens on, the one
//! session it and the workspaces share, navigation and the things every screen
//! shares.

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use iced::keyboard::{self, Key, key::Named};
use iced::theme::Mode;
use iced::widget::{Stack, button, column, container, row, text};
use iced::{Alignment, Event, Fill, Length, Size, Subscription, Task, Theme, event, window};
use tbs_core::api::{Api, ApiEvent};
use tbs_core::config::{self, Preferences};
use tbs_core::entitlement::{CachedEntitlement, Verdict};
use tbs_core::model::{UserInfo, WORKSPACE_CHECKS};
use tbs_core::secret::{KeyringStore, SecretStore};
use tbs_core::store::{self, Store};

use crate::ctx::{Ctx, Request, SyncState, text_error};
use crate::focus;
use crate::screens::gate::{self, LaunchGate};
use crate::screens::audit::{self, Audit};
use crate::screens::hosted::{self, Hosted};
use crate::screens::login::{self, Login, Subsystem};
use crate::screens::logs::{self, Logs};
use crate::screens::reference::{self, Reference};
use crate::screens::settings::{self, Settings};
use crate::screens::telegram::{self, Telegram};
use crate::screens::transactions::{self, Transactions};
use crate::theme::{self, Tone, size, space};
use crate::ui::{self, El, Icon};

/// What the program calls itself: the window title and its dialogs. A
/// workspace opened from the start window goes by its own name: Checks in
/// its rail, a workspace that is a page in its bar.
pub const PRODUCT: &str = "TBS inform CRM";
/// The way from a workspace back to the start window, session kept.
const HUB: &str = "Главный экран";
const HUB_HINT: &str = "К выбору рабочего пространства, без выхода из учётной записи";
const PING_EVERY: Duration = Duration::from_secs(30);
const TOAST_LIFE: Duration = Duration::from_secs(5);
pub const RAIL_WIDTH: f32 = 208.0;
/// A line of the rail, and the square its icon keeps once the rail is folded.
const RAIL_ITEM: f32 = 36.0;
const RAIL_PAD: f32 = 10.0;
/// The rail folded down to its icons.
const RAIL_FOLDED: f32 = RAIL_ITEM + 2.0 * RAIL_PAD;
/// The line under the rail's head: «РАЗДЕЛЫ», or a rule once folded.
const RAIL_CAPTION: f32 = 24.0;
/// The one line a workspace that is a page has instead of the rail, and the
/// rule `ui::divider` draws under it. The page starts below both.
const BAR_HEIGHT: f32 = 40.0;
const BAR_RULE: f32 = 1.0;
pub const BAR: f32 = BAR_HEIGHT + BAR_RULE;
/// The most of the bar an account's name may take.
const BAR_NAME: f32 = 240.0;
const TOAST_MIN: f32 = 160.0;
const TOAST_MAX: f32 = 420.0;
/// The Excel sheet, which sets how far from the right a toast has to end.
const WIDEST_SHEET: f32 = 620.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Transactions,
    Reference,
    Telegram,
    Logs,
    Settings,
    Audit,
}

impl Route {
    const ALL: [Route; 6] = [Route::Transactions, Route::Reference, Route::Telegram, Route::Logs, Route::Settings, Route::Audit];

    fn title(self) -> &'static str {
        match self {
            Route::Transactions => "Транзакции",
            Route::Reference => "Справочник",
            Route::Telegram => "Telegram",
            Route::Logs => "Логи",
            Route::Settings => "Настройки",
            Route::Audit => "Аудит",
        }
    }

    /// The rail's glyph: filled for the open section, the way Windows marks it.
    fn glyph(self, open: bool) -> Icon {
        match (self, open) {
            (Route::Transactions, false) => Icon::Table20,
            (Route::Transactions, true) => Icon::Table20Filled,
            (Route::Reference, false) => Icon::Book20,
            (Route::Reference, true) => Icon::Book20Filled,
            (Route::Telegram, false) => Icon::Send20,
            (Route::Telegram, true) => Icon::Send20Filled,
            (Route::Logs, false) => Icon::Logs20,
            (Route::Logs, true) => Icon::Logs20Filled,
            (Route::Settings, false) => Icon::Settings20,
            (Route::Settings, true) => Icon::Settings20Filled,
            (Route::Audit, false) => Icon::Shield20,
            (Route::Audit, true) => Icon::Shield20Filled,
        }
    }

    fn tab(self) -> Option<&'static str> {
        match self {
            Route::Transactions => Some("dashboard"),
            Route::Reference => Some("reference"),
            Route::Telegram => Some("userbot"),
            Route::Logs => Some("logs"),
            Route::Audit => Some("admin"),
            Route::Settings => None,
        }
    }

    fn open_to(self, user: &UserInfo) -> bool {
        user.is_admin() || self.tab().is_none_or(|tab| user.has_tab(tab))
    }
}

const WORK_WINDOW: Size = Size::new(1360.0, 860.0);
const WORK_MIN_WINDOW: Size = Size::new(1024.0, 640.0);
/// The title bar and the taskbar, which a monitor's size still counts.
const SCREEN_CHROME: f32 = 80.0;

enum Phase {
    /// The start window. Every launch opens here, and a workspace comes back
    /// here: signed in, signed out, or still asking the server which.
    Login(Login),
    Checking,
    Blocked { expired_at: Option<String>, busy: bool },
    Launch(LaunchGate),
    Main,
}

#[derive(Clone, Debug)]
pub enum Entitled {
    Allowed,
    Blocked(Option<String>),
}

struct Toast {
    id: u64,
    tone: Tone,
    message: String,
}

#[derive(Clone, Debug)]
pub enum Message {
    Boot(Result<Option<UserInfo>, String>),
    Entitled(Entitled),
    RetryEntitlement,
    Login(login::Msg),
    Launch(gate::Msg),
    Nav(Route),
    /// Back to the start window. The session stays as it is.
    Hub,
    /// Folds the rail down to its icons, or opens it again.
    ToggleRail,
    Tx(transactions::Msg),
    Reference(reference::Msg),
    Logs(logs::Msg),
    Audit(audit::Msg),
    Settings(settings::Msg),
    Telegram(telegram::Msg),
    /// The page of a workspace that is one: Time, CRM administration.
    Hosted(hosted::Msg),
    /// What the server says of the account now: what it may open.
    Account(u64, Result<UserInfo, String>),
    Resized(iced::Size),
    /// The other window is up: `true` for the small one of the start window.
    WindowSwapped(bool),
    Tab { back: bool },
    Ping(bool),
    PingTick,
    Api(ApiEvent),
    SystemTheme(Mode),
    DismissToast(u64),
    Logout,
    LoggedOut,
    #[cfg(feature = "devtools")]
    Dev(crate::devtools::Msg),
}

pub struct App {
    ctx: Ctx,
    phase: Phase,
    /// The window on screen is the small one of the start window.
    compact: bool,
    swapping: bool,
    /// How many times the server was asked about the account. Answers come
    /// back in any order, and only the latest question's is taken.
    account_asked: u64,
    /// What the work window holds: the choice made on the start window.
    workspace: Subsystem,
    route: Route,
    tx: Option<Transactions>,
    reference: Option<Reference>,
    logs: Option<Logs>,
    audit: Option<Audit>,
    settings: Option<Settings>,
    telegram: Option<Telegram>,
    hosted: Option<Hosted>,
    toasts: Vec<Toast>,
    next_toast: u64,
    system_dark: bool,
    #[cfg(feature = "devtools")]
    pub dev: crate::devtools::Dev,
}

/// Lets the event stream key a subscription without comparing clients.
struct Feed(Api);

impl Hash for Feed {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "api-events".hash(state);
    }
}

fn api_events(feed: &Feed) -> impl iced::futures::Stream<Item = ApiEvent> + use<> {
    use tokio::sync::broadcast::error::RecvError;
    iced::futures::stream::unfold(feed.0.subscribe(), |mut events| async move {
        loop {
            match events.recv().await {
                Ok(event) => return Some((event, events)),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    })
}

fn is_local(base: &str) -> bool {
    base.contains("://127.0.0.1") || base.contains("://localhost")
}

async fn check_entitlement(api: Api, dir: std::path::PathBuf) -> Entitled {
    if !api.has_system_token() && !is_local(&api.base()) {
        return Entitled::Blocked(None);
    }
    let cached = CachedEntitlement::load(&dir);
    let local = cached.evaluate(Utc::now());
    match api.entitlement().await {
        Ok(fresh) => {
            let state = CachedEntitlement::from_server(&fresh, Utc::now());
            if let Err(error) = state.save(&dir) {
                tracing::warn!("entitlement cache not saved: {error}");
            }
            if fresh.expired { Entitled::Blocked(fresh.allowed_until) } else { Entitled::Allowed }
        }
        // The server is out of reach: the last answer stands, and no answer
        // at all must not lock anyone out of their own books.
        Err(_) => match local {
            Verdict::Blocked { allowed_until } => Entitled::Blocked(allowed_until),
            _ => Entitled::Allowed,
        },
    }
}

fn store_path(ctx: &Ctx, user: &UserInfo) -> std::path::PathBuf {
    ctx.data_dir.join(format!("cache-{}-{}.db", user.id, ctx.host_key()))
}

fn rail_width(folded: bool) -> f32 {
    if folded { RAIL_FOLDED } else { RAIL_WIDTH }
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let data_dir = config::data_dir();
        let prefs: Preferences = config::load_json(&data_dir.join("preferences.json"));
        let access = config::discover_access();
        let base = access
            .api_base_url
            .as_deref()
            .and_then(|raw| config::canonical_base_url(raw).ok())
            .unwrap_or_else(|| config::DEFAULT_API_BASE_URL.to_owned());
        let vault: Arc<dyn SecretStore> = Arc::new(KeyringStore);
        let api = Api::new(base, access.system_token, vault);
        let ctx = Ctx {
            api,
            user: None,
            store: None,
            rail: rail_width(prefs.rail_collapsed),
            bar: 0.0,
            prefs,
            data_dir,
            dark: false,
            online: true,
            sync: SyncState::Idle,
            toasts: Vec::new(),
            requests: Vec::new(),
            window: login::WINDOW,
        };
        let app = Self::with(ctx);
        let api = app.ctx.api.clone();
        let resume = Task::perform(async move { api.resume().await.map_err(text_error) }, Message::Boot);
        let system = iced::system::theme().map(Message::SystemTheme);
        (app, Task::batch([system, resume, Task::done(Message::PingTick)]))
    }

    /// Opens on the start window, whatever session is saved: the saved one is
    /// asked about while the window is already up.
    fn with(ctx: Ctx) -> Self {
        let start = Login::starting(&ctx);
        Self {
            ctx,
            phase: Phase::Login(start),
            compact: true,
            swapping: false,
            account_asked: 0,
            workspace: Subsystem::Receipts,
            route: Route::Transactions,
            tx: None,
            reference: None,
            logs: None,
            audit: None,
            settings: None,
            telegram: None,
            hosted: None,
            toasts: Vec::new(),
            next_toast: 0,
            system_dark: false,
            #[cfg(feature = "devtools")]
            dev: crate::devtools::Dev::from_env(),
        }
    }

    fn theme(&self) -> Theme {
        theme::iced_theme(self.ctx.dark)
    }

    fn apply_theme(&mut self) {
        self.ctx.dark = match self.ctx.prefs.theme.as_deref() {
            Some("dark") => true,
            Some("system") => self.system_dark,
            _ => false,
        };
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut all = vec![
            Subscription::run_with(Feed(self.ctx.api.clone()), api_events).map(Message::Api),
            iced::system::theme_changes().map(Message::SystemTheme),
            iced::window::resize_events().map(|(_, size)| Message::Resized(size)),
            event::listen_with(tab_key),
        ];
        match &self.phase {
            Phase::Login(screen) => {
                all.push(iced::time::every(PING_EVERY).map(|_| Message::PingTick));
                all.push(screen.subscription().map(Message::Login));
            }
            Phase::Launch(screen) => all.push(screen.subscription().map(Message::Launch)),
            Phase::Main => {
                all.push(iced::time::every(PING_EVERY).map(|_| Message::PingTick));
                if let Some(tx) = &self.tx {
                    all.push(tx.subscription(self.route == Route::Transactions).map(Message::Tx));
                }
                all.extend(self.settings.as_ref().map(|screen| screen.background().map(Message::Settings)));
                all.extend(self.telegram.as_ref().map(|screen| screen.background().map(Message::Telegram)));
                if let Some(screen) = &self.hosted {
                    all.push(screen.background().map(Message::Hosted));
                    all.push(screen.subscription().map(Message::Hosted));
                }
                match self.route {
                    Route::Settings => all.extend(self.settings.as_ref().map(|screen| screen.subscription().map(Message::Settings))),
                    Route::Reference => all.extend(self.reference.as_ref().map(|screen| screen.subscription().map(Message::Reference))),
                    Route::Logs => all.extend(self.logs.as_ref().map(|screen| screen.subscription().map(Message::Logs))),
                    Route::Audit => all.extend(self.audit.as_ref().map(|screen| screen.subscription().map(Message::Audit))),
                    Route::Telegram => all.extend(self.telegram.as_ref().map(|screen| screen.subscription().map(Message::Telegram))),
                    _ => {}
                }
            }
            _ => {}
        }
        #[cfg(feature = "devtools")]
        all.push(self.dev.subscription());
        Subscription::batch(all)
    }

    fn check(&mut self) -> Task<Message> {
        self.phase = Phase::Checking;
        let (api, dir) = (self.ctx.api.clone(), self.ctx.data_dir.clone());
        Task::perform(check_entitlement(api, dir), Message::Entitled)
    }

    /// Checks has the rail on the left and no bar. A workspace that is a page
    /// has no rail: one slim bar at the top, and the page gets the rest.
    fn lay_out(&mut self) {
        (self.ctx.rail, self.ctx.bar) = if self.workspace == Subsystem::Receipts { (rail_width(self.ctx.prefs.rail_collapsed), 0.0) } else { (0.0, BAR) };
    }

    fn enter_main(&mut self) -> Task<Message> {
        self.phase = Phase::Main;
        self.lay_out();
        let Some(user) = self.ctx.user.clone() else { return Task::none() };
        let page = match self.workspace {
            Subsystem::Receipts => return self.enter_receipts(&user),
            Subsystem::Time => hosted::Kind::Time,
            Subsystem::Admin => hosted::Kind::Admin,
        };
        // Nothing of Checks is opened for a workspace that is a page: not its
        // cache, not its screens. The account may not have Checks at all.
        self.hosted = Some(Hosted::new(page, self.workspace.name()));
        Task::batch([self.enter_hosted(), Task::done(Message::PingTick)])
    }

    fn enter_receipts(&mut self, user: &UserInfo) -> Task<Message> {
        self.route = Route::ALL.into_iter().find(|r| r.open_to(user)).unwrap_or(Route::Settings);
        match Store::open(&store_path(&self.ctx, user)) {
            Ok(store) => self.ctx.store = Some(Arc::new(store)),
            Err(error) => {
                tracing::error!("local cache unavailable: {error}");
                self.ctx.toast(Tone::Warning, "Локальный кеш недоступен, данные загрузятся с сервера");
                self.ctx.store = Store::in_memory().ok().map(Arc::new);
            }
        }
        let (tx, load) = Transactions::new(&self.ctx);
        self.tx = Some(tx);
        self.settings = Some(Settings::new(&self.ctx));
        self.telegram = Some(Telegram::new());
        let settings = if self.route == Route::Settings { self.enter_settings() } else { Task::none() };
        let telegram = if self.route == Route::Telegram { self.enter_telegram() } else { Task::none() };
        Task::batch([load.map(Message::Tx), settings, telegram, Task::done(Message::PingTick)])
    }

    fn enter_settings(&mut self) -> Task<Message> {
        let Some(screen) = &mut self.settings else { return Task::none() };
        screen.enter(&mut self.ctx).map(Message::Settings)
    }

    fn enter_telegram(&mut self) -> Task<Message> {
        let Some(screen) = &mut self.telegram else { return Task::none() };
        screen.enter(&mut self.ctx).map(Message::Telegram)
    }

    /// Starts the workspace's page once the work window is the one on screen.
    /// A web view is a child of the window it was made in, and the small
    /// start window is closed as soon as the work window is up.
    fn enter_hosted(&mut self) -> Task<Message> {
        if self.compact || self.swapping || !matches!(self.phase, Phase::Main) {
            return Task::none();
        }
        let Some(screen) = &mut self.hosted else { return Task::none() };
        screen.enter(&mut self.ctx).map(Message::Hosted)
    }

    fn sign_out(&mut self) -> Task<Message> {
        // Whoever signs in next finds nothing the pages kept for this account:
        // the open one is emptied, and the folders of both are removed.
        let mut kept = hosted::folders(&self.ctx);
        // Earlier versions drew Telegram as a page, and its web view kept this.
        kept.push(self.ctx.data_dir.join("webview"));
        self.ctx.user = None;
        self.account_asked += 1;
        if let Some(screen) = &mut self.hosted {
            screen.forget_on_close();
        }
        self.close_workspace();
        hosted::discard(kept);
        self.enter_login()
    }

    /// Back to the start window with the session as it is: the workspace's
    /// screens go, the account does not, so coming back in asks for nothing.
    fn to_hub(&mut self) -> Task<Message> {
        // As when leaving the table: a locked folder takes a fresh code next time.
        if let Some(tx) = &mut self.tx {
            tx.leave(&self.ctx);
        }
        self.close_workspace();
        self.enter_login()
    }

    fn close_workspace(&mut self) {
        self.ctx.store = None;
        self.tx = None;
        self.reference = None;
        self.logs = None;
        self.audit = None;
        self.settings = None;
        self.telegram = None;
        self.hosted = None;
        self.ctx.sync = SyncState::Idle;
    }

    /// The start window for the session there is: the sign-in without one.
    /// It comes back on the workspace that was just left, if the account
    /// still has it.
    fn enter_login(&mut self) -> Task<Message> {
        let mut screen = Login::new(&self.ctx);
        screen.choose(self.workspace, &self.ctx);
        let focus = screen.focus_first().map(Message::Login);
        self.phase = Phase::Login(screen);
        Task::batch([focus, Task::done(Message::PingTick)])
    }

    /// Asks the server what the account may open now. Grants and revocations
    /// are made while clients run; the start window and an open workspace
    /// follow them without anybody signing in again.
    fn refresh_account(&mut self) -> Task<Message> {
        if self.ctx.user.is_none() || matches!(self.phase, Phase::Blocked { .. }) {
            return Task::none();
        }
        self.account_asked += 1;
        let (api, asked) = (self.ctx.api.clone(), self.account_asked);
        Task::perform(async move { api.me().await.map_err(text_error) }, move |answer| Message::Account(asked, answer))
    }

    /// Somebody signed in at the start window. The workspace chosen there
    /// opens if the account has it; otherwise the window shows what it has.
    fn signed_in(&mut self) -> Task<Message> {
        let chosen = match &self.phase {
            Phase::Login(screen) => screen.chosen(),
            _ => self.workspace,
        };
        let settled = self.access_changed();
        let Some(user) = &self.ctx.user else { return settled };
        if chosen.open_to(user) {
            self.workspace = chosen;
            return Task::batch([settled, self.check()]);
        }
        if let Phase::Login(screen) = &mut self.phase {
            screen.resumed(&self.ctx);
        }
        settled
    }

    /// What the account may open is not what it was, or may not be. A
    /// workspace on screen that it no longer has is closed, the start window
    /// lists what is left, and the local copy of Checks goes with Checks:
    /// rows of a workspace are not kept for an account that lost it.
    fn access_changed(&mut self) -> Task<Message> {
        let Some(user) = self.ctx.user.clone() else { return Task::none() };
        let lost_checks = user.workspaces_known() && !user.has_workspace(WORKSPACE_CHECKS);
        if lost_checks
            && let Some(cache) = &self.ctx.store
            && let Err(error) = cache.wipe()
        {
            tracing::warn!("the cache of a revoked workspace was not emptied: {error}");
        }
        let at_work = matches!(self.phase, Phase::Main | Phase::Checking | Phase::Launch(_));
        let task = if at_work && !self.workspace.open_to(&user) {
            self.ctx.toast(Tone::Warning, format!("Доступ к разделу «{}» закрыт администратором", self.workspace.name()));
            if let Some(screen) = &mut self.hosted {
                screen.forget_on_close();
            }
            self.to_hub()
        } else {
            if let Phase::Login(screen) = &mut self.phase {
                screen.account_changed(&self.ctx);
            }
            Task::none()
        };
        // After the workspace is closed: a file its screens hold open stays.
        if lost_checks && !store::remove_cache(&store_path(&self.ctx, &user)) {
            tracing::warn!("the cache of a revoked workspace is still on disk, emptied");
        }
        // The same for a page the account may no longer open, open just now or not.
        let pages = [(Subsystem::Time, hosted::Kind::Time), (Subsystem::Admin, hosted::Kind::Admin)];
        hosted::discard(pages.into_iter().filter(|(workspace, _)| !workspace.open_to(&user)).map(|(_, page)| hosted::folder(&self.ctx, page)).collect());
        if let Some(screen) = &mut self.settings {
            screen.access_changed(&self.ctx);
        }
        task
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.route_message(message);
        let follow = self.settle();
        // Web views float above the app, so they follow every change of screen,
        // the launch gate included.
        let main = matches!(self.phase, Phase::Main);
        if let Some(screen) = &mut self.telegram {
            screen.set_shown(main && self.route == Route::Telegram);
        }
        if let Some(screen) = &mut self.hosted {
            screen.set_shown(main);
        }
        Task::batch([task, follow, self.fit_window()])
    }

    /// One swap at a time, so two quick changes of phase cannot leave two
    /// windows open: the next one starts when this one reports back.
    fn fit_window(&mut self) -> Task<Message> {
        let compact = wants_compact(&self.phase);
        if compact == self.compact || self.swapping {
            return Task::none();
        }
        self.swapping = true;
        swap_window(compact)
    }

    /// Carries out what screens asked for while they updated.
    fn settle(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for (tone, text) in std::mem::take(&mut self.ctx.toasts) {
            // A page that covers the app's toasts shows them itself.
            if self.hosted.as_ref().is_some_and(|screen| screen.toast(tone, &text)) {
                continue;
            }
            self.next_toast += 1;
            let id = self.next_toast;
            self.toasts.push(Toast { id, tone, message: text });
            tasks.push(Task::perform(tokio::time::sleep(TOAST_LIFE), move |_| Message::DismissToast(id)));
        }
        for request in std::mem::take(&mut self.ctx.requests) {
            match request {
                Request::SignedIn(user) => {
                    self.ctx.user = Some(user);
                    tasks.push(self.signed_in());
                }
                Request::Open(system) => {
                    // Only what the account has, whoever asks: the list on the
                    // start window may be a moment older than the server's word.
                    if matches!(self.phase, Phase::Login(_)) && self.ctx.user.as_ref().is_some_and(|user| system.open_to(user)) {
                        self.workspace = system;
                        tasks.push(self.check());
                    }
                }
                Request::SignOut => tasks.push(Task::done(Message::Logout)),
                Request::ThemeChanged => {
                    self.apply_theme();
                    self.ctx.save_prefs();
                    if let Some(screen) = &self.telegram {
                        screen.theme_changed(&self.ctx);
                    }
                    if let Some(screen) = &self.hosted {
                        screen.theme_changed(&self.ctx);
                    }
                }
                Request::TransactionsChanged => {
                    if self.tx.is_some() {
                        // The server caches its manifest for minutes and a write does
                        // not reset it, so asking it whether anything changed is no use.
                        tasks.push(Task::done(Message::Tx(transactions::Msg::Sync(true))));
                    }
                }
                Request::AccessChanged => tasks.push(self.refresh_account()),
            }
        }
        Task::batch(tasks)
    }

    fn route_message(&mut self, message: Message) -> Task<Message> {
        match message {
            // The answer about the saved session changes what the start window
            // offers and nothing else: a workspace opens when it is chosen.
            Message::Boot(answer) => {
                match answer {
                    Ok(user) => self.ctx.user = user,
                    Err(reason) => tracing::warn!("session resume failed: {reason}"),
                }
                if let Phase::Login(screen) = &mut self.phase {
                    screen.resumed(&self.ctx);
                }
                // The account may have lost Checks since the last run.
                let settled = self.access_changed();
                if let Phase::Login(screen) = &self.phase {
                    return Task::batch([settled, screen.focus_first().map(Message::Login)]);
                }
                return settled;
            }
            // Asked for by a start window that has since been signed out of, or
            // come back to: nothing waits for the answer any more.
            Message::Entitled(_) if matches!(self.phase, Phase::Login(_)) => {}
            Message::Entitled(Entitled::Allowed) => return self.enter_main(),
            Message::Entitled(Entitled::Blocked(expired_at)) => self.phase = Phase::Blocked { expired_at, busy: false },
            Message::RetryEntitlement => {
                if let Phase::Blocked { busy, .. } = &mut self.phase {
                    *busy = true;
                }
                let (api, dir) = (self.ctx.api.clone(), self.ctx.data_dir.clone());
                return Task::perform(check_entitlement(api, dir), Message::Entitled);
            }
            Message::Login(inner) => {
                if let Phase::Login(screen) = &mut self.phase {
                    return screen.update(inner, &mut self.ctx).map(Message::Login);
                }
            }
            Message::Launch(inner) => {
                if let Phase::Launch(screen) = &mut self.phase {
                    let task = screen.update(inner, &mut self.ctx).map(Message::Launch);
                    if screen.unlocked {
                        self.phase = Phase::Main;
                        // The workspace the password stood before: opened if it
                        // was not yet; a page the server refused starts over.
                        if let Some(page) = &mut self.hosted {
                            page.reload();
                            // Asked for while the work window was on its way,
                            // the page has not been started: nothing else will.
                            return Task::batch([task, self.enter_hosted()]);
                        }
                        if self.workspace != Subsystem::Receipts || self.tx.is_none() {
                            return self.enter_main();
                        }
                    }
                    return task;
                }
            }
            Message::Nav(route) => return self.go_to(route),
            Message::Hub => {
                if matches!(self.phase, Phase::Main) {
                    return self.to_hub();
                }
            }
            Message::ToggleRail => {
                self.ctx.prefs.rail_collapsed = !self.ctx.prefs.rail_collapsed;
                self.lay_out();
                self.ctx.save_prefs();
                // A mini app's frame is centred in what the rail leaves.
                if let Some(screen) = &self.telegram {
                    screen.resized(&self.ctx);
                }
            }
            Message::Tab { back } => {
                let main = matches!(self.phase, Phase::Main);
                if main && self.route == Route::Transactions && self.tx.as_ref().is_some_and(Transactions::editing) {
                    return self.route_message(Message::Tx(transactions::Msg::Edit(transactions::edit::Msg::Commit)));
                }
                return focus::step(back, main && self.overlay().is_some());
            }
            Message::Resized(size) => {
                self.ctx.window = size;
                if let Some(screen) = &self.telegram {
                    screen.resized(&self.ctx);
                }
                if let Some(screen) = &self.hosted {
                    screen.resized(&self.ctx);
                }
            }
            Message::WindowSwapped(compact) => {
                self.compact = compact;
                self.swapping = false;
                // Focus asked for before the window existed went nowhere.
                if let Phase::Login(screen) = &self.phase {
                    return screen.focus_first().map(Message::Login);
                }
                // The work window is up: a workspace's page can be put in it.
                return self.enter_hosted();
            }
            Message::Tx(inner) => {
                if let Some(tx) = &mut self.tx {
                    return tx.update(inner, &mut self.ctx).map(Message::Tx);
                }
            }
            Message::Reference(inner) => {
                if let Some(screen) = &mut self.reference {
                    return screen.update(inner, &mut self.ctx).map(Message::Reference);
                }
            }
            Message::Logs(inner) => {
                if let Some(screen) = &mut self.logs {
                    return screen.update(inner, &mut self.ctx).map(Message::Logs);
                }
            }
            Message::Settings(inner) => {
                if let Some(screen) = &mut self.settings {
                    return screen.update(inner, &mut self.ctx).map(Message::Settings);
                }
            }
            Message::Audit(inner) => {
                if let Some(screen) = &mut self.audit {
                    return screen.update(inner, &mut self.ctx).map(Message::Audit);
                }
            }
            Message::Telegram(inner) => {
                if let Some(screen) = &mut self.telegram {
                    return screen.update(inner, &mut self.ctx).map(Message::Telegram);
                }
            }
            Message::Hosted(inner) => {
                if let Some(screen) = &mut self.hosted {
                    return screen.update(inner, &mut self.ctx).map(Message::Hosted);
                }
            }
            Message::PingTick => {
                let api = self.ctx.api.clone();
                let ping = Task::perform(async move { api.ping().await }, Message::Ping);
                return Task::batch([ping, self.refresh_account()]);
            }
            Message::Ping(online) => {
                self.ctx.online = online;
                if let Phase::Login(screen) = &mut self.phase {
                    screen.server_answered(online);
                }
            }
            Message::Account(asked, Ok(fresh)) => {
                // The server has been asked again since: this answer may be
                // older than what the client already knows.
                if asked != self.account_asked {
                    return Task::none();
                }
                // Asked for by an account that has since signed out, or been replaced.
                let Some(current) = self.ctx.user.as_mut().filter(|current| current.id == fresh.id) else { return Task::none() };
                current.adopt_access(&fresh);
                return self.access_changed();
            }
            // Out of reach, or a refusal the session's own events have told of.
            Message::Account(_, Err(reason)) => tracing::debug!("account not refreshed: {reason}"),
            // Late news of a session that is already gone: nobody is signed in.
            Message::Api(ApiEvent::SessionRevoked | ApiEvent::AccountDisabled) if self.ctx.user.is_none() => {}
            Message::Api(ApiEvent::SessionRevoked) => {
                // Every request the session failed says so: it is told once.
                let news = "Сессия завершена, войдите снова";
                if !self.toasts.iter().any(|toast| toast.message == news) {
                    self.ctx.toast(Tone::Warning, news);
                }
                return self.sign_out();
            }
            Message::Api(ApiEvent::AccountDisabled) => {
                let news = "Учётная запись отключена администратором";
                if !self.toasts.iter().any(|toast| toast.message == news) {
                    self.ctx.toast(Tone::Warning, news);
                }
                return self.sign_out();
            }
            Message::Api(ApiEvent::WorkspaceRevoked(workspace)) => {
                // The server's word on one workspace, taken at once; the rest
                // of what the account may open is asked for after it.
                if let Some(user) = &mut self.ctx.user {
                    user.drop_workspace(&workspace);
                }
                return Task::batch([self.access_changed(), self.refresh_account()]);
            }
            Message::Api(ApiEvent::AdminRevoked) => {
                if let Some(user) = &mut self.ctx.user {
                    user.global_admin = Some(false);
                }
                return Task::batch([self.access_changed(), self.refresh_account()]);
            }
            Message::Api(ApiEvent::AccessExpired) => {
                // The term ran out at work: the same door the next opening would
                // meet, with the workspace closed behind it.
                if matches!(self.phase, Phase::Main) {
                    if let Some(tx) = &mut self.tx {
                        tx.leave(&self.ctx);
                    }
                    self.close_workspace();
                    return self.check();
                }
            }
            Message::Api(ApiEvent::LaunchRequired) => {
                // The launch password stands before the work, not before the
                // start window: there the next thing opened asks for it again.
                if !matches!(self.phase, Phase::Launch(_) | Phase::Login(_)) {
                    self.phase = Phase::Launch(LaunchGate::default());
                    return LaunchGate::load(&self.ctx).map(Message::Launch);
                }
            }
            Message::Api(ApiEvent::ScopeExpired) => {
                self.ctx.toast(Tone::Warning, "Доступ к периоду истёк");
                let telegram = self.telegram.as_mut().map_or_else(Task::none, |screen| screen.scope_expired(&mut self.ctx).map(Message::Telegram));
                let table = if self.tx.is_some() { Task::done(Message::Tx(transactions::Msg::Access(transactions::AccessMsg::Expired))) } else { Task::none() };
                return Task::batch([telegram, table]);
            }
            Message::Api(ApiEvent::VersionMismatch) => self.ctx.toast(Tone::Warning, "Версия сервера обновилась. Перезапустите клиент."),
            Message::SystemTheme(mode) => {
                self.system_dark = mode == Mode::Dark;
                self.apply_theme();
                if let Some(screen) = &self.telegram {
                    screen.theme_changed(&self.ctx);
                }
                if let Some(screen) = &self.hosted {
                    screen.theme_changed(&self.ctx);
                }
            }
            Message::DismissToast(id) => self.toasts.retain(|t| t.id != id),
            Message::Logout => {
                let api = self.ctx.api.clone();
                return Task::perform(async move { api.logout().await }, |_| Message::LoggedOut);
            }
            Message::LoggedOut => return self.sign_out(),
            #[cfg(feature = "devtools")]
            Message::Dev(inner) => return crate::devtools::update(self, inner),
        }
        Task::none()
    }

    /// A signed-in window with no screens in it. A test of one screen draws
    /// its rail beside the page, so the page gets the room the window leaves it.
    #[cfg(test)]
    pub(crate) fn shell_for_tests(ctx: Ctx) -> Self {
        Self { phase: Phase::Main, compact: false, ..Self::with(ctx) }
    }

    #[cfg(test)]
    pub(crate) fn rail_for_tests(&self) -> El<'_, Message> {
        self.rail()
    }

    #[cfg(feature = "devtools")]
    pub fn is_ready(&self) -> bool {
        self.tx.as_ref().is_some_and(Transactions::is_ready) || self.hosted.as_ref().is_some_and(Hosted::is_ready)
    }

    #[cfg(feature = "devtools")]
    pub fn on_login(&self) -> bool {
        matches!(self.phase, Phase::Login(_))
    }

    #[cfg(feature = "devtools")]
    pub fn ctx_for_test(&mut self) -> &mut Ctx {
        &mut self.ctx
    }

    #[cfg(feature = "devtools")]
    pub fn sign_in_for_test(&mut self, username: String, password: String) -> Option<Task<Message>> {
        let Phase::Login(screen) = &mut self.phase else { return None };
        let _ = screen.update(login::Msg::Username(username), &mut self.ctx);
        let _ = screen.update(login::Msg::Password(password), &mut self.ctx);
        Some(screen.update(login::Msg::Submit, &mut self.ctx).map(Message::Login))
    }

    /// Screens other than the table start on the first visit.
    pub fn go_to(&mut self, route: Route) -> Task<Message> {
        let from = std::mem::replace(&mut self.route, route);
        let mut tasks = Vec::new();
        if from != route
            && let Some(tx) = &mut self.tx
        {
            if from == Route::Transactions {
                tx.leave(&self.ctx);
            }
            if route == Route::Transactions {
                tasks.push(Task::done(Message::Tx(transactions::Msg::Enter)));
                // Descriptions and mappings are resolved per row by the server,
                // and editing them changes no transaction the manifest could see.
                if from == Route::Reference {
                    tasks.push(Task::done(Message::Tx(transactions::Msg::Sync(true))));
                }
            }
        }
        if route == Route::Reference && self.reference.is_none() {
            let (screen, load) = Reference::new(&self.ctx);
            self.reference = Some(screen);
            tasks.push(load.map(Message::Reference));
        }
        if route == Route::Logs && self.logs.is_none() {
            let (screen, load) = Logs::new(&self.ctx);
            self.logs = Some(screen);
            tasks.push(load.map(Message::Logs));
        }
        if route == Route::Settings && from != route {
            tasks.push(self.enter_settings());
        }
        if route == Route::Telegram && from != route {
            tasks.push(self.enter_telegram());
        }
        if route == Route::Audit && self.audit.is_none() {
            let (screen, load) = Audit::new(&self.ctx);
            self.audit = Some(screen);
            tasks.push(load.map(Message::Audit));
        }
        Task::batch(tasks)
    }

    /// The account's initials in a round mark, and the name they stand for.
    fn account(&self) -> (El<'_, Message>, &str) {
        let name = self.ctx.user.as_ref().map(UserInfo::title).unwrap_or_default();
        let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).flat_map(char::to_uppercase).collect();
        let avatar = container(text(initials).size(size::SM).font(theme::FONT_BOLD))
            .width(28.0)
            .height(28.0)
            .center_x(28.0)
            .center_y(28.0)
            .style(theme::pill(Tone::Accent));
        (avatar.into(), name)
    }

    /// The connection in a dot and a word, and the hint that says it in full.
    fn connection(&self) -> (El<'_, Message>, &str) {
        let (dot, state, hint) = if self.ctx.online { (Tone::Success, "онлайн", "Backend доступен") } else { (Tone::Danger, "офлайн", "Backend недоступен") };
        (row![ui::status_dot(dot), ui::muted(state)].spacing(space::SM).align_y(Alignment::Center).into(), hint)
    }

    /// The top of a workspace that is a page: the way back to the start
    /// window, the workspace's name, the connection, the account and the
    /// sign-out in one slim line. Such a workspace has no rail and nothing to
    /// fold: its sections are inside the page.
    ///
    /// The page's web view floats above everything the app draws below this
    /// line, so the bar's hints open sideways, inside the line.
    fn bar(&self) -> El<'_, Message> {
        let hub = ui::hint_beside(ui::btn_icon(Icon::Home20, HUB, ui::Kind::Ghost, Some(Message::Hub)), HUB_HINT);
        let (network, hint) = self.connection();
        let (avatar, name) = self.account();
        let logout = ui::btn_icon(Icon::Logout, "Выйти", ui::Kind::Ghost, Some(Message::Logout));
        let line = row![
            hub,
            container(ui::divider_v()).height(theme::control::SM),
            ui::heading(self.workspace.name()),
            ui::fill(),
            ui::hint_before(network, hint),
            container(ui::divider_v()).height(theme::control::SM),
            avatar,
            container(ui::label(name)).max_width(BAR_NAME).clip(true),
            logout,
        ]
        .spacing(space::MD)
        .align_y(Alignment::Center);
        let bar = container(line).width(Fill).height(BAR_HEIGHT).center_y(BAR_HEIGHT).padding([0.0, space::MD]).style(theme::rail);
        column![bar, ui::divider()].into()
    }

    /// The navigation of Checks on the left. Folded, it keeps its icons and
    /// says the rest in hints, so the table gets the width.
    fn rail(&self) -> El<'_, Message> {
        let folded = self.ctx.prefs.rail_collapsed;
        let user = self.ctx.user.as_ref();

        let burger = ui::hint_beside(
            button(container(ui::icon(Icon::Menu, 20.0)).center(RAIL_ITEM)).padding(0).style(theme::btn_ghost).on_press(Message::ToggleRail),
            if folded { "Развернуть меню" } else { "Свернуть меню" },
        );
        let (head, caption): (El<'_, Message>, El<'_, Message>) = if folded {
            (burger, ui::divider())
        } else {
            // The workspace's own name: the product's stands on the start window.
            let brand = column![ui::heading(self.workspace.name()), ui::faint("учёт чеков")].spacing(0.0);
            let caption: El<'_, Message> = container(ui::faint("РАЗДЕЛЫ")).padding([0.0, space::SM]).into();
            (row![burger, brand].spacing(space::SM).align_y(Alignment::Center).into(), caption)
        };

        // A line of the rail: a glyph and a name, or the glyph alone once folded.
        let line = |glyph: El<'static, Message>, title: &'static str, hint: Option<&'static str>, open: bool, message: Message| -> El<'static, Message> {
            if folded {
                let square = button(container(glyph).center(RAIL_ITEM)).padding(0).style(theme::btn_nav(open)).on_press(message);
                return ui::hint_beside(square, title);
            }
            // The glyph stands where it does in the folded square.
            let name = text(title).size(size::BASE).font(if open { theme::FONT_MEDIUM } else { theme::FONT });
            let item = button(container(row![glyph, name].spacing(space::MD - 2.0).align_y(Alignment::Center)).center_y(Fill))
                .width(Fill)
                .height(RAIL_ITEM)
                .padding([0.0, (RAIL_ITEM - 20.0) / 2.0])
                .style(theme::btn_nav(open))
                .on_press(message);
            match hint {
                Some(hint) => ui::hint_beside(item, hint),
                None => item.into(),
            }
        };

        // The way back to the start window stands apart from the sections, above
        // them: it leaves the workspace, and unlike «Выйти» keeps the session.
        let hub = line(ui::icon(Icon::Home20, 20.0).into(), HUB, Some(HUB_HINT), false, Message::Hub);

        let mut nav = column![].spacing(space::XXS);
        for route in Route::ALL {
            if !user.is_some_and(|u| route.open_to(u)) {
                continue;
            }
            let open = self.route == route;
            let glyph = if open { ui::icon_tone(route.glyph(true), 20.0, Tone::Accent) } else { ui::icon(route.glyph(false), 20.0) };
            nav = nav.push(line(glyph.into(), route.title(), None, open, Message::Nav(route)));
        }

        let (avatar, name) = self.account();
        let logout = ui::icon_btn(Icon::Logout, "Выйти", Some(Message::Logout));
        let failed = matches!(self.ctx.sync, SyncState::Error(_));
        let foot: El<'_, Message> = if folded {
            // One dot for both lines of the open rail: the worse of the two.
            let (dot, hint) = match (self.ctx.online, failed) {
                (false, _) => (Tone::Danger, "Backend недоступен"),
                (true, true) => (Tone::Warning, "Ошибка обновления"),
                (true, false) => (Tone::Success, "Backend доступен"),
            };
            column![
                ui::hint_beside(container(ui::status_dot(dot)).center_x(RAIL_ITEM).padding([space::XS, 0.0]), hint),
                ui::divider(),
                ui::hint_beside(container(avatar).center_x(RAIL_ITEM), name),
                container(logout).center_x(RAIL_ITEM),
            ]
            .spacing(space::SM)
            .into()
        } else {
            let (network, hint) = self.connection();
            let sync = match &self.ctx.sync {
                SyncState::Idle => ui::faint("синхронизировано"),
                SyncState::Syncing => ui::faint("обновление…"),
                SyncState::Error(_) => ui::toned("ошибка обновления", Tone::Danger),
            };
            let account = row![avatar, container(ui::label(name)).width(Fill).clip(true), logout].spacing(space::XS).align_y(Alignment::Center);
            column![container(column![ui::hint_on(network, hint), sync].spacing(space::XS)).padding([0.0, space::SM]), ui::divider(), account].spacing(space::SM).into()
        };

        let content = column![head, hub, container(caption).height(RAIL_CAPTION).center_y(RAIL_CAPTION), nav, ui::grow(), foot].spacing(space::XS);
        container(content).width(Length::Fixed(self.ctx.rail)).height(Fill).padding(RAIL_PAD).style(theme::rail).into()
    }

    fn page(&self) -> El<'_, Message> {
        if let Some(screen) = &self.hosted {
            return screen.view(&self.ctx).map(Message::Hosted);
        }
        if self.workspace != Subsystem::Receipts {
            return ui::page(ui::loading("Загрузка…"));
        }
        match (self.route, &self.tx) {
            (Route::Transactions, Some(tx)) => tx.view(&self.ctx).map(Message::Tx),
            (Route::Reference, _) if self.reference.is_some() => self.reference.as_ref().map(|screen| screen.view(&self.ctx).map(Message::Reference)).unwrap_or_else(|| ui::loading("Загрузка…")),
            (Route::Logs, _) if self.logs.is_some() => self.logs.as_ref().map(|screen| screen.view(&self.ctx).map(Message::Logs)).unwrap_or_else(|| ui::loading("Загрузка…")),
            (Route::Settings, _) if self.settings.is_some() => self.settings.as_ref().map(|screen| screen.view(&self.ctx).map(Message::Settings)).unwrap_or_else(|| ui::loading("Загрузка…")),
            (Route::Audit, _) if self.audit.is_some() => self.audit.as_ref().map(|screen| screen.view(&self.ctx).map(Message::Audit)).unwrap_or_else(|| ui::loading("Загрузка…")),
            (Route::Telegram, _) if self.telegram.is_some() => self.telegram.as_ref().map(|screen| screen.view(&self.ctx).map(Message::Telegram)).unwrap_or_else(|| ui::loading("Загрузка…")),
            (route, _) => ui::page(ui::empty_state(route.glyph(false), route.title(), "Раздел переносится.")),
        }
    }

    /// Bottom left, past the rail: side sheets keep their buttons bottom right,
    /// and a toast there took the click meant for them. With a sheet open the
    /// toast also stays narrow enough to end before the sheet starts.
    fn toasts(&self) -> El<'_, Message> {
        let rail = if matches!(self.phase, Phase::Main) { self.ctx.rail } else { 0.0 };
        let room = if matches!(self.phase, Phase::Main) && self.overlay().is_some() {
            (self.ctx.window.width - rail - WIDEST_SHEET - 2.0 * space::LG).clamp(TOAST_MIN, TOAST_MAX)
        } else {
            TOAST_MAX
        };
        let mut list = column![].spacing(space::SM).align_x(Alignment::Start);
        for toast in &self.toasts {
            let close = ui::icon_btn(Icon::Close, "Закрыть", Some(Message::DismissToast(toast.id)));
            let line = row![ui::label(toast.message.as_str()), close].spacing(space::SM).align_y(Alignment::Center);
            list = list.push(container(line).padding([space::SM, space::MD]).max_width(room).style(theme::toast(toast.tone)));
        }
        let inset = iced::Padding::new(space::LG).left(rail + space::LG);
        container(list).width(Fill).height(Fill).padding(inset).align_x(Alignment::Start).align_y(Alignment::End).into()
    }

    fn overlay(&self) -> Option<El<'_, Message>> {
        if self.workspace != Subsystem::Receipts {
            return None;
        }
        match self.route {
            Route::Transactions => self.tx.as_ref()?.overlay(&self.ctx).map(|layer| layer.map(Message::Tx)),
            Route::Reference => self.reference.as_ref()?.overlay(&self.ctx).map(|layer| layer.map(Message::Reference)),
            Route::Logs => self.logs.as_ref()?.overlay(&self.ctx).map(|layer| layer.map(Message::Logs)),
            Route::Settings => self.settings.as_ref()?.overlay(&self.ctx).map(|layer| layer.map(Message::Settings)),
            Route::Audit => self.audit.as_ref()?.overlay(&self.ctx).map(|layer| layer.map(Message::Audit)),
            Route::Telegram => self.telegram.as_ref()?.overlay(&self.ctx).map(|layer| layer.map(Message::Telegram)),
        }
    }

    fn view(&self) -> El<'_, Message> {
        let base: El<'_, Message> = match &self.phase {
            Phase::Checking => container(ui::loading("Загрузка…")).width(Fill).height(Fill).style(theme::page).into(),
            Phase::Login(screen) => screen.view(&self.ctx).map(Message::Login),
            Phase::Blocked { expired_at, busy } => gate::blocked(expired_at.as_deref(), Message::RetryEntitlement, *busy),
            Phase::Launch(screen) => screen.view().map(Message::Launch),
            Phase::Main if self.workspace == Subsystem::Receipts => row![self.rail(), self.page()].into(),
            Phase::Main => column![self.bar(), self.page()].into(),
        };
        let mut layers: Vec<El<'_, Message>> = vec![container(base).id(focus::BASE).width(Fill).height(Fill).into()];
        if matches!(self.phase, Phase::Main)
            && let Some(overlay) = self.overlay()
        {
            layers.push(container(overlay).id(focus::LAYER).width(Fill).height(Fill).into());
        }
        if !self.toasts.is_empty() {
            layers.push(self.toasts());
        }
        Stack::with_children(layers).width(Fill).height(Fill).into()
    }
}

/// Only a Tab nothing else wanted: the table takes it to walk between cells.
fn tab_key(event: Event, status: event::Status, _window: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Named(Named::Tab), modifiers, .. }) if status == event::Status::Ignored => {
            Some(Message::Tab { back: modifiers.shift() })
        }
        _ => None,
    }
}

fn wants_compact(phase: &Phase) -> bool {
    matches!(phase, Phase::Login(_))
}

/// The work window never goes past the screen, nor under the size its pages
/// are laid out for.
fn work_size(screen: Option<Size>) -> Size {
    let Some(screen) = screen else { return WORK_WINDOW };
    Size::new(
        WORK_WINDOW.width.min(screen.width).max(WORK_MIN_WINDOW.width),
        WORK_WINDOW.height.min(screen.height - SCREEN_CHROME).max(WORK_MIN_WINDOW.height),
    )
}

fn window_settings(compact: bool, screen: Option<Size>) -> window::Settings {
    let shared = window::Settings {
        position: window::Position::Centered,
        icon: window::icon::from_file_data(include_bytes!("../assets/app-256.png"), None).ok(),
        ..Default::default()
    };
    if compact {
        window::Settings { size: login::WINDOW, resizable: false, ..shared }
    } else {
        window::Settings { size: work_size(screen), min_size: Some(WORK_MIN_WINDOW), ..shared }
    }
}

/// Two windows rather than one that changes size: Windows gives a window its
/// maximize button when it is created, and a fixed one cannot get it back.
fn swap_window(compact: bool) -> Task<Message> {
    window::oldest().then(move |old| {
        let Some(old) = old else { return Task::none() };
        window::monitor_size(old).then(move |screen| {
            let settings = window_settings(compact, screen);
            let size = settings.size;
            let (_, opened) = window::open(settings);
            opened
                .then(move |_| window::close(old))
                .chain(Task::done(Message::Resized(size)))
                .chain(Task::done(Message::WindowSwapped(compact)))
        })
    })
}

pub fn run() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title(PRODUCT)
        .theme(App::theme)
        .subscription(App::subscription)
        .default_font(theme::FONT)
        .window(window_settings(true, None))
        .antialiasing(true)
        .run()
}

#[cfg(test)]
#[path = "live_ui.rs"]
mod live_ui;

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn the_sign_in_window_is_small_and_fixed() {
        let settings = window_settings(true, None);
        assert_eq!(settings.size, login::WINDOW);
        assert!(!settings.resizable);
    }

    #[test]
    fn the_work_window_can_be_resized_and_maximized() {
        let settings = window_settings(false, Some(Size::new(1920.0, 1080.0)));
        assert_eq!(settings.size, WORK_WINDOW);
        assert_eq!(settings.min_size, Some(WORK_MIN_WINDOW));
        assert!(settings.resizable);
    }

    #[test]
    fn the_work_window_stays_on_a_small_screen() {
        assert_eq!(work_size(Some(Size::new(1280.0, 800.0))), Size::new(1280.0, 720.0));
    }

    #[test]
    fn the_work_window_keeps_its_smallest_size_on_a_tiny_screen() {
        assert_eq!(work_size(Some(Size::new(800.0, 600.0))), WORK_MIN_WINDOW);
    }

    #[test]
    fn only_the_start_window_is_compact() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_tests(dir.path());
        assert!(wants_compact(&Phase::Login(Login::new(&ctx))));
        assert!(!wants_compact(&Phase::Checking));
        assert!(!wants_compact(&Phase::Main));
    }
}

#[cfg(test)]
mod rail_tests {
    use iced_test::Simulator;
    use iced_test::core::Settings;

    use super::*;

    fn signed_in(dir: &tempfile::TempDir) -> App {
        let mut ctx = Ctx::for_tests(dir.path());
        ctx.user = Some(UserInfo { username: "anna".into(), role: "admin".into(), ..Default::default() });
        App::with(ctx)
    }

    fn sees(app: &App, wanted: &str) -> bool {
        let mut sim = Simulator::with_size(Settings::default(), WORK_WINDOW, app.rail());
        sim.find(wanted).is_ok()
    }

    #[test]
    fn the_folded_rail_keeps_its_icons_and_drops_the_words() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = signed_in(&dir);
        for words in [Subsystem::Receipts.name(), HUB, "РАЗДЕЛЫ", "Справочник", "онлайн", "anna"] {
            assert!(sees(&app, words), "{words} is missing from the open rail");
        }

        let _ = app.update(Message::ToggleRail);
        for words in [Subsystem::Receipts.name(), HUB, "РАЗДЕЛЫ", "Справочник", "онлайн", "anna"] {
            assert!(!sees(&app, words), "{words} is still on the folded rail");
        }
        // The avatar stays, and with it who is signed in.
        assert!(sees(&app, "A"));
    }

    #[test]
    fn the_rail_is_headed_by_the_workspace_and_not_by_the_product() {
        let dir = tempfile::tempdir().unwrap();
        let app = signed_in(&dir);
        assert!(sees(&app, "Чеки"));
        assert!(!sees(&app, PRODUCT), "the product's name belongs to the start window");
        // The window, the installer and the update keep the product's name.
        assert_eq!(PRODUCT, "TBS inform CRM");
    }

    #[test]
    fn the_rail_leads_back_to_the_start_window_folded_or_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = signed_in(&dir);
        let pressed = |app: &App, at: iced::Point| -> Vec<Message> {
            let mut sim = Simulator::with_size(Settings::default(), WORK_WINDOW, app.rail());
            sim.point_at(at);
            let _ = sim.simulate(iced_test::simulator::click());
            sim.into_messages().collect()
        };

        let mut sim = Simulator::with_size(Settings::default(), WORK_WINDOW, app.rail());
        sim.click(HUB).expect("the way back on the open rail");
        let messages: Vec<Message> = sim.into_messages().collect();
        assert!(matches!(messages.as_slice(), [Message::Hub]), "{messages:?}");

        // Folded, it is the square under the burger: the second line of the rail.
        let _ = app.update(Message::ToggleRail);
        let second = iced::Point::new(RAIL_FOLDED / 2.0, RAIL_PAD + RAIL_ITEM + space::XS + RAIL_ITEM / 2.0);
        let messages = pressed(&app, second);
        assert!(matches!(messages.as_slice(), [Message::Hub]), "{messages:?}");
    }

    #[test]
    fn folding_is_remembered_and_moves_what_stands_beside_the_rail() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = signed_in(&dir);
        assert_eq!(app.ctx.rail, RAIL_WIDTH);

        let _ = app.update(Message::ToggleRail);
        assert_eq!(app.ctx.rail, RAIL_FOLDED);
        let saved: Preferences = config::load_json(&dir.path().join("preferences.json"));
        assert!(saved.rail_collapsed);
        assert_eq!(rail_width(saved.rail_collapsed), RAIL_FOLDED);

        let _ = app.update(Message::ToggleRail);
        assert_eq!(app.ctx.rail, RAIL_WIDTH);
        let saved: Preferences = config::load_json(&dir.path().join("preferences.json"));
        assert!(!saved.rail_collapsed);
    }
}

/// Every launch opens on the start window, and one session serves it and the
/// workspace alike.
#[cfg(test)]
mod hub_tests {
    use iced_test::Simulator;
    use iced_test::core::Settings;
    use tbs_core::api::LoginResponse;
    use tbs_core::secret::MemoryStore;

    use super::*;

    fn anna() -> UserInfo {
        UserInfo { id: 7, username: "anna".into(), display_name: Some("Анна К.".into()), role: "admin".into(), ..Default::default() }
    }

    /// A launch: nobody is known yet, and the last run may have left a session.
    fn launched(dir: &tempfile::TempDir, saved_session: bool) -> App {
        let mut ctx = Ctx::for_tests(dir.path());
        let vault = Arc::new(MemoryStore::default());
        if saved_session {
            // Where the client keeps its refresh token: one entry per server.
            vault.set(&format!("refresh:{}", ctx.api.base()), "saved-by-the-last-run");
        }
        ctx.api = Api::new(ctx.api.base(), None, vault);
        App::with(ctx)
    }

    fn on_the_start_window(app: &App) -> bool {
        matches!(app.phase, Phase::Login(_))
    }

    /// The window the phase is shown in.
    fn window(app: &App) -> Size {
        if wants_compact(&app.phase) { login::WINDOW } else { WORK_WINDOW }
    }

    fn sees(app: &App, wanted: &str) -> bool {
        let mut sim = Simulator::with_size(Settings::default(), window(app), app.view());
        sim.find(wanted).is_ok()
    }

    /// A click on the words, and what it says carried out as the window would.
    fn press(app: &mut App, wanted: &str) {
        let messages: Vec<Message> = {
            let mut sim = Simulator::with_size(Settings::default(), window(app), app.view());
            sim.click(wanted).unwrap_or_else(|_| panic!("no «{wanted}» on the screen"));
            sim.into_messages().collect()
        };
        assert!(!messages.is_empty(), "«{wanted}» does nothing");
        for message in messages {
            let _ = app.update(message);
        }
    }

    /// A launch with a session that the server confirmed.
    fn resumed(dir: &tempfile::TempDir) -> App {
        let mut app = launched(dir, true);
        let _ = app.update(Message::Boot(Ok(Some(anna()))));
        app
    }

    /// The same, with the workspace opened from the start window.
    fn at_work(dir: &tempfile::TempDir) -> App {
        let mut app = resumed(dir);
        press(&mut app, "Открыть Чеки");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert!(matches!(app.phase, Phase::Main));
        app
    }

    #[test]
    fn a_launch_with_a_saved_session_opens_on_the_start_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = launched(&dir, true);
        // Before the server has answered: the start window already, and no form on it.
        assert!(on_the_start_window(&app));
        assert!(sees(&app, "РАБОЧЕЕ ПРОСТРАНСТВО") && sees(&app, "Чеки"));
        assert!(sees(&app, "Проверяем сохранённый вход на этом компьютере"));
        assert!(!sees(&app, "Войти"));

        let _ = app.update(Message::Boot(Ok(Some(anna()))));
        assert!(on_the_start_window(&app), "a saved session must not skip the start window");
        assert!(app.tx.is_none() && app.ctx.store.is_none(), "nothing of the table is opened before it is asked for");
        assert!(sees(&app, "РАБОЧЕЕ ПРОСТРАНСТВО") && sees(&app, "CRM"));
        assert!(sees(&app, "Вход выполнен") && sees(&app, "Анна К.") && sees(&app, "@anna"));
        assert!(sees(&app, "Открыть Чеки"));
        assert!(!sees(&app, "Войти") && !sees(&app, "Пароль"));
    }

    #[test]
    fn a_launch_without_a_session_opens_on_the_start_window_with_the_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = launched(&dir, false);
        // Nothing saved, so nothing to wait for: the sign-in is there at once.
        assert!(on_the_start_window(&app));
        assert!(sees(&app, "Вход в Чеки") && sees(&app, "Войти"));

        let _ = app.update(Message::Boot(Ok(None)));
        assert!(on_the_start_window(&app));
        assert!(app.ctx.user.is_none());
        assert!(sees(&app, "РАБОЧЕЕ ПРОСТРАНСТВО") && sees(&app, "Вход в Чеки") && sees(&app, "Войти"));
        assert!(!sees(&app, "Открыть Чеки"));
    }

    #[test]
    fn a_session_the_server_could_not_be_asked_about_is_kept_for_the_next_launch() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = launched(&dir, true);
        let _ = app.update(Message::Boot(Err("сервер недоступен".into())));
        assert!(on_the_start_window(&app));
        assert!(sees(&app, "Вход в Чеки") && sees(&app, "Войти"));
        assert!(app.ctx.api.has_refresh_token(), "an unreachable server is no reason to forget the session");
    }

    #[test]
    fn choosing_the_workspace_opens_it_on_the_saved_session() {
        for way_in in ["Транзакции и учёт чеков", "Открыть Чеки"] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = resumed(&dir);
            press(&mut app, way_in);
            assert!(matches!(app.phase, Phase::Checking), "«{way_in}» did not ask for the workspace");

            let _ = app.update(Message::Entitled(Entitled::Allowed));
            assert!(matches!(app.phase, Phase::Main));
            assert!(app.tx.is_some() && app.ctx.store.is_some());
            assert_eq!(app.route, Route::Transactions);
            // The session it runs on is the saved one: nobody was asked to sign in.
            assert_eq!(app.ctx.user.as_ref().map(|user| user.id), Some(7));
            assert!(app.ctx.api.has_refresh_token());
        }
    }

    #[test]
    fn an_expired_licence_still_stands_between_the_start_window_and_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = resumed(&dir);
        press(&mut app, "Открыть Чеки");
        let _ = app.update(Message::Entitled(Entitled::Blocked(Some("2026-07-01".into()))));
        assert!(matches!(app.phase, Phase::Blocked { .. }));
        assert!(app.tx.is_none());
        assert!(sees(&app, "Срок доступа истёк"));
    }

    #[test]
    fn signing_in_at_the_start_window_opens_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = launched(&dir, false);
        let _ = app.update(Message::Boot(Ok(None)));
        let answer = LoginResponse { ok: true, user: Some(anna()), ..Default::default() };
        let _ = app.update(Message::Login(login::Msg::Answered(Ok(answer))));
        assert!(matches!(app.phase, Phase::Checking));
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert!(matches!(app.phase, Phase::Main));
        assert!(app.tx.is_some());
    }

    #[test]
    fn the_way_back_keeps_the_session_and_the_way_in_asks_for_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = at_work(&dir);
        assert!(sees(&app, "Чеки") && sees(&app, HUB));

        press(&mut app, HUB);
        assert!(on_the_start_window(&app));
        assert!(app.tx.is_none() && app.settings.is_none() && app.telegram.is_none() && app.ctx.store.is_none());
        assert_eq!(app.ctx.user.as_ref().map(|user| user.id), Some(7), "going back is not signing out");
        assert!(app.ctx.api.has_refresh_token());
        assert!(sees(&app, "Вход выполнен") && sees(&app, "Открыть Чеки"));
        assert!(!sees(&app, "Войти"));

        press(&mut app, "Открыть Чеки");
        assert!(matches!(app.phase, Phase::Checking));
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert!(matches!(app.phase, Phase::Main));
        assert!(app.tx.is_some());
    }

    #[test]
    fn the_way_back_works_from_the_workspace_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = resumed(&dir);
        press(&mut app, "Открыть Чеки");
        let _ = app.update(Message::Hub);
        assert!(matches!(app.phase, Phase::Checking), "there is nothing to go back from yet");
    }

    #[test]
    fn signing_out_comes_back_to_the_start_window_signed_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = at_work(&dir);
        // What follows the server's answer to «Выйти».
        let _ = app.update(Message::LoggedOut);
        assert!(on_the_start_window(&app));
        assert!(app.ctx.user.is_none() && app.tx.is_none() && app.ctx.store.is_none());
        assert!(sees(&app, "РАБОЧЕЕ ПРОСТРАНСТВО") && sees(&app, "Вход в Чеки") && sees(&app, "Войти"));
        assert!(!sees(&app, "Открыть Чеки") && !sees(&app, "Вход выполнен"));
    }

    #[test]
    fn signing_out_at_the_start_window_leaves_the_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = resumed(&dir);
        press(&mut app, "Выйти из учётной записи");
        // The request went to the server; its answer ends the session here.
        assert!(on_the_start_window(&app));
        let _ = app.update(Message::LoggedOut);
        assert!(on_the_start_window(&app));
        assert!(app.ctx.user.is_none());
        assert!(sees(&app, "Вход в Чеки") && !sees(&app, "Открыть Чеки"));
    }

    /// Says so in a toast, and a toast is timed: that takes a runtime.
    #[tokio::test(flavor = "current_thread")]
    async fn a_session_that_ends_comes_back_to_the_sign_in_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        for mut app in [at_work(&dir), resumed(&dir)] {
            let _ = app.update(Message::Api(ApiEvent::SessionRevoked));
            assert!(on_the_start_window(&app));
            assert!(app.ctx.user.is_none() && app.tx.is_none());
            assert!(sees(&app, "Вход в Чеки") && sees(&app, "Сессия завершена, войдите снова"));
            // Two requests failed on the same ended session: still one piece of news.
            let _ = app.update(Message::Api(ApiEvent::SessionRevoked));
            assert!(on_the_start_window(&app));
            assert_eq!(app.toasts.len(), 1);
        }
    }

    #[test]
    fn nothing_opens_the_workspace_from_the_start_window_but_choosing_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = resumed(&dir);
        // Answers that come late: the licence asked about before going back, a
        // launch password the server wanted from a request already over.
        for late in [Message::Entitled(Entitled::Allowed), Message::Entitled(Entitled::Blocked(None)), Message::Api(ApiEvent::LaunchRequired)] {
            let _ = app.update(late);
            assert!(on_the_start_window(&app));
            assert!(app.tx.is_none());
        }
        // And without a session choosing it does not open it either.
        let dir = tempfile::tempdir().unwrap();
        let mut app = launched(&dir, false);
        let _ = app.update(Message::Boot(Ok(None)));
        press(&mut app, "Транзакции и учёт чеков");
        assert!(on_the_start_window(&app));
        assert!(sees(&app, "Вход в Чеки"));
    }

    #[test]
    fn the_launch_password_still_stands_before_the_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = resumed(&dir);
        press(&mut app, "Открыть Чеки");
        let _ = app.update(Message::Api(ApiEvent::LaunchRequired));
        assert!(matches!(app.phase, Phase::Launch(_)));
    }
}

/// More than one thing to open: Checks, Time, CRM administration. Each account
/// gets what the server says it has, and loses it when the server says so.
#[cfg(test)]
mod workspace_tests {
    use iced_test::Simulator;
    use iced_test::core::Settings;
    use tbs_core::api::{LaunchVerify, LoginResponse};
    use tbs_core::model::Transaction;
    use tbs_core::secret::MemoryStore;

    use super::*;

    /// An account as a server that knows workspaces describes it.
    fn account(workspaces: &[&str], global_admin: bool) -> UserInfo {
        UserInfo {
            id: 7,
            username: "anna".into(),
            display_name: Some("Анна К.".into()),
            role: "operator".into(),
            workspaces: Some(workspaces.iter().map(|name| (*name).to_owned()).collect()),
            global_admin: Some(global_admin),
            ..Default::default()
        }
    }

    /// The same account on a server from before workspaces.
    fn older() -> UserInfo {
        UserInfo { id: 7, username: "anna".into(), display_name: Some("Анна К.".into()), role: "admin".into(), ..Default::default() }
    }

    fn launched(dir: &tempfile::TempDir, saved_session: bool) -> App {
        let mut ctx = Ctx::for_tests(dir.path());
        let vault = Arc::new(MemoryStore::default());
        if saved_session {
            vault.set(&format!("refresh:{}", ctx.api.base()), "saved-by-the-last-run");
        }
        ctx.api = Api::new(ctx.api.base(), None, vault);
        App::with(ctx)
    }

    /// A launch on the saved session of this account.
    fn resumed(dir: &tempfile::TempDir, user: UserInfo) -> App {
        let mut app = launched(dir, true);
        let _ = app.update(Message::Boot(Ok(Some(user))));
        app
    }

    fn on_the_start_window(app: &App) -> bool {
        matches!(app.phase, Phase::Login(_))
    }

    fn window(app: &App) -> Size {
        if wants_compact(&app.phase) { login::WINDOW } else { WORK_WINDOW }
    }

    fn sees(app: &App, wanted: &str) -> bool {
        let mut sim = Simulator::with_size(Settings::default(), window(app), app.view());
        sim.find(wanted).is_ok()
    }

    /// What a click on these words asks the shell for.
    fn clicked(app: &App, wanted: &str) -> Vec<Message> {
        let mut sim = Simulator::with_size(Settings::default(), window(app), app.view());
        sim.click(wanted).unwrap_or_else(|_| panic!("no «{wanted}» on the screen"));
        sim.into_messages().collect()
    }

    fn press(app: &mut App, wanted: &str) {
        let messages = clicked(app, wanted);
        assert!(!messages.is_empty(), "«{wanted}» does nothing");
        for message in messages {
            let _ = app.update(message);
        }
    }

    /// Chosen on the start window, let in by the server, the work window up.
    fn opened(dir: &tempfile::TempDir, user: UserInfo, way_in: &str) -> App {
        let mut app = resumed(dir, user);
        press(&mut app, way_in);
        assert!(matches!(app.phase, Phase::Checking), "«{way_in}» did not ask for anything");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert!(matches!(app.phase, Phase::Main));
        let _ = app.update(Message::WindowSwapped(false));
        app
    }

    /// Asks the shell for a workspace the way a screen would.
    fn ask(app: &mut App, system: Subsystem) {
        app.ctx.request(Request::Open(system));
        let _ = app.update(Message::Ping(true));
    }

    const CHECKS_ONLY: [&str; 8] = ["РАЗДЕЛЫ", "Транзакции", "Справочник", "Telegram", "Логи", "Настройки", "Аудит", "синхронизировано"];

    #[test]
    fn time_opens_as_a_page_on_the_session_there_is_and_nothing_of_checks_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let app = opened(&dir, account(&["time"], false), "Открыть Учёт времени");
        assert_eq!(app.workspace, Subsystem::Time);
        assert!(app.hosted.as_ref().is_some_and(Hosted::started));
        assert!(app.tx.is_none() && app.settings.is_none() && app.telegram.is_none() && app.ctx.store.is_none(), "something of Checks was opened for a Time account");
        assert!(!store_path(&app.ctx, app.ctx.user.as_ref().unwrap()).exists(), "a Checks cache was made for an account without Checks");
        // Nobody was asked to sign in again.
        assert_eq!(app.ctx.user.as_ref().map(|user| user.id), Some(7));
        assert!(app.ctx.api.has_refresh_token());
        // No rail and nothing to fold: one bar with Time's name, the way back,
        // the connection, the account and the sign-out; none of Checks' sections.
        assert_eq!((app.ctx.rail, app.ctx.bar), (0.0, BAR));
        for words in ["Учёт времени", HUB, "онлайн", "Анна К.", "Выйти"] {
            assert!(sees(&app, words), "«{words}» is missing from the bar");
        }
        for words in CHECKS_ONLY {
            assert!(!sees(&app, words), "«{words}» of a rail is on Time's screen");
        }
    }

    #[test]
    fn administration_opens_for_an_administrator_who_has_no_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let app = opened(&dir, account(&[], true), "Открыть Администрирование CRM");
        assert_eq!(app.workspace, Subsystem::Admin);
        assert!(app.hosted.as_ref().is_some_and(Hosted::started));
        assert!(app.tx.is_none() && app.ctx.store.is_none());
        assert_eq!((app.ctx.rail, app.ctx.bar), (0.0, BAR));
        for words in ["Администрирование CRM", HUB, "онлайн", "Анна К.", "Выйти"] {
            assert!(sees(&app, words), "«{words}» is missing from the bar");
        }
        for words in CHECKS_ONLY {
            assert!(!sees(&app, words), "«{words}» of a rail is on the administration screen");
        }
    }

    #[test]
    fn the_bar_leads_back_to_the_start_window_and_signs_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["time"], true), "Табель и отметки сотрудников");
        assert!(matches!(clicked(&app, "Выйти").as_slice(), [Message::Logout]));
        // The way back keeps the session; the bar goes with the workspace.
        press(&mut app, HUB);
        assert!(on_the_start_window(&app) && app.hosted.is_none());
        assert_eq!(app.ctx.user.as_ref().map(|user| user.id), Some(7));
        assert!(app.ctx.api.has_refresh_token());
        assert!(sees(&app, "Табель и отметки сотрудников") && sees(&app, "Учётные записи и доступ"));
        // The line says when the server cannot be reached.
        let mut app = opened(&dir, account(&["time"], false), "Открыть Учёт времени");
        app.ctx.online = false;
        assert!(sees(&app, "офлайн") && !sees(&app, "онлайн"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn checks_keeps_its_rail_and_a_page_never_gets_one() {
        let dir = tempfile::tempdir().unwrap();
        // Checks: the rail as it was, folded or not, and no bar.
        let mut app = opened(&dir, account(&["checks", "time"], true), "Транзакции и учёт чеков");
        assert_eq!((app.ctx.rail, app.ctx.bar), (RAIL_WIDTH, 0.0));
        for words in ["Чеки", "РАЗДЕЛЫ", "синхронизировано", HUB] {
            assert!(sees(&app, words), "«{words}» is missing from the rail of Checks");
        }
        let _ = app.update(Message::ToggleRail);
        assert_eq!((app.ctx.rail, app.ctx.bar), (RAIL_FOLDED, 0.0));

        // From Checks to a page on the same session: the rail goes, folded or not.
        let _ = app.update(Message::Hub);
        press(&mut app, "Табель и отметки сотрудников");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert_eq!(app.workspace, Subsystem::Time);
        assert_eq!((app.ctx.rail, app.ctx.bar), (0.0, BAR));
        for words in CHECKS_ONLY {
            assert!(!sees(&app, words), "«{words}» of Checks is on Time's screen");
        }

        // To the other page, and back to Checks: each gets its own again.
        let _ = app.update(Message::Hub);
        press(&mut app, "Учётные записи и доступ");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert_eq!(app.workspace, Subsystem::Admin);
        assert_eq!((app.ctx.rail, app.ctx.bar), (0.0, BAR));
        let _ = app.update(Message::Hub);
        press(&mut app, "Транзакции и учёт чеков");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert_eq!(app.workspace, Subsystem::Receipts);
        assert_eq!((app.ctx.rail, app.ctx.bar), (RAIL_FOLDED, 0.0));
        assert!(app.ctx.prefs.rail_collapsed, "the fold of Checks was lost on the way through the pages");
    }

    #[test]
    fn a_page_is_started_only_once_the_work_window_is_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = resumed(&dir, account(&["time"], false));
        press(&mut app, "Открыть Учёт времени");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        // The small start window is still the one on screen, and about to be
        // closed: a web view made in it would go with it.
        assert!(app.hosted.as_ref().is_some_and(|page| !page.started()));
        assert!(sees(&app, "Загрузка…"));
        let _ = app.update(Message::WindowSwapped(false));
        assert!(app.hosted.as_ref().is_some_and(Hosted::started));
    }

    #[test]
    fn checks_opens_as_it_did_for_an_account_that_has_it() {
        let dir = tempfile::tempdir().unwrap();
        for user in [older(), account(&["checks"], false), account(&["checks", "time"], true)] {
            let app = opened(&dir, user, "Открыть Чеки");
            assert_eq!(app.workspace, Subsystem::Receipts);
            assert!(app.tx.is_some() && app.ctx.store.is_some() && app.hosted.is_none());
            assert!(sees(&app, "Чеки") && sees(&app, "РАЗДЕЛЫ") && sees(&app, HUB));
        }
    }

    #[test]
    fn nothing_the_account_does_not_have_opens_by_asking() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [(UserInfo, &[Subsystem]); 5] = [
            (older(), &[Subsystem::Time, Subsystem::Admin]),
            (account(&["checks"], false), &[Subsystem::Time, Subsystem::Admin]),
            (account(&["time"], false), &[Subsystem::Receipts, Subsystem::Admin]),
            (account(&[], true), &[Subsystem::Receipts, Subsystem::Time]),
            (account(&[], false), &[Subsystem::Receipts, Subsystem::Time, Subsystem::Admin]),
        ];
        for (user, closed) in cases {
            let mut app = resumed(&dir, user);
            for system in closed {
                ask(&mut app, *system);
                assert!(on_the_start_window(&app), "{system:?} opened for {:?}", app.ctx.user);
                assert!(app.hosted.is_none() && app.tx.is_none());
            }
        }
    }

    #[test]
    fn signing_in_opens_the_chosen_workspace_only_for_an_account_that_has_it() {
        let dir = tempfile::tempdir().unwrap();
        let sign_in = |user: UserInfo| {
            let mut app = launched(&dir, false);
            let _ = app.update(Message::Boot(Ok(None)));
            let answer = LoginResponse { ok: true, user: Some(user), ..Default::default() };
            let _ = app.update(Message::Login(login::Msg::Answered(Ok(answer))));
            app
        };
        // Checks is what the window offers before anybody is known, and this account has it.
        assert!(matches!(sign_in(account(&["checks", "time"], false)).phase, Phase::Checking));
        assert!(matches!(sign_in(older()).phase, Phase::Checking));

        // This one has Time only: Checks does not open, and the window shows what the account has.
        let app = sign_in(account(&["time"], false));
        assert!(on_the_start_window(&app), "Checks was opened for an account without it");
        assert!(sees(&app, "Вход выполнен") && sees(&app, "Открыть Учёт времени") && !sees(&app, "Открыть Чеки"));
        assert!(!sees(&app, "Войти"));

        // And this one nothing at all.
        let app = sign_in(account(&[], false));
        assert!(on_the_start_window(&app));
        assert!(sees(&app, "Нет доступных пространств") && sees(&app, "Выйти из учётной записи"));
    }

    #[test]
    fn the_way_back_from_a_page_keeps_the_session_and_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["checks", "time"], false), "Табель и отметки сотрудников");
        press(&mut app, HUB);
        assert!(on_the_start_window(&app) && app.hosted.is_none());
        assert_eq!(app.ctx.user.as_ref().map(|user| user.id), Some(7), "going back is not signing out");
        assert!(app.ctx.api.has_refresh_token());
        // It comes back on the workspace that was left, and in again without a word.
        assert!(sees(&app, "Открыть Учёт времени") && !sees(&app, "Войти"));
        press(&mut app, "Открыть Учёт времени");
        assert!(matches!(app.phase, Phase::Checking));
        // The other workspace is a choice away, on the same session.
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        let _ = app.update(Message::WindowSwapped(false));
        press(&mut app, HUB);
        press(&mut app, "Транзакции и учёт чеков");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert!(app.tx.is_some() && app.hosted.is_none());
        assert_eq!(app.workspace, Subsystem::Receipts);
    }

    /// Says so in a toast, and a toast is timed: that takes a runtime.
    #[tokio::test(flavor = "current_thread")]
    async fn a_workspace_taken_away_closes_and_the_start_window_lists_what_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["checks", "time"], false), "Табель и отметки сотрудников");
        let _ = app.update(Message::Api(ApiEvent::WorkspaceRevoked("time".into())));
        assert!(on_the_start_window(&app));
        assert!(app.hosted.is_none(), "the page of a workspace the account lost is still open");
        assert_eq!(app.ctx.user.as_ref().map(|user| user.id), Some(7), "losing a workspace is not losing the session");
        assert!(app.ctx.api.has_refresh_token());
        assert!(sees(&app, "Доступ к разделу «Учёт времени» закрыт администратором"));
        assert!(sees(&app, "Открыть Чеки") && !sees(&app, "Табель и отметки сотрудников"));
        // Every request the page had on its way says the same: it is told once.
        let _ = app.update(Message::Api(ApiEvent::WorkspaceRevoked("time".into())));
        assert_eq!(app.toasts.len(), 1);
        // And it does not open again by asking.
        ask(&mut app, Subsystem::Time);
        assert!(on_the_start_window(&app) && app.hosted.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_the_server_says_of_the_account_is_followed_while_the_client_runs() {
        let dir = tempfile::tempdir().unwrap();
        // At work: the workspace on screen is taken away, and the periodic question finds out.
        let mut app = opened(&dir, account(&["time"], true), "Открыть Учёт времени");
        let _ = app.update(Message::Account(app.account_asked, Ok(account(&[], true))));
        assert!(on_the_start_window(&app) && app.hosted.is_none());
        assert!(sees(&app, "Открыть Администрирование CRM") && !sees(&app, "Открыть Учёт времени"));

        // On the start window: a grant appears, a revocation disappears.
        let _ = app.update(Message::Account(app.account_asked, Ok(account(&["checks", "time"], false))));
        assert!(sees(&app, "Транзакции и учёт чеков") && sees(&app, "Табель и отметки сотрудников") && !sees(&app, "Учётные записи и доступ"));

        // Somebody else's answer, come late: nothing of it is taken.
        let mut other = account(&[], true);
        other.id = 99;
        let _ = app.update(Message::Account(app.account_asked, Ok(other)));
        assert!(sees(&app, "Транзакции и учёт чеков") && !sees(&app, "Учётные записи и доступ"));
        // A server that could not be asked changes nothing.
        let _ = app.update(Message::Account(app.account_asked, Err("Нет связи с сервером".into())));
        assert!(sees(&app, "Табель и отметки сотрудников"));

        // An open workspace the account still has is left alone.
        let mut app = opened(&dir, account(&["checks", "time"], true), "Табель и отметки сотрудников");
        let _ = app.update(Message::Account(app.account_asked, Ok(account(&["time"], false))));
        assert!(matches!(app.phase, Phase::Main) && app.hosted.is_some());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn losing_administration_closes_the_panel_and_keeps_the_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["checks"], true), "Учётные записи и доступ");
        assert_eq!(app.workspace, Subsystem::Admin);
        let _ = app.update(Message::Api(ApiEvent::AdminRevoked));
        assert!(on_the_start_window(&app) && app.hosted.is_none());
        assert!(sees(&app, "Доступ к разделу «Администрирование CRM» закрыт администратором"));
        assert!(sees(&app, "Открыть Чеки") && !sees(&app, "Учётные записи и доступ"));

        // In Checks the same refusal closes nothing: it answered a call the
        // settings made, and the sections that made it go.
        let mut app = opened(&dir, account(&["checks"], true), "Открыть Чеки");
        assert!(app.ctx.manages_accounts());
        let _ = app.update(Message::Api(ApiEvent::AdminRevoked));
        assert!(matches!(app.phase, Phase::Main) && app.tx.is_some());
        assert!(!app.ctx.manages_accounts());
        assert!(app.toasts.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn checks_taken_away_takes_its_local_copy_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["checks", "time"], false), "Открыть Чеки");
        let user = app.ctx.user.clone().unwrap();
        let cache = store_path(&app.ctx, &user);
        let row = Transaction { id: 1, raw_message: Some("ROW-OF-AN-ACCOUNT-THAT-LOST-CHECKS".into()), ..Default::default() };
        app.ctx.store.as_ref().unwrap().replace_transactions(&[row], Some("sha"), Some("digest")).unwrap();
        assert!(cache.exists());
        // Another account's cache on the same computer.
        let other = dir.path().join(format!("cache-8-{}.db", app.ctx.host_key()));
        drop(Store::open(&other).unwrap());

        let _ = app.update(Message::Api(ApiEvent::WorkspaceRevoked("checks".into())));
        assert!(on_the_start_window(&app) && app.tx.is_none() && app.ctx.store.is_none());
        assert!(!cache.exists(), "the rows of a workspace the account lost are still on disk");
        assert!(other.exists(), "another account's cache was touched");
        assert!(sees(&app, "Открыть Учёт времени") && !sees(&app, "Открыть Чеки"));
        assert!(sees(&app, "Доступ к разделу «Чеки» закрыт администратором"));
    }

    #[test]
    fn a_cache_left_by_an_earlier_run_goes_once_the_server_says_checks_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join(format!("cache-7-{}.db", Ctx::for_tests(dir.path()).host_key()));
        drop(Store::open(&cache).unwrap());
        // A server from before workspaces says nothing of the kind, and its silence removes nothing.
        let _ = resumed(&dir, older());
        assert!(cache.exists());
        let _ = resumed(&dir, account(&["checks", "time"], false));
        assert!(cache.exists());
        // The account lost Checks while the client was closed.
        let app = resumed(&dir, account(&["time"], false));
        assert!(!cache.exists());
        assert!(sees(&app, "Открыть Учёт времени"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_disabled_account_is_signed_out_and_told_why() {
        let dir = tempfile::tempdir().unwrap();
        for way_in in ["Транзакции и учёт чеков", "Табель и отметки сотрудников"] {
            let mut app = opened(&dir, account(&["checks", "time"], false), way_in);
            let _ = app.update(Message::Api(ApiEvent::AccountDisabled));
            assert!(on_the_start_window(&app), "{way_in}");
            assert!(app.ctx.user.is_none() && app.tx.is_none() && app.hosted.is_none() && app.ctx.store.is_none());
            assert!(sees(&app, "Учётная запись отключена администратором") && sees(&app, "Войти"));
            // Every request it had on its way says the same: it is told once.
            let _ = app.update(Message::Api(ApiEvent::AccountDisabled));
            assert_eq!(app.toasts.len(), 1);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_session_that_ends_closes_a_page_like_any_workspace() {
        let dir = tempfile::tempdir().unwrap();
        for (user, way_in) in [(account(&["time"], false), "Открыть Учёт времени"), (account(&[], true), "Открыть Администрирование CRM")] {
            let mut app = opened(&dir, user, way_in);
            let _ = app.update(Message::Api(ApiEvent::SessionRevoked));
            assert!(on_the_start_window(&app));
            assert!(app.ctx.user.is_none() && app.hosted.is_none());
            assert!(sees(&app, "Сессия завершена, войдите снова") && sees(&app, "Войти"));
            // Signed out, the window says nothing of what the last account could open.
            assert!(!sees(&app, "Учёт времени") && !sees(&app, "Администрирование CRM"));
        }
    }

    #[test]
    fn the_next_account_on_the_computer_gets_nothing_of_the_last_ones() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["time"], true), "Открыть Учёт времени");
        let _ = app.update(Message::LoggedOut);
        assert!(on_the_start_window(&app) && app.ctx.user.is_none() && app.hosted.is_none());

        // The next one has Checks only.
        let mut next = account(&["checks"], false);
        (next.id, next.username) = (8, "boris".into());
        let answer = LoginResponse { ok: true, user: Some(next), ..Default::default() };
        let _ = app.update(Message::Login(login::Msg::Answered(Ok(answer))));
        assert_eq!(app.workspace, Subsystem::Receipts, "the last account's workspace was chosen for the next");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert!(app.tx.is_some() && app.hosted.is_none());
        // Its cache is its own file.
        let anna = Ctx::for_tests(dir.path()).data_dir.join(format!("cache-7-{}.db", app.ctx.host_key()));
        assert!(!anna.exists());
        assert!(dir.path().join(format!("cache-8-{}.db", app.ctx.host_key())).exists());
        ask(&mut app, Subsystem::Time);
        assert!(app.hosted.is_none());
    }

    #[test]
    fn a_term_that_runs_out_at_work_closes_the_workspace_behind_the_same_door() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["time"], false), "Открыть Учёт времени");
        let _ = app.update(Message::Api(ApiEvent::AccessExpired));
        assert!(matches!(app.phase, Phase::Checking) && app.hosted.is_none());
        let _ = app.update(Message::Entitled(Entitled::Blocked(Some("2026-11-01".into()))));
        assert!(sees(&app, "Срок доступа истёк"));
        // Extended by an administrator: the same workspace opens again.
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        assert!(matches!(app.phase, Phase::Main) && app.hosted.is_some());
        assert_eq!(app.workspace, Subsystem::Time);
    }

    #[test]
    fn the_launch_password_stands_before_a_page_and_leads_back_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["time"], false), "Открыть Учёт времени");
        let _ = app.update(Message::Api(ApiEvent::LaunchRequired));
        assert!(matches!(app.phase, Phase::Launch(_)));
        assert!(app.hosted.is_some(), "the page waits behind the password");
        let unlocked = LaunchVerify { verified: true, session_token: Some("launch".into()), ..Default::default() };
        let _ = app.update(Message::Launch(gate::Msg::Answer(Ok(unlocked))));
        assert!(matches!(app.phase, Phase::Main));
        assert_eq!(app.workspace, Subsystem::Time);
        assert!(app.hosted.is_some() && app.tx.is_none(), "Checks was opened behind Time's password");

        // Asked for before the page existed: the workspace chosen opens, not Checks.
        let mut app = resumed(&dir, account(&["checks", "time"], true));
        press(&mut app, "Учётные записи и доступ");
        let _ = app.update(Message::Api(ApiEvent::LaunchRequired));
        let unlocked = LaunchVerify { verified: true, session_token: Some("launch".into()), ..Default::default() };
        let _ = app.update(Message::Launch(gate::Msg::Answer(Ok(unlocked))));
        assert!(matches!(app.phase, Phase::Main));
        assert_eq!(app.workspace, Subsystem::Admin);
        assert!(app.hosted.is_some() && app.tx.is_none());
    }

    #[test]
    fn a_page_the_launch_password_stood_before_is_started_after_it() {
        let dir = tempfile::tempdir().unwrap();
        // The password is asked for while the work window is on its way: the
        // page has not been started yet, and the window's arrival finds the
        // password in front of it.
        let mut app = resumed(&dir, account(&["time"], false));
        press(&mut app, "Открыть Учёт времени");
        let _ = app.update(Message::Entitled(Entitled::Allowed));
        let _ = app.update(Message::Api(ApiEvent::LaunchRequired));
        let _ = app.update(Message::WindowSwapped(false));
        assert!(matches!(app.phase, Phase::Launch(_)));
        assert!(app.hosted.as_ref().is_some_and(|page| !page.started()), "the page was started behind the password");
        let unlocked = LaunchVerify { verified: true, session_token: Some("launch".into()), ..Default::default() };
        let _ = app.update(Message::Launch(gate::Msg::Answer(Ok(unlocked))));
        assert!(matches!(app.phase, Phase::Main));
        assert!(app.hosted.as_ref().is_some_and(Hosted::started), "the page stays on «Загрузка…» for good");
    }

    #[test]
    fn an_older_answer_about_the_account_does_not_undo_a_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = resumed(&dir, account(&["checks", "time"], false));
        assert!(sees(&app, "Табель и отметки сотрудников"));
        // Asked twice, Time taken away in between; the answers arrive the other way round.
        app.account_asked = 2;
        let _ = app.update(Message::Account(2, Ok(account(&["checks"], false))));
        assert!(!sees(&app, "Табель и отметки сотрудников"));
        let _ = app.update(Message::Account(1, Ok(account(&["checks", "time"], false))));
        assert!(!sees(&app, "Табель и отметки сотрудников"), "the first answer, come last, gave Time back");
    }

    fn gone(folder: &std::path::Path) -> bool {
        for _ in 0..100 {
            if !folder.exists() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn what_the_pages_kept_goes_with_the_sign_in_and_with_the_access() {
        let dir = tempfile::tempdir().unwrap();
        // On the start window, no page open: signing out removes the folders all the same.
        let mut app = resumed(&dir, account(&["time"], true));
        let kept = hosted::folders(&app.ctx);
        for folder in &kept {
            std::fs::create_dir_all(folder).unwrap();
        }
        let _ = app.update(Message::LoggedOut);
        assert!(app.ctx.user.is_none() && kept.iter().all(|folder| gone(folder)));
        // Late news of the session that ended changes nothing for whoever is next.
        let asked = app.account_asked;
        let _ = app.update(Message::Api(ApiEvent::SessionRevoked));
        let _ = app.update(Message::Api(ApiEvent::AccountDisabled));
        assert!(app.toasts.is_empty() && app.account_asked == asked);

        // Time is taken away while its page is closed: its folder goes, the panel's stays.
        let mut app = resumed(&dir, account(&["checks", "time"], true));
        let (time, admin) = (hosted::folder(&app.ctx, hosted::Kind::Time), hosted::folder(&app.ctx, hosted::Kind::Admin));
        for folder in [&time, &admin] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let _ = app.update(Message::Account(app.account_asked, Ok(account(&["checks"], true))));
        assert!(gone(&time) && admin.exists());
    }

    #[test]
    fn a_refused_page_has_the_account_asked_about_and_decides_nothing_itself() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = opened(&dir, account(&["time"], false), "Открыть Учёт времени");
        let owner = app.hosted.as_ref().unwrap().owner();
        let lost = hosted::Msg::Signal(hosted::Signal::Said(owner, r#"{"type":"access-lost"}"#.into()));
        let _ = app.update(Message::Hosted(lost));
        // The page's word closes nothing: the server's does, when it is asked.
        assert!(matches!(app.phase, Phase::Main) && app.hosted.is_some());
        assert!(app.ctx.requests.is_empty(), "the question was not taken up: {:?}", app.ctx.requests);
    }
}
