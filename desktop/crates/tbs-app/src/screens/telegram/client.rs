//! The chats themselves: what the server said, what is open and what the
//! person is doing with it. Nothing is drawn here.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use iced::widget::operation::{self, RelativeOffset};
use iced::widget::{image, text_editor};
use iced::{Point, Task};
use tbs_core::api::{Api, ApiError, TgButtonSource, TgChat, TgChatPage, TgMenuButton, TgMessage, TgMessagePage, TgStatus, TgUnlock};

use super::dates::{self, Preset, Range};
use super::style::Work;
use crate::ctx::{Ctx, Request, text_error};
use crate::files;
use crate::theme::Tone;

pub const STREAM: &str = "tg-stream";
const CHATS: usize = 200;
const MESSAGES: usize = 100;
const STATUS_READY: Duration = Duration::from_secs(30);
const STATUS_WAITING: Duration = Duration::from_secs(4);
const CHATS_EVERY: Duration = Duration::from_secs(15);
const MESSAGES_EVERY: Duration = Duration::from_secs(6);
const PROCESS_POLL: Duration = Duration::from_millis(300);
const PROCESS_ATTEMPTS: usize = 30;
const MENU_BUTTON_LIFE: Duration = Duration::from_secs(300);
const PHOTOS_AT_ONCE: usize = 4;
const COPIED_FOR: Duration = Duration::from_millis(1400);
/// Closer to the newest message than this is "at the bottom".
const NEAR_BOTTOM: f32 = 80.0;
/// What the server takes as a document, and what the picker offers beside it.
const UPLOADS: [(&str, &[&str]); 2] = [("PDF", &["pdf"]), ("Изображения", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    Active,
    All,
    Hidden,
}

impl Filter {
    fn holds(self, hidden: bool) -> bool {
        match self {
            Filter::Active => !hidden,
            Filter::All => true,
            Filter::Hidden => hidden,
        }
    }
}

/// A step of Telegram's own sign-in that needs something typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Phone,
    Code,
    Password,
}

#[derive(Default)]
pub struct AuthForm {
    pub value: String,
    pub busy: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Chat(i64),
    Message(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordMode {
    Set,
    Remove,
}

pub struct PasswordForm {
    pub chat: i64,
    pub title: String,
    pub mode: PasswordMode,
    pub password: String,
    pub confirm: String,
    pub error: Option<String>,
    pub busy: bool,
}

/// The password of a protected chat, asked for before its messages.
#[derive(Default)]
pub struct LockForm {
    pub password: String,
    pub busy: bool,
    pub error: Option<String>,
}

pub struct PeriodForm {
    pub custom: bool,
    pub from: String,
    pub to: String,
    pub error: Option<String>,
}

/// Selected messages going to the receipt worker one after another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bulk {
    chat: i64,
    queue: VecDeque<i64>,
    current: Option<i64>,
    pub processed: usize,
    pub failed: usize,
    pub total: usize,
    pub busy: bool,
}

/// Chat, message, file.
pub type PhotoKey = (i64, i64, i64);

#[derive(Clone, Debug)]
pub enum Photo {
    Loading,
    Ready(image::Handle),
    Failed,
}

/// A mini app to open: where its button was and what it pointed at.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRequest {
    pub chat: i64,
    pub bot: Option<i64>,
    pub url: String,
    pub source: TgButtonSource,
    pub text: String,
}

