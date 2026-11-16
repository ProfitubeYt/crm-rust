//! What the chats are made of and what can happen to them.

use std::collections::VecDeque;

use iced::Point;
use iced::widget::{image, text_editor};
use tbs_core::api::{ApiError, TgButtonSource, TgChatPage, TgMenuButton, TgMessagePage, TgStatus, TgUnlock};

use super::super::dates::Preset;
use crate::ctx::text_error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    Active,
    All,
    Hidden,
}

impl Filter {
    pub(super) fn holds(self, hidden: bool) -> bool {
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
    pub(super) chat: i64,
    pub(super) queue: VecDeque<i64>,
    pub(super) current: Option<i64>,
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

pub(super) fn refusal(error: &ApiError) -> Refusal {
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
