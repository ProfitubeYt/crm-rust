//! The chats themselves: what the server said, what is open and what the
//! person is doing with it. Nothing is drawn here.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use iced::widget::operation::{self, RelativeOffset};
use iced::widget::{image, text_editor};
use iced::{Point, Task};
use tbs_core::api::{Api, TgButtonSource, TgChat, TgMenuButton, TgMessage, TgStatus};

mod model;
mod side;

pub use model::*;

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
            other => return self.update_side(other, ctx),
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests;