/// What the chats cannot do by themselves and leave to the section.
#[derive(Clone, Debug, PartialEq)]
pub enum Want {
    App(AppRequest),
    Link(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    /// The chat is behind its password.
    pub locked: bool,
    /// Too many wrong passwords: nothing opens it for a while.
    pub barred: bool,
    pub text: String,
}

fn refusal(error: &ApiError) -> Refusal {
    let barred = error.status() == Some(423);
    let locked = barred || matches!(error, ApiError::Http { status: 403, code: Some(code), .. } if code == "chat_locked");
    Refusal { locked, barred, text: text_error(error) }
}

#[derive(Clone, Debug)]
pub enum Msg {
    Status(Result<TgStatus, String>),
    AuthInput(String),
    AuthSubmit,
    AuthDone(Result<TgStatus, String>),
    Chats(Result<TgChatPage, String>),
    Query(String),
    Filter(Filter),
    Open(i64),
    Back,
    Messages(i64, u64, Result<TgMessagePage, Refusal>),
    Draft(text_editor::Action),
    Send,
    Sent(i64, Result<(), String>),
    Attach,
    Attached(i64, Result<bool, String>),
    Process(i64),
    Processed(i64, i64, bool),
    ToggleSelecting,
    Pick(i64),
    RowPress(i64),
    SelectAll,
    SelectNone,
    CancelSelection,
    BulkStart,
    Monitor(i64, bool),
    Monitored(i64, bool, Result<(), String>),
    Hide(i64, bool),
    Hidden(i64, bool, Result<(), String>),
    Pointer(Point),
    Context(Target),
    CloseMenu,
    AskPassword(i64, PasswordMode),
    PasswordInput(String),
    PasswordConfirm(String),
    PasswordSubmit,
    PasswordDone(PasswordMode, Result<(), String>),
    ClosePassword,
    LockInput(String),
    Unlock,
    Unlocked(i64, Result<TgUnlock, String>),
    MenuButton(i64, Result<TgMenuButton, String>),
    MenuApp,
    Key(i64, usize, usize),
    CopiedOff(i64, usize, usize),
    TogglePeriod,
    Preset(Preset),
    PeriodCustom,
    PeriodFrom(String),
    PeriodTo(String),
    PeriodApply,
    ClearRange,
    Photo(PhotoKey, Result<image::Handle, String>),
    ViewPhoto(PhotoKey),
    CloseViewer,
    SavePhoto,
    Document(i64),
    Saved(Result<Option<String>, String>),
    Scrolled(f32),
    ToBottom,
    Command(bool),
}

pub struct Client {
    pub status: Option<TgStatus>,
    pub status_error: Option<String>,
    status_asked: Option<Instant>,
    status_busy: bool,
    pub auth: AuthForm,
    pub chats: Vec<TgChat>,
    pub chats_loaded: bool,
    pub chats_error: Option<String>,
    chats_asked: Option<Instant>,
    chats_busy: bool,
    /// Chats hidden or shown here that the server has not confirmed yet.
    hiding: HashMap<i64, bool>,
    pub filter: Filter,
    pub query: String,
    pub active: Option<i64>,
    /// The person went back to the list; no chat is opened for them.
    stay_on_list: bool,
    pub messages: Vec<TgMessage>,
    pub messages_loaded: bool,
    pub messages_error: Option<String>,
    messages_asked: Option<Instant>,
    messages_busy: bool,
    /// Moves with every change of chat: an answer for an earlier one is dropped.
    opening: u64,
    pub lock: Option<LockForm>,
    pub range: Range,
    pub period: Option<PeriodForm>,
    pub selecting: bool,
    pub selected: BTreeSet<i64>,
    processing: HashSet<(i64, i64)>,
    processed: HashSet<(i64, i64)>,
    failed: HashSet<(i64, i64)>,
    pub bulk: Option<Bulk>,
    pub draft: text_editor::Content,
    sending_text: bool,
    sending_file: bool,
    pub menu: Option<(Target, Point)>,
    pointer: Point,
    pub password: Option<PasswordForm>,
    menu_buttons: HashMap<i64, (Instant, TgMenuButton)>,
    pub photos: HashMap<PhotoKey, Photo>,
    photo_queue: VecDeque<PhotoKey>,
    photo_flights: usize,
    pub viewer: Option<PhotoKey>,
    pub saving: Option<i64>,
    pub copied: Option<(i64, usize, usize)>,
    /// The stream is scrolled away from its newest message.
    pub away: bool,
    command: bool,
    pub wants: Vec<Want>,
}

/// Oldest first, as a chat reads; the server sends the newest first.
fn in_order(mut items: Vec<TgMessage>) -> Vec<TgMessage> {
    items.sort_by_key(|message| {
        let id = message.id.unwrap_or(0);
        (message.date.as_deref().and_then(dates::local).map_or(id, |moment| moment.and_utc().timestamp_millis()), id)
    });
    items
}

/// Hands a message to the worker and waits for its verdict, as long as the
/// worker usually takes.
async fn run_process(api: Api, chat: i64, message: i64) -> bool {
    let Ok(started) = api.tg_process_message(chat, message).await else { return false };
    let mut state = started.status;
    let mut attempts = 0;
    while state != "done" && state != "failed" && attempts < PROCESS_ATTEMPTS {
        tokio::time::sleep(PROCESS_POLL).await;
        let Ok(rows) = api.tg_receipt_status(chat, &[message]).await else { return false };
        if let Some(row) = rows.into_iter().next().filter(|row| !row.status.is_empty()) {
            state = row.status;
        }
        attempts += 1;
    }
    state == "done"
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Self {
            status: None,
            status_error: None,
            status_asked: None,
            status_busy: false,
            auth: AuthForm::default(),
            chats: Vec::new(),
            chats_loaded: false,
            chats_error: None,
            chats_asked: None,
            chats_busy: false,
            hiding: HashMap::new(),
            filter: Filter::Active,
            query: String::new(),
            active: None,
            stay_on_list: false,
            messages: Vec::new(),
            messages_loaded: false,
            messages_error: None,
            messages_asked: None,
            messages_busy: false,
            opening: 0,
            lock: None,
            range: Range::default(),
            period: None,
            selecting: false,
            selected: BTreeSet::new(),
            processing: HashSet::new(),
            processed: HashSet::new(),
            failed: HashSet::new(),
            bulk: None,
            draft: text_editor::Content::new(),
            sending_text: false,
            sending_file: false,
            menu: None,
            pointer: Point::ORIGIN,
            password: None,
            menu_buttons: HashMap::new(),
            photos: HashMap::new(),
            photo_queue: VecDeque::new(),
            photo_flights: 0,
            viewer: None,
            saving: None,
            copied: None,
            away: false,
            command: false,
            wants: Vec::new(),
        }
    }

    pub fn ready(&self) -> bool {
        self.status.as_ref().is_some_and(TgStatus::is_ready)
    }

