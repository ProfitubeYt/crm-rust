//! The bookkeeping workbook on disk: where it is and whether it changed.
//!
//! Excel replaces the file on save and holds an exclusive lock while the book
//! is open, so a plain periodic look at size and time is the sturdiest signal,
//! and reading has to wait the lock out instead of failing.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use tbs_core::config::{load_json, save_json};
use tbs_core::updates::sha256_hex;

const STATE_FILE: &str = "annotation-watcher.json";
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
const LOCK_RETRY: Duration = Duration::from_millis(750);
const LOCK_MAX_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct WatchState {
    pub file_path: Option<PathBuf>,
    pub enabled: bool,
    pub last_sha256: Option<String>,
    pub last_run_at: Option<String>,
}

impl WatchState {
    pub fn load(dir: &Path) -> Self {
        load_json(&dir.join(STATE_FILE))
    }

    pub fn save(&self, dir: &Path) {
        if let Err(error) = save_json(&dir.join(STATE_FILE), self) {
            tracing::warn!("watcher state not saved: {error}");
        }
    }

    pub fn file_name(&self) -> Option<String> {
        self.file_path.as_deref()?.file_name().map(|name| name.to_string_lossy().into_owned())
    }
}

/// Changes whenever Excel writes the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signature {
    modified: Option<SystemTime>,
    size: u64,
}

pub fn signature(path: &Path) -> Option<Signature> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Signature { modified: meta.modified().ok(), size: meta.len() })
}

#[derive(Clone, Debug)]
pub struct Book {
    pub name: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

/// Blocking: run it off the UI thread.
pub fn read_waiting(path: &Path) -> Result<Book, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "book.xlsx".to_owned());
    let deadline = Instant::now() + LOCK_MAX_WAIT;
    loop {
        let attempt = std::fs::metadata(path).and_then(|meta| {
            if meta.len() > MAX_FILE_BYTES {
                return Err(std::io::Error::other("too_large"));
            }
            std::fs::read(path)
        });
        match attempt {
            Ok(bytes) => {
                let sha256 = sha256_hex(&bytes);
                return Ok(Book { name, bytes, sha256 });
            }
            Err(error) if error.to_string() == "too_large" => return Err("Файл больше 25 МБ.".to_owned()),
            Err(_) if Instant::now() < deadline => std::thread::sleep(LOCK_RETRY),
            Err(error) => return Err(format!("Файл недоступен: {error}")),
        }
    }
}

/// Decides when a run of file changes has settled.
#[derive(Debug, Default)]
pub struct Settling {
    last: Option<Signature>,
    dirty: bool,
}

impl Settling {
    /// Anything may have happened while nobody was looking.
    pub fn startup() -> Self {
        Self { last: None, dirty: true }
    }

    /// `true` once the file stopped changing after it had changed.
    pub fn observe(&mut self, seen: Option<Signature>) -> bool {
        let Some(seen) = seen else { return false };
        match self.last.replace(seen) {
            Some(before) if before != seen => {
                self.dirty = true;
                false
            }
            Some(_) if self.dirty => {
                self.dirty = false;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(size: u64) -> Option<Signature> {
        Some(Signature { modified: None, size })
    }

    #[test]
    fn a_change_fires_only_once_the_file_is_quiet() {
        let mut settling = Settling::default();
        assert!(!settling.observe(at(1)));
        assert!(!settling.observe(at(1)));
        assert!(!settling.observe(at(2)));
        assert!(!settling.observe(at(3)));
        assert!(settling.observe(at(3)));
        assert!(!settling.observe(at(3)));
    }

    #[test]
    fn a_startup_check_looks_at_the_file_once() {
        let mut settling = Settling::startup();
        assert!(!settling.observe(at(1)));
        assert!(settling.observe(at(1)));
        assert!(!settling.observe(at(1)));
    }

    #[test]
    fn a_vanished_file_is_mid_save_not_a_change() {
        let mut settling = Settling::default();
        assert!(!settling.observe(at(1)));
        assert!(!settling.observe(None));
        assert!(!settling.observe(at(1)));
    }

    #[test]
    fn the_watch_state_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let state = WatchState { file_path: Some("C:\\book.xlsm".into()), enabled: true, last_sha256: Some("ab".into()), last_run_at: None };
        state.save(dir.path());
        assert_eq!(WatchState::load(dir.path()), state);
        assert_eq!(state.file_name().as_deref(), Some("book.xlsm"));
    }

    #[test]
    fn reading_hashes_the_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.xlsx");
        std::fs::write(&path, b"abc").unwrap();
        let book = read_waiting(&path).unwrap();
        assert_eq!(book.name, "b.xlsx");
        assert_eq!(book.sha256, sha256_hex(b"abc"));
    }
}
