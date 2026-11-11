//! Where the server is and how this machine proves it may talk to it.
//!
//! The lookup order matches the Electron client so an installed
//! `client-access.json` keeps working after the switch.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_API_BASE_URL: &str = "http://127.0.0.1:8000";
pub const ACCESS_FILE: &str = "client-access.json";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientAccess {
    pub system_token: Option<String>,
    pub api_base_url: Option<String>,
    pub source: Option<PathBuf>,
}

#[derive(Deserialize)]
struct AccessFile {
    #[serde(alias = "systemToken", alias = "system_token", alias = "token")]
    system_access_token: Option<String>,
    #[serde(alias = "apiBaseUrl")]
    api_base_url: Option<String>,
}

/// Parses one candidate. A file with no token is treated as absent so a later
/// candidate can still supply one.
pub fn read_access_file(path: &Path) -> Option<ClientAccess> {
    let raw = fs::read_to_string(path).ok()?;
    let parsed: AccessFile = serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    let token = parsed.system_access_token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    token.as_ref()?;
    Some(ClientAccess {
        system_token: token,
        api_base_url: parsed.api_base_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        source: Some(path.to_path_buf()),
    })
}

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = env::var("CLIENT_ACCESS_CONFIG_PATH") {
        if !p.trim().is_empty() {
            out.push(PathBuf::from(p));
        }
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("resources").join("security").join(ACCESS_FILE));
            out.push(dir.join("security").join(ACCESS_FILE));
        }
    }
    if let Ok(pd) = env::var("PROGRAMDATA") {
        out.push(PathBuf::from(pd).join("TBSparcer").join("security").join(ACCESS_FILE));
    }
    // Running from a checkout: walk up to the repository's security/ folder.
    if let Ok(cwd) = env::current_dir() {
        for dir in cwd.ancestors().take(6) {
            out.push(dir.join("security").join(ACCESS_FILE));
        }
    }
    out.push(data_dir().join("security").join(ACCESS_FILE));
    out
}

pub fn discover_access() -> ClientAccess {
    let mut access = candidates().iter().find_map(|p| read_access_file(p)).unwrap_or_default();
    if let Ok(url) = env::var("TBS_API_BASE_URL") {
        if !url.trim().is_empty() {
            access.api_base_url = Some(url.trim().to_string());
        }
    }
    access
}

/// Accepts https anywhere and plain http only on loopback. Tokens must not
/// travel in the clear, so this is a refusal rather than a warning.
pub fn canonical_base_url(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| format!("Некорректный адрес сервера: {raw}"))?;
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1");
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err("Адрес сервера должен использовать https".into()),
    }
    let mut out = url.clone();
    if host == "localhost" {
        let _ = out.set_host(Some("127.0.0.1"));
    }
    Ok(out.as_str().trim_end_matches('/').to_string())
}

/// `%LOCALAPPDATA%\TBSparcer`, or `TBS_DATA_DIR` for tests and portable runs.
pub fn data_dir() -> PathBuf {
    if let Ok(p) = env::var("TBS_DATA_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    directories::BaseDirs::new()
        .map(|d| d.data_local_dir().join("TBSparcer"))
        .unwrap_or_else(|| PathBuf::from(".tbsparcer"))
}

/// Per-user interface state: theme, density, column layout.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub theme: Option<String>,
    /// The single login older clients kept. Read so nobody loses it, folded
    /// into `known_users` on the first change, and never written back.
    #[serde(skip_serializing)]
    pub remember_username: Option<String>,
    /// Who signed in on this PC, latest first. The sign-in window offers these;
    /// the server does not name its users to someone who has not signed in.
    pub known_users: Vec<String>,
    pub table: serde_json::Value,
    /// The navigation rail is folded down to its icons.
    pub rail_collapsed: bool,
}

const KNOWN_USERS_MAX: usize = 8;

impl Preferences {
    pub fn users(&self) -> Vec<String> {
        let mut users = self.known_users.clone();
        if let Some(older) = &self.remember_username
            && !users.contains(older)
        {
            users.push(older.clone());
        }
        users
    }

    pub fn remember_user(&mut self, name: &str) {
        self.known_users = self.users();
        self.remember_username = None;
        self.known_users.retain(|known| known != name);
        self.known_users.insert(0, name.to_owned());
        self.known_users.truncate(KNOWN_USERS_MAX);
    }

    pub fn forget_user(&mut self, name: &str) {
        self.known_users = self.users();
        self.remember_username = None;
        self.known_users.retain(|known| known != name);
    }
}

pub fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Write-then-rename so a crash mid-save cannot leave half a settings file.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_rules() {
        assert_eq!(canonical_base_url("http://localhost:8000/").unwrap(), "http://127.0.0.1:8000");
        assert_eq!(canonical_base_url("https://a.example/").unwrap(), "https://a.example");
        assert!(canonical_base_url("http://64.188.106.221").is_err());
        assert!(canonical_base_url("ftp://localhost").is_err());
        assert!(canonical_base_url("не адрес").is_err());
    }

    #[test]
    fn access_file_variants() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(ACCESS_FILE);
        fs::write(&p, "\u{feff}{\"systemToken\":\" abc \",\"api_base_url\":\"https://x.example\"}").unwrap();
        let a = read_access_file(&p).unwrap();
        assert_eq!(a.system_token.as_deref(), Some("abc"));
        assert_eq!(a.api_base_url.as_deref(), Some("https://x.example"));
        fs::write(&p, "{\"system_access_token\":\"  \"}").unwrap();
        assert!(read_access_file(&p).is_none());
    }

    #[test]
    fn json_roundtrip_is_atomic_and_tolerant() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a").join("prefs.json");
        let prefs = Preferences { theme: Some("dark".into()), ..Default::default() };
        save_json(&p, &prefs).unwrap();
        assert_eq!(load_json::<Preferences>(&p), prefs);
        fs::write(&p, "{broken").unwrap();
        assert_eq!(load_json::<Preferences>(&p), Preferences::default());
    }

    #[test]
    fn the_login_an_older_client_saved_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("prefs.json");
        fs::write(&p, "{\"remember_username\":\"anna\"}").unwrap();
        let mut prefs = load_json::<Preferences>(&p);
        assert_eq!(prefs.users(), ["anna"]);

        prefs.remember_user("boris");
        assert_eq!(prefs.users(), ["boris", "anna"]);
        save_json(&p, &prefs).unwrap();
        assert!(!fs::read_to_string(&p).unwrap().contains("remember_username"));
        assert_eq!(load_json::<Preferences>(&p).users(), ["boris", "anna"]);
    }

    #[test]
    fn the_latest_user_comes_first_and_the_list_stays_short() {
        let mut prefs = Preferences::default();
        for name in ["a", "b", "a"] {
            prefs.remember_user(name);
        }
        assert_eq!(prefs.users(), ["a", "b"]);
        for n in 0..20 {
            prefs.remember_user(&format!("user{n}"));
        }
        assert_eq!(prefs.users().len(), KNOWN_USERS_MAX);
        assert_eq!(prefs.users()[0], "user19");
    }

    #[test]
    fn a_forgotten_user_does_not_come_back_from_the_old_field() {
        let mut prefs = Preferences { remember_username: Some("anna".into()), ..Default::default() };
        prefs.forget_user("anna");
        assert!(prefs.users().is_empty());
        prefs.forget_user("nobody");
        assert!(prefs.users().is_empty());
    }
}