    pub fn step(&self) -> Option<Step> {
        match self.status.as_ref()?.state.as_str() {
            "wait_phone_number" => Some(Step::Phone),
            "wait_code" => Some(Step::Code),
            "wait_password" => Some(Step::Password),
            _ => None,
        }
    }

    pub fn hidden(&self, chat: &TgChat) -> bool {
        self.hiding.get(&chat.chat_id).copied().unwrap_or(chat.is_hidden)
    }

    /// All, active, hidden.
    pub fn counts(&self) -> (usize, usize, usize) {
        let hidden = self.chats.iter().filter(|chat| self.hidden(chat)).count();
        (self.chats.len(), self.chats.len() - hidden, hidden)
    }

    /// The chats of the open tab that the search leaves.
    pub fn listed(&self) -> Vec<&TgChat> {
        let needle = self.query.trim().to_lowercase();
        self.chats
            .iter()
            .filter(|chat| self.filter.holds(self.hidden(chat)))
            .filter(|chat| needle.is_empty() || chat.title.to_lowercase().contains(&needle) || chat.username.as_deref().is_some_and(|name| name.to_lowercase().contains(&needle)))
            .collect()
    }

    pub fn chat(&self, id: i64) -> Option<&TgChat> {
        self.chats.iter().find(|chat| chat.chat_id == id)
    }

    pub fn active_chat(&self) -> Option<&TgChat> {
        self.chat(self.active?)
    }

    /// The messages of the open chat that the chosen period leaves.
    pub fn shown(&self) -> Vec<&TgMessage> {
        self.messages.iter().filter(|message| self.range.holds(message.date.as_deref().and_then(dates::local))).collect()
    }

    /// How many of the shown messages can be ticked.
    pub fn selectable(&self) -> usize {
        self.shown().iter().filter(|message| message.id.is_some_and(|id| id != 0)).count()
    }

    pub fn in_flight(&self) -> usize {
        self.processing.len()
    }

    pub fn sending(&self) -> bool {
        self.sending_text || self.sending_file
    }

    pub fn bulk_busy(&self) -> bool {
        self.bulk.as_ref().is_some_and(|bulk| bulk.busy)
    }

    pub fn work_of(&self, chat: i64, message: &TgMessage) -> Work {
        let Some(id) = message.id else { return Work::Idle };
        let key = (chat, id);
        if self.processing.contains(&key) {
            Work::Busy
        } else if self.failed.contains(&key) {
            Work::Failed
        } else if self.processed.contains(&key) || message.processed == Some(true) {
            Work::Done
        } else if message.processed_status.as_deref() == Some("failed") {
            Work::Failed
        } else {
            Work::Idle
        }
    }

    /// The mini app on the open bot's menu button, when it has one.
    pub fn menu_app(&self, chat: i64) -> Option<&TgMenuButton> {
        self.menu_buttons.get(&chat).map(|(_, button)| button).filter(|button| button.available && button.url.as_deref().is_some_and(|url| !url.is_empty()))
    }

    /// Asks again for everything on screen: the section was opened or returned to.
    pub fn resume(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.status_asked = None;
        self.chats_asked = None;
        self.messages_asked = None;
        self.poll(ctx, Instant::now())
    }

    /// Whatever is due: the sign-in every half a minute once it stands and
    /// every few seconds until then, the chats and the open chat's messages.
    pub fn poll(&mut self, ctx: &Ctx, now: Instant) -> Task<Msg> {
        let due = |asked: Option<Instant>, every: Duration| asked.is_none_or(|at| now.saturating_duration_since(at) >= every);
        let mut tasks = Vec::new();
        if due(self.status_asked, if self.ready() { STATUS_READY } else { STATUS_WAITING }) {
            tasks.push(self.ask_status(ctx, now));
        }
        if self.ready() {
            if due(self.chats_asked, CHATS_EVERY) {
                tasks.push(self.ask_chats(ctx, now));
            }
            if due(self.messages_asked, MESSAGES_EVERY) {
                tasks.push(self.ask_messages(ctx, now));
            }
        }
        Task::batch(tasks)
    }

    fn ask_status(&mut self, ctx: &Ctx, now: Instant) -> Task<Msg> {
        if self.status_busy {
            return Task::none();
        }
        self.status_busy = true;
        self.status_asked = Some(now);
        let api = ctx.api.clone();
        Task::perform(async move { api.tg_status().await.map_err(|e| text_error(&e)) }, Msg::Status)
    }

    fn ask_chats(&mut self, ctx: &Ctx, now: Instant) -> Task<Msg> {
        if self.chats_busy {
            return Task::none();
        }
        self.chats_busy = true;
        self.chats_asked = Some(now);
        let api = ctx.api.clone();
        Task::perform(async move { api.tg_chats(CHATS, true).await.map_err(|e| text_error(&e)) }, Msg::Chats)
    }

    fn ask_messages(&mut self, ctx: &Ctx, now: Instant) -> Task<Msg> {
        let Some(chat) = self.active else { return Task::none() };
        if self.messages_busy || self.lock.is_some() {
            return Task::none();
        }
        self.messages_busy = true;
        self.messages_asked = Some(now);
        let (api, opening) = (ctx.api.clone(), self.opening);
        Task::perform(async move { api.tg_messages(chat, MESSAGES).await.map_err(|e| refusal(&e)) }, move |outcome| Msg::Messages(chat, opening, outcome))
    }

