//! What every screen may touch: the session, local data and the app-level
//! requests a screen cannot carry out by itself.

use std::path::PathBuf;
use std::sync::Arc;

use iced::Size;
use tbs_core::api::Api;
use tbs_core::config::{Preferences, save_json};
use tbs_core::model::UserInfo;
use tbs_core::store::Store;

use crate::screens::login::Subsystem;
use crate::theme::Tone;

/// Things a screen asks the application shell to do.
#[derive(Clone, Debug)]
pub enum Request {
    SignedIn(UserInfo),
    /// The start window: open this workspace on the session there is.
    Open(Subsystem),
    SignOut,
    /// The saved theme choice changed.
    ThemeChanged,
    /// Local data changed in a way other screens should notice.
    TransactionsChanged,
    /// The server refused a workspace's page: ask it what the account may
    /// still open.
    AccessChanged,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SyncState {
    #[default]
    Idle,
    Syncing,
    Error(String),
}

pub struct Ctx {
    pub api: Api,
    pub user: Option<UserInfo>,
    pub store: Option<Arc<Store>>,
    pub prefs: Preferences,
    pub data_dir: PathBuf,
    pub dark: bool,
    pub online: bool,
    pub sync: SyncState,
    pub toasts: Vec<(Tone, String)>,
    pub requests: Vec<Request>,
    /// Keeps menus drawn at a pointer inside the window.
    pub window: Size,
    /// The width the navigation rail takes on the left of the work window:
    /// nothing in a workspace that is a page, which has a bar instead.
    pub rail: f32,
    /// The height the bar of such a workspace takes at the top of the work
    /// window: nothing in Checks, which has the rail.
    pub bar: f32,
}

impl Ctx {
    #[cfg(test)]
    pub fn for_tests(dir: &std::path::Path) -> Self {
        let vault = Arc::new(tbs_core::secret::MemoryStore::default());
        Self {
            api: Api::new("http://127.0.0.1:9".to_owned(), None, vault),
            user: None,
            store: None,
            prefs: Preferences::default(),
            data_dir: dir.to_path_buf(),
            dark: false,
            online: true,
            sync: SyncState::Idle,
            toasts: Vec::new(),
            requests: Vec::new(),
            window: Size::new(1360.0, 860.0),
            rail: crate::app::RAIL_WIDTH,
            bar: 0.0,
        }
    }

    pub fn toast(&mut self, tone: Tone, message: impl Into<String>) {
        self.toasts.push((tone, message.into()));
    }

    pub fn request(&mut self, request: Request) {
        self.requests.push(request);
    }

    pub fn is_admin(&self) -> bool {
        self.user.as_ref().is_some_and(|u| u.is_admin())
    }

    /// May manage the CRM's accounts, which is not the same as running Checks.
    pub fn manages_accounts(&self) -> bool {
        self.user.as_ref().is_some_and(|u| u.manages_accounts())
    }

    pub fn save_prefs(&self) {
        if let Err(error) = save_json(&self.data_dir.join("preferences.json"), &self.prefs) {
            tracing::warn!("preferences not saved: {error}");
        }
    }

    /// A short key that keeps one user's saved state away from another's.
    pub fn user_key(&self) -> String {
        match &self.user {
            Some(user) => format!("u{}", user.id),
            None => "anonymous".into(),
        }
    }

    /// The server's address as a file name: ids are one server's own.
    pub fn host_key(&self) -> String {
        self.api.base().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
    }

    pub fn state_path(&self, name: &str) -> PathBuf {
        self.data_dir.join(format!("{}-{name}.json", self.user_key()))
    }
}

/// Errors cross message boundaries as text: messages must be `Clone`, and the
/// user only ever sees the message anyway.
pub fn text_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}
