//! Where embedded web content may go. Mini apps run third-party pages, so what
//! they can load and what they can hand to the system is decided here.

use reqwest::Url;

const EXTERNAL_HTTPS_HOSTS: [&str; 2] = ["t.me", "telegram.me"];
const EXTERNAL_TG_HOSTS: [&str; 3] = ["join", "resolve", "share"];

fn has_credentials(url: &Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

/// A mini app is only ever loaded over HTTPS, without credentials in the URL.
pub fn is_safe_mini_app_url(raw: &str) -> bool {
    Url::parse(raw).is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some() && !has_credentials(&url))
}

/// The origin of a URL, comparable between navigations.
pub fn origin_of(raw: &str) -> Option<String> {
    let origin = Url::parse(raw).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// Whether a mini app may navigate to `target`. Launch URLs usually redirect, so
/// the origin is pinned only after the first page has finished loading.
pub fn mini_app_may_navigate(target: &str, pinned_origin: Option<&str>) -> bool {
    if !is_safe_mini_app_url(target) {
        return false;
    }
    pinned_origin.is_none_or(|pinned| origin_of(target).as_deref() == Some(pinned))
}

/// A link the user may be offered to open outside the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalLink {
    pub url: String,
    pub host: String,
}

/// Only Telegram's own links leave the app; anything else is dropped.
pub fn classify_external(raw: &str) -> Option<ExternalLink> {
    let url = Url::parse(raw.trim()).ok()?;
    if has_credentials(&url) {
        return None;
    }
    let host = url.host_str()?.to_owned();
    let allowed = match url.scheme() {
        "https" => EXTERNAL_HTTPS_HOSTS.contains(&host.as_str()) && url.port().is_none(),
        "tg" => EXTERNAL_TG_HOSTS.contains(&host.as_str()),
        _ => false,
    };
    allowed.then(|| ExternalLink { url: url.to_string(), host })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mini_apps_need_https_without_credentials() {
        assert!(is_safe_mini_app_url("https://app.example.com/start?x=1"));
        assert!(!is_safe_mini_app_url("http://app.example.com/"));
        assert!(!is_safe_mini_app_url("https://user:pw@app.example.com/"));
        assert!(!is_safe_mini_app_url("https://user@app.example.com/"));
        assert!(!is_safe_mini_app_url("file:///C:/secrets.txt"));
        assert!(!is_safe_mini_app_url("javascript:alert(1)"));
        assert!(!is_safe_mini_app_url("not a url"));
    }

    #[test]
    fn a_launch_may_redirect_but_a_loaded_app_stays_on_its_origin() {
        assert!(mini_app_may_navigate("https://other.example.com/", None));
        assert!(!mini_app_may_navigate("http://other.example.com/", None));
        let pinned = Some("https://app.example.com");
        assert!(mini_app_may_navigate("https://app.example.com/next", pinned));
        assert!(!mini_app_may_navigate("https://evil.example.com/", pinned));
        assert!(!mini_app_may_navigate("https://app.example.com:8443/", pinned));
    }

    #[test]
    fn only_telegram_links_leave_the_app() {
        assert_eq!(classify_external("https://t.me/channel").map(|l| l.host).as_deref(), Some("t.me"));
        assert!(classify_external("https://telegram.me/x").is_some());
        assert!(classify_external("https://t.me:443/x").is_some());
        assert!(classify_external("tg://resolve?domain=x").is_some());
        assert!(classify_external("tg://join?invite=x").is_some());
        assert!(classify_external("tg://share?url=x").is_some());
    }

    #[test]
    fn everything_else_is_dropped() {
        for raw in [
            "https://evil.example.com/",
            "https://t.me.evil.example.com/",
            "https://t.me:8443/x",
            "http://t.me/x",
            "https://user:pw@t.me/x",
            "tg://settings",
            "tg:resolve",
            "file:///C:/Windows/notepad.exe",
            "ms-msdt:/id",
            "blob:http://tbs.localhost/1",
            "",
        ] {
            assert_eq!(classify_external(raw), None, "{raw}");
        }
    }

    #[test]
    fn a_link_handed_to_the_system_is_the_normalized_one() {
        assert_eq!(classify_external("  https://t.me/a b ").unwrap().url, "https://t.me/a%20b");
    }
}