    /// At once when nothing is on its way; right after the answer that is.
    fn reload_messages(&mut self, chat: i64, ctx: &Ctx) -> Task<Msg> {
        if self.active != Some(chat) {
            return Task::none();
        }
        self.messages_asked = None;
        self.ask_messages(ctx, Instant::now())
    }

    fn reload_chats(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.chats_asked = None;
        self.ask_chats(ctx, Instant::now())
    }

    fn take_status(&mut self, status: TgStatus, ctx: &Ctx) -> Task<Msg> {
        let step = self.step();
        self.status_error = None;
        self.status = Some(status);
        if self.step() != step {
            // What was typed for one step is not the next step's, and not kept.
            self.auth.value.clear();
        }
        if self.ready() && !self.chats_loaded { self.ask_chats(ctx, Instant::now()) } else { Task::none() }
    }

    fn open(&mut self, chat: i64, ctx: &Ctx) -> Task<Msg> {
        if self.active == Some(chat) {
            return Task::none();
        }
        self.active = Some(chat);
        self.stay_on_list = false;
        self.opening += 1;
        self.messages.clear();
        self.messages_loaded = false;
        self.messages_error = None;
        self.messages_busy = false;
        self.lock = None;
        self.selecting = false;
        self.selected.clear();
        self.range = Range::default();
        self.period = None;
        self.menu = None;
        self.viewer = None;
        self.away = false;
        self.photos.clear();
        self.photo_queue.clear();
        Task::batch([self.ask_messages(ctx, Instant::now()), self.ask_menu_button(ctx), operation::snap_to(STREAM, RelativeOffset::START)])
    }

    fn ask_menu_button(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Some(chat) = self.active_chat().filter(|chat| chat.chat_type == "bot").map(|chat| chat.chat_id) else { return Task::none() };
        if self.menu_buttons.get(&chat).is_some_and(|(asked, _)| asked.elapsed() < MENU_BUTTON_LIFE) {
            return Task::none();
        }
        self.menu_buttons.insert(chat, (Instant::now(), TgMenuButton::default()));
        let api = ctx.api.clone();
        Task::perform(async move { api.tg_menu_button(chat).await.map_err(|e| text_error(&e)) }, move |outcome| Msg::MenuButton(chat, outcome))
    }

    /// Photos of the open chat, the newest first and a few at a time.
    fn want_photos(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Some(chat) = self.active else { return Task::none() };
        let wanted: Vec<PhotoKey> = self.messages.iter().rev().filter_map(|message| Some((chat, message.id.filter(|id| *id != 0)?, message.photo.as_ref()?.file_id?))).collect();
        self.photos.retain(|key, _| wanted.contains(key));
        self.photo_queue.retain(|key| wanted.contains(key));
        for key in wanted {
            if !self.photos.contains_key(&key) {
                self.photos.insert(key, Photo::Loading);
                self.photo_queue.push_back(key);
            }
        }
        self.pump_photos(ctx)
    }

    fn pump_photos(&mut self, ctx: &Ctx) -> Task<Msg> {
        let mut tasks = Vec::new();
        while self.photo_flights < PHOTOS_AT_ONCE
            && let Some(key) = self.photo_queue.pop_front()
        {
            self.photo_flights += 1;
            let api = ctx.api.clone();
            let name = self.messages.iter().find(|message| message.id == Some(key.1)).and_then(|message| message.photo.as_ref()).and_then(|photo| photo.file_name.clone());
            tasks.push(Task::perform(
                async move { api.tg_file(key.0, key.1, key.2, name.as_deref()).await.map(|file| image::Handle::from_bytes(file.bytes)).map_err(|e| text_error(&e)) },
                move |outcome| Msg::Photo(key, outcome),
            ));
        }
        Task::batch(tasks)
    }

    fn start_process(&mut self, chat: i64, message: i64, ctx: &Ctx) -> Task<Msg> {
        if message == 0 || !self.processing.insert((chat, message)) {
            return Task::none();
        }
        self.failed.remove(&(chat, message));
        Task::perform(run_process(ctx.api.clone(), chat, message), move |done| Msg::Processed(chat, message, done))
    }

    /// The next of the selected messages, or the end of them.
    fn bulk_step(&mut self, chat: i64, message: i64, done: bool, ctx: &mut Ctx) -> Task<Msg> {
        let Some(bulk) = self.bulk.as_mut().filter(|bulk| bulk.busy && bulk.chat == chat && bulk.current == Some(message)) else { return Task::none() };
        if done {
            bulk.processed += 1;
        } else {
            bulk.failed += 1;
        }
        bulk.current = bulk.queue.pop_front();
        if let Some(next) = bulk.current {
            return self.start_process(chat, next, ctx);
        }
        bulk.busy = false;
        let (processed, failed) = (bulk.processed, bulk.failed);
        if failed > 0 {
            ctx.toast(Tone::Warning, format!("Готово: {processed}, ошибок: {failed}"));
        } else {
            ctx.toast(Tone::Success, format!("Готово: {processed}"));
        }
        if processed > 0 {
            ctx.request(Request::TransactionsChanged);
        }
        self.reload_messages(chat, ctx)
    }

    fn toggle(&mut self, message: i64) {
        if message == 0 {
            return;
        }
        self.selecting = true;
        if !self.selected.remove(&message) {
            self.selected.insert(message);
        }
    }

    fn cancel_selection(&mut self) {
        if self.bulk_busy() {
            return;
        }
        self.selecting = false;
        self.selected.clear();
        self.bulk = None;
    }

    /// Escape puts away the topmost thing; `false` when there was nothing.
    pub fn escape(&mut self) -> bool {
        if self.viewer.take().is_some() || self.menu.take().is_some() {
            return true;
        }
        if let Some(form) = &self.password {
            if !form.busy {
                self.password = None;
            }
            return true;
        }
        if self.period.take().is_some() {
            return true;
        }
        if self.selecting {
            self.cancel_selection();
            return true;
        }
        false
    }

    /// Whether anything is drawn over the chats.
    pub fn layered(&self) -> bool {
        self.viewer.is_some() || self.menu.is_some() || self.password.is_some() || self.period.is_some()
    }

    fn submit_password(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Some(form) = self.password.as_mut().filter(|form| !form.busy) else { return Task::none() };
        let password = form.password.trim().to_owned();
        let problem = if password.is_empty() {
            Some("Введите пароль")
        } else if form.mode == PasswordMode::Set && password.chars().count() < 4 {
            Some("Минимум 4 символа")
        } else if form.mode == PasswordMode::Set && password != form.confirm.trim() {
            Some("Пароли не совпадают")
        } else {
            None
        };
        if let Some(problem) = problem {
            form.error = Some(problem.to_owned());
            return Task::none();
        }
        form.error = None;
        form.busy = true;
        let (api, chat, mode) = (ctx.api.clone(), form.chat, form.mode);
        Task::perform(
            async move {
                let outcome = match mode {
                    PasswordMode::Set => api.tg_set_chat_password(chat, &password).await,
                    PasswordMode::Remove => api.tg_remove_chat_password(chat, &password).await,
                };
                outcome.map(|_| ()).map_err(|e| text_error(&e))
            },
            move |outcome| Msg::PasswordDone(mode, outcome),
        )
    }

    fn apply_period(&mut self) {
        let Some(form) = &mut self.period else { return };
        let (Ok(from), Ok(to)) = (dates::read_day(&form.from), dates::read_day(&form.to)) else {
            form.error = Some("Дата в формате ДД.ММ.ГГГГ".to_owned());
            return;
        };
        if from.is_none() && to.is_none() {
            return;
        }
        if let (Some(from), Some(to)) = (from, to)
            && from > to
        {
            form.error = Some("Начало периода позже его конца".to_owned());
            return;
        }
        self.range = Range { from, to };
        self.period = None;
    }

    fn press_key(&mut self, message: i64, row: usize, col: usize) -> Task<Msg> {
        let Some(chat) = self.active else { return Task::none() };
        let Some(found) = self.messages.iter().find(|candidate| candidate.id.unwrap_or(0) == message) else { return Task::none() };
        let Some(markup) = &found.reply_markup else { return Task::none() };
        let Some(button) = markup.rows.get(row).and_then(|line| line.get(col)) else { return Task::none() };
        let url = button.url.clone().filter(|url| !url.is_empty());
        match (button.kind.as_str(), url) {
            ("web_app", Some(url)) => {
                let source = if markup.kind == "keyboard" { TgButtonSource::Keyboard } else { TgButtonSource::Inline };
                self.wants.push(Want::App(AppRequest { chat, bot: found.sender(), url, source, text: button.text.clone() }));
            }
            ("url" | "login_url", Some(url)) => self.wants.push(Want::Link(url)),
            ("copy_text", url) => {
                let text = url.unwrap_or_else(|| button.text.clone());
                self.copied = Some((message, row, col));
                return Task::batch([iced::clipboard::write(text), Task::perform(tokio::time::sleep(COPIED_FOR), move |_| Msg::CopiedOff(message, row, col))]);
            }
            _ => {}
        }
        Task::none()
    }

    fn save_document(&mut self, message: i64, ctx: &Ctx) -> Task<Msg> {
        let Some(chat) = self.active else { return Task::none() };
        if self.saving.is_some() {
            return Task::none();
        }
        let Some(document) = self.messages.iter().find(|candidate| candidate.id == Some(message)).and_then(|found| found.document.as_ref()) else { return Task::none() };
        let Some(file) = document.file_id else { return Task::none() };
        let name = document.file_name.clone().filter(|name| !name.is_empty()).unwrap_or_else(|| "document".to_owned());
        self.saving = Some(message);
        let api = ctx.api.clone();
        Task::perform(
            async move {
                let download = api.tg_file(chat, message, file, Some(&name)).await.map_err(|e| text_error(&e))?;
                files::save_any(name, download.bytes).await
            },
            Msg::Saved,
        )
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Status(outcome) => {
                self.status_busy = false;
                match outcome {
                    Ok(status) => return self.take_status(status, ctx),
                    Err(reason) => self.status_error = Some(reason),
                }
            }
            Msg::AuthInput(value) => {
                if !self.auth.busy {
                    self.auth.value = value;
                }
            }
            Msg::AuthSubmit => {
                let Some(step) = self.step() else { return Task::none() };
                let value = self.auth.value.trim().to_owned();
                if value.is_empty() || self.auth.busy {
                    return Task::none();
                }
                self.auth.busy = true;
                let api = ctx.api.clone();
                return Task::perform(
                    async move {
                        let outcome = match step {
                            Step::Phone => api.tg_send_phone(&value).await,
                            Step::Code => api.tg_send_code(&value).await,
                            Step::Password => api.tg_send_password(&value).await,
                        };
                        outcome.map_err(|e| text_error(&e))
                    },
                    Msg::AuthDone,
                );
            }
            Msg::AuthDone(outcome) => {
                self.auth.busy = false;
                match outcome {
                    Ok(status) => {
                        self.auth.error = None;
                        // TDLib moves on by itself; the next look is not half a minute away.
                        self.status_asked = None;
                        return self.take_status(status, ctx);
                    }
                    Err(reason) => self.auth.error = Some(reason),
                }
            }
            Msg::Chats(outcome) => {
                self.chats_busy = false;
                match outcome {
                    Ok(page) => {
                        self.chats = page.items;
                        self.chats_loaded = true;
                        self.chats_error = None;
                        let first = self.chats.iter().find(|chat| self.filter.holds(self.hidden(chat))).map(|chat| chat.chat_id);
                        if self.active.is_none()
                            && !self.stay_on_list
                            && let Some(first) = first
                        {
                            return self.open(first, ctx);
                        }
                        return self.ask_menu_button(ctx);
                    }
                    Err(reason) => self.chats_error = Some(reason),
                }
            }
            Msg::Query(value) => self.query = value,
            Msg::Filter(filter) => self.filter = filter,
            Msg::Open(chat) => return self.open(chat, ctx),
            Msg::Back => {
                self.active = None;
                self.stay_on_list = true;
                self.opening += 1;
                self.messages.clear();
                self.selecting = false;
                self.selected.clear();
            }
            Msg::Messages(chat, opening, outcome) => {
                if self.active != Some(chat) || opening != self.opening {
                    return Task::none();
                }
                self.messages_busy = false;
                match outcome {
                    Ok(page) => {
                        self.lock = None;
                        self.messages_error = None;
                        self.messages_loaded = true;
                        self.messages = in_order(page.items);
                        return self.want_photos(ctx);
                    }
                    Err(refusal) if refusal.locked => {
                        self.messages.clear();
                        self.messages_loaded = true;
                        let form = self.lock.get_or_insert_with(LockForm::default);
                        if refusal.barred {
                            form.error = Some("Слишком много неверных попыток. Чат временно заблокирован.".to_owned());
                        }
                    }
                    Err(refusal) => self.messages_error = Some(refusal.text),
                }
            }
            Msg::Draft(action) => self.draft.perform(action),
            Msg::Send => {
                let Some(chat) = self.active else { return Task::none() };
                let text = self.draft.text().trim().to_owned();
                if text.is_empty() || self.sending() || self.selecting || self.lock.is_some() {
                    return Task::none();
                }
                self.sending_text = true;
                self.draft = text_editor::Content::new();
                self.away = false;
                let api = ctx.api.clone();
                return Task::batch([
                    Task::perform(async move { api.tg_send_message(chat, &text).await.map(|_| ()).map_err(|e| text_error(&e)) }, move |outcome| Msg::Sent(chat, outcome)),
                    operation::snap_to(STREAM, RelativeOffset::START),
                ]);
            }
            Msg::Sent(chat, outcome) => {
                self.sending_text = false;
                match outcome {
                    Ok(()) => return self.reload_messages(chat, ctx),
                    Err(reason) => ctx.toast(Tone::Danger, reason),
                }
            }
            Msg::Attach => {
                let Some(chat) = self.active else { return Task::none() };
                if self.sending() || self.lock.is_some() {
                    return Task::none();
                }
                self.sending_file = true;
                let api = ctx.api.clone();
                return Task::perform(
                    async move {
                        let Some(file) = files::pick_of(&UPLOADS).await? else { return Ok(false) };
                        api.tg_send_document(chat, &file.name, file.bytes).await.map(|_| true).map_err(|e| text_error(&e))
                    },
                    move |outcome| Msg::Attached(chat, outcome),
                );
            }
            Msg::Attached(chat, outcome) => {
                self.sending_file = false;
                match outcome {
                    Ok(true) => {
                        ctx.toast(Tone::Success, "Файл отправлен");
                        self.away = false;
                        return Task::batch([self.reload_messages(chat, ctx), operation::snap_to(STREAM, RelativeOffset::START)]);
                    }
                    Ok(false) => {}
                    Err(reason) => ctx.toast(Tone::Danger, reason),
                }
            }
            Msg::Process(message) => {
                self.menu = None;
                let Some(chat) = self.active else { return Task::none() };
                return self.start_process(chat, message, ctx);
            }
            Msg::Processed(chat, message, done) => {
                let key = (chat, message);
                self.processing.remove(&key);
                if done {
                    self.processed.insert(key);
                } else {
                    self.processed.remove(&key);
                    self.failed.insert(key);
                }
                if self.bulk.as_ref().is_some_and(|bulk| bulk.busy && bulk.chat == chat && bulk.current == Some(message)) {
                    return self.bulk_step(chat, message, done, ctx);
                }
                if done {
                    ctx.request(Request::TransactionsChanged);
                }
                return self.reload_messages(chat, ctx);
            }
            Msg::ToggleSelecting => {
                if self.selecting {
                    self.cancel_selection();
                } else {
                    self.selecting = true;
                }
            }
            Msg::Pick(message) => {
                self.menu = None;
                self.toggle(message);
            }
            Msg::RowPress(message) => {
                if self.selecting || self.command {
                    self.toggle(message);
                }
            }
            Msg::SelectAll => {
                let ids: BTreeSet<i64> = self.shown().iter().filter_map(|message| message.id.filter(|id| *id != 0)).collect();
                if !ids.is_empty() {
                    self.selecting = true;
                    self.selected = ids;
                }
            }
            Msg::SelectNone => self.selected.clear(),
            Msg::CancelSelection => self.cancel_selection(),
            Msg::BulkStart => {
                let Some(chat) = self.active else { return Task::none() };
                if self.bulk_busy() || self.selected.is_empty() {
                    return Task::none();
                }
                let mut queue: VecDeque<i64> = self.selected.iter().copied().collect();
                let first = queue.pop_front();
                self.bulk = Some(Bulk { chat, total: self.selected.len(), queue, current: first, processed: 0, failed: 0, busy: true });
                if let Some(first) = first {
                    return self.start_process(chat, first, ctx);
                }
            }
            Msg::Monitor(chat, enabled) => {
                let api = ctx.api.clone();
                return Task::perform(async move { api.tg_set_monitor(chat, enabled).await.map(|_| ()).map_err(|e| text_error(&e)) }, move |outcome| Msg::Monitored(chat, enabled, outcome));
            }
            Msg::Monitored(chat, enabled, outcome) => match outcome {
                Ok(()) => {
                    if let Some(found) = self.chats.iter_mut().find(|candidate| candidate.chat_id == chat) {
                        found.monitor_enabled = enabled;
                        found.is_monitored |= enabled;
                    }
                    ctx.toast(Tone::Success, if enabled { "Мониторинг включён" } else { "Мониторинг выключен" });
                    return self.reload_chats(ctx);
                }
                Err(reason) => ctx.toast(Tone::Danger, reason),
            },
            Msg::Hide(chat, hidden) => {
                self.menu = None;
                // Shown at once; the server's word settles it either way.
                self.hiding.insert(chat, hidden);
                let api = ctx.api.clone();
                return Task::perform(async move { api.tg_set_hidden(chat, hidden).await.map(|_| ()).map_err(|e| text_error(&e)) }, move |outcome| Msg::Hidden(chat, hidden, outcome));
            }
            Msg::Hidden(chat, hidden, outcome) => {
                // A later choice for the same chat is still on its way.
                if self.hiding.get(&chat) == Some(&hidden) {
                    self.hiding.remove(&chat);
                }
                match outcome {
                    Ok(()) => {
                        if let Some(found) = self.chats.iter_mut().find(|candidate| candidate.chat_id == chat) {
                            found.is_hidden = hidden;
                        }
                        ctx.toast(Tone::Success, if hidden { "Чат скрыт" } else { "Чат показан" });
                    }
                    Err(reason) => ctx.toast(Tone::Danger, reason),
                }
            }
            Msg::Pointer(at) => self.pointer = at,
            Msg::Context(target) => self.menu = Some((target, self.pointer)),
            Msg::CloseMenu => self.menu = None,
            Msg::AskPassword(chat, mode) => {
                self.menu = None;
                let title = self.chat(chat).map(|found| found.title.clone()).unwrap_or_default();
                self.password = Some(PasswordForm { chat, title, mode, password: String::new(), confirm: String::new(), error: None, busy: false });
            }
            Msg::PasswordInput(value) => {
                if let Some(form) = self.password.as_mut().filter(|form| !form.busy) {
                    form.password = value;
                }
            }
            Msg::PasswordConfirm(value) => {
                if let Some(form) = self.password.as_mut().filter(|form| !form.busy) {
                    form.confirm = value;
                }
            }
            Msg::PasswordSubmit => return self.submit_password(ctx),
            Msg::PasswordDone(mode, outcome) => match outcome {
                Ok(()) => {
                    self.password = None;
                    ctx.toast(Tone::Success, if mode == PasswordMode::Set { "Пароль установлен" } else { "Пароль снят" });
                    return self.reload_chats(ctx);
                }
                Err(reason) => {
                    if let Some(form) = &mut self.password {
                        form.busy = false;
                        form.error = Some(reason);
                    }
                }
            },
            Msg::ClosePassword => {
                if self.password.as_ref().is_some_and(|form| !form.busy) {
                    self.password = None;
                }
            }
            Msg::LockInput(value) => {
                if let Some(form) = self.lock.as_mut().filter(|form| !form.busy) {
                    form.password = value;
                }
            }
            Msg::Unlock => {
                let Some(chat) = self.active else { return Task::none() };
                let Some(form) = self.lock.as_mut().filter(|form| !form.busy && !form.password.trim().is_empty()) else { return Task::none() };
                form.busy = true;
                form.error = None;
                let (api, password) = (ctx.api.clone(), form.password.trim().to_owned());
                return Task::perform(async move { api.tg_unlock_chat(chat, &password).await.map_err(|e| text_error(&e)) }, move |outcome| Msg::Unlocked(chat, outcome));
            }
            Msg::Unlocked(chat, outcome) => {
                if self.active != Some(chat) {
                    return Task::none();
                }
                let Some(form) = &mut self.lock else { return Task::none() };
                form.busy = false;
                match outcome {
                    Ok(answer) if answer.verified => {
                        self.lock = None;
                        self.messages_loaded = false;
                        return Task::batch([self.reload_messages(chat, ctx), self.ask_menu_button(ctx)]);
                    }
                    Ok(answer) if answer.locked => form.error = Some("Слишком много неверных попыток. Чат временно заблокирован.".to_owned()),
                    Ok(answer) => {
                        form.password.clear();
                        form.error = Some(match answer.attempts_left {
                            Some(left) => format!("Неверный пароль (осталось попыток: {left})"),
                            None => "Неверный пароль".to_owned(),
                        });
                    }
                    Err(reason) => form.error = Some(reason),
                }
            }
            Msg::MenuButton(chat, outcome) => {
                // A refusal is remembered as "none" until it ages, not asked again at once.
                if let Ok(button) = outcome {
                    self.menu_buttons.insert(chat, (Instant::now(), button));
                }
            }
            Msg::MenuApp => {
                if let Some(chat) = self.active
                    && let Some(button) = self.menu_app(chat)
                    && let Some(url) = button.url.clone()
                {
                    let text = button.text.clone().filter(|text| !text.is_empty()).unwrap_or_else(|| "Открыть приложение".to_owned());
                    let bot = button.bot_user_id;
                    self.wants.push(Want::App(AppRequest { chat, bot, url, source: TgButtonSource::Menu, text }));
                }
            }
            Msg::Key(message, row, col) => return self.press_key(message, row, col),
            Msg::CopiedOff(message, row, col) => {
                if self.copied == Some((message, row, col)) {
                    self.copied = None;
                }
            }
            Msg::TogglePeriod => {
                self.period = match self.period {
                    Some(_) => None,
                    None => {
                        let today = dates::now().date();
                        let custom = self.range.is_set() && !Preset::ALL.into_iter().any(|preset| preset.range(today) == self.range);
                        Some(PeriodForm { custom, from: dates::typed(self.range.from), to: dates::typed(self.range.to), error: None })
                    }
                };
            }
            Msg::Preset(preset) => {
                self.range = preset.range(dates::now().date());
                self.period = None;
            }
            Msg::PeriodCustom => {
                if let Some(form) = &mut self.period {
                    form.custom = !form.custom;
                }
            }
            Msg::PeriodFrom(value) => {
                if let Some(form) = &mut self.period {
                    form.from = value;
                    form.error = None;
                }
            }
            Msg::PeriodTo(value) => {
                if let Some(form) = &mut self.period {
                    form.to = value;
                    form.error = None;
                }
            }
            Msg::PeriodApply => self.apply_period(),
            Msg::ClearRange => {
                self.range = Range::default();
                self.period = None;
            }
            Msg::Photo(key, outcome) => {
                self.photo_flights = self.photo_flights.saturating_sub(1);
                if let Some(slot) = self.photos.get_mut(&key) {
                    *slot = outcome.map_or(Photo::Failed, Photo::Ready);
                }
                return self.pump_photos(ctx);
            }
            Msg::ViewPhoto(key) => {
                if self.selecting || self.command {
                    self.toggle(key.1);
                } else if matches!(self.photos.get(&key), Some(Photo::Ready(_))) {
                    self.viewer = Some(key);
                }
            }
            Msg::CloseViewer => self.viewer = None,
            Msg::SavePhoto => {
                if let Some(key) = self.viewer
                    && let Some(Photo::Ready(image::Handle::Bytes(_, bytes))) = self.photos.get(&key)
                {
                    let name = self.messages.iter().find(|message| message.id == Some(key.1)).and_then(|message| message.photo.as_ref()).and_then(|photo| photo.file_name.clone());
                    return Task::perform(files::save_any(name.unwrap_or_else(|| "photo.jpg".to_owned()), bytes.to_vec()), Msg::Saved);
                }
            }
            Msg::Document(message) => {
                if self.selecting || self.command {
                    self.toggle(message);
                } else {
                    return self.save_document(message, ctx);
                }
            }
            Msg::Saved(outcome) => {
                self.saving = None;
                match outcome {
                    Ok(Some(path)) => ctx.toast(Tone::Success, format!("Файл сохранён: {path}")),
                    Ok(None) => {}
                    Err(reason) => ctx.toast(Tone::Danger, format!("Не удалось сохранить файл: {reason}")),
                }
            }
            Msg::Scrolled(distance) => self.away = distance > NEAR_BOTTOM && !self.messages.is_empty(),
            Msg::ToBottom => {
                self.away = false;
                return operation::snap_to(STREAM, RelativeOffset::START);
            }
            Msg::Command(held) => self.command = held,
        }
        Task::none()
    }
}
