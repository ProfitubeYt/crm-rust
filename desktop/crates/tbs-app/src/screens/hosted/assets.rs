//! What the page of a hosted workspace may reach: its own files and its own
//! part of the API, nothing else. Each page has an origin of its own, so it
//! shares neither storage nor an allowlist with another page or with Telegram.

include!(concat!(env!("OUT_DIR"), "/hosted_assets.rs"));

/// The workspaces that are web pages, both compiled in: Time's own interface
/// and the CRM administration panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Time,
    Admin,
}

impl Kind {
    /// The custom scheme the page is served from.
    pub fn scheme(self) -> &'static str {
        match self {
            Kind::Time => "tbstime",
            Kind::Admin => "tbsadmin",
        }
    }

    /// WebView2 serves a custom scheme from `http://<scheme>.localhost`; the
    /// other engines keep the scheme itself.
    pub fn origin(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Kind::Time, true) => "http://tbstime.localhost",
            (Kind::Time, false) => "tbstime://localhost",
            (Kind::Admin, true) => "http://tbsadmin.localhost",
            (Kind::Admin, false) => "tbsadmin://localhost",
        }
    }

    /// The document, named as the page's own links expect to find its files.
    pub fn entry(self) -> &'static str {
        match self {
            Kind::Time => "index.html",
            Kind::Admin => "admin/index.html",
        }
    }

    /// The one part of the API the page's requests are forwarded to. The
    /// server judges every one of them again by the account.
    pub fn api(self) -> &'static str {
        match self {
            Kind::Time => "/api/time",
            Kind::Admin => "/api/admin",
        }
    }

    pub fn files(self) -> &'static [(&'static str, &'static [u8])] {
        match self {
            Kind::Time => TIME_ASSETS,
            Kind::Admin => ADMIN_ASSETS,
        }
    }

    /// Nothing from anywhere else, no frames, no form posts. Time builds its
    /// tables with inline styles; the panel's own policy has none, the same
    /// as the server sends with it (`backend/api/admin_panel.py`).
    pub fn policy(self) -> &'static str {
        match self {
            Kind::Time => {
                "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                 font-src 'self'; connect-src 'self'; form-action 'none'; base-uri 'none'; frame-src 'none'; object-src 'none'"
            }
            Kind::Admin => "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; form-action 'none'; base-uri 'none'",
        }
    }

    /// Runs before the page's own scripts and tells it where it is: inside
    /// the program, on the program's session. The page then asks for no
    /// password and sends no credential; it has none.
    pub fn greeting(self, dark: bool) -> String {
        let theme = theme_name(dark);
        match self {
            Kind::Time => format!("window.TIMELIST_HUB = Object.freeze({{ apiBase: '/api/time', native: true, theme: '{theme}' }});"),
            Kind::Admin => format!("window.CRM_ADMIN_SHELL = Object.freeze({{ native: true, theme: '{theme}' }});"),
        }
    }

    pub fn theme_script(self, dark: bool) -> String {
        let theme = theme_name(dark);
        match self {
            Kind::Time => format!("window.timelistSetTheme&&window.timelistSetTheme('{theme}')"),
            Kind::Admin => format!("window.crmAdminShell&&window.crmAdminShell.theme('{theme}')"),
        }
    }

    /// A toast of the program, shown by the page that covers the program's own.
    pub fn toast_script(self, message: &str, failed: bool) -> String {
        let text = serde_json::to_string(message).unwrap_or_else(|_| "\"\"".to_owned());
        match self {
            Kind::Time => format!("window.timelistToast&&window.timelistToast({text},{failed})"),
            Kind::Admin => format!("window.crmAdminShell&&window.crmAdminShell.toast({text},{failed})"),
        }
    }
}

fn theme_name(dark: bool) -> &'static str {
    if dark { "dark" } else { "light" }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Asset { bytes: &'static [u8], mime: &'static str, document: bool },
    Proxy,
    Forbidden,
    Missing,
}

pub fn route(kind: Kind, method: &str, path: &str) -> Route {
    route_in(kind.files(), kind.api(), method, path)
}

fn route_in(files: &'static [(&'static str, &'static [u8])], api: &str, method: &str, path: &str) -> Route {
    let lower = path.to_ascii_lowercase();
    if ["..", "\\", "%2e", "%2f", "%5c", "%00"].iter().any(|bad| lower.contains(bad)) {
        return Route::Forbidden;
    }
    if path == "/api" || path.starts_with("/api/") {
        let own = path.strip_prefix(api).is_some_and(|rest| rest.starts_with('/'));
        let known = matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE");
        return if own && known { Route::Proxy } else { Route::Forbidden };
    }
    if method != "GET" && method != "HEAD" {
        return Route::Forbidden;
    }
    let name = path.trim_start_matches('/');
    match files.iter().find(|(file, _)| *file == name) {
        Some((file, bytes)) => Route::Asset { bytes, mime: mime_of(file), document: file.ends_with(".html") },
        None => Route::Missing,
    }
}

fn mime_of(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_pages_are_compiled_in_under_the_names_their_links_use() {
        let names = |kind: Kind| kind.files().iter().map(|(name, _)| *name).collect::<Vec<_>>();
        assert_eq!(names(Kind::Time), ["index.html", "assets/app.css", "assets/app.js"]);
        assert_eq!(names(Kind::Admin), ["admin/index.html", "admin/admin.css", "admin/admin.js"]);
        for kind in [Kind::Time, Kind::Admin] {
            assert!(matches!(route(kind, "GET", &format!("/{}", kind.entry())), Route::Asset { document: true, mime: "text/html; charset=utf-8", .. }));
            let page = kind.files().iter().find(|(name, _)| *name == kind.entry()).map(|(_, bytes)| String::from_utf8_lossy(bytes)).unwrap();
            for (name, _) in kind.files().iter().filter(|(name, _)| !name.ends_with(".html")) {
                assert!(page.contains(&format!("\"/{name}\"")), "{name} is not what {} links to", kind.entry());
            }
        }
        // The pages know the program's greeting: without it they would ask for a password of their own.
        let script = |kind: Kind, name: &str| kind.files().iter().find(|(file, _)| *file == name).map(|(_, bytes)| String::from_utf8_lossy(bytes)).unwrap();
        assert!(script(Kind::Time, "assets/app.js").contains("window.TIMELIST_HUB") && script(Kind::Time, "assets/app.js").contains("window.timelistToast"));
        assert!(script(Kind::Admin, "admin/admin.js").contains("window.CRM_ADMIN_SHELL") && script(Kind::Admin, "admin/admin.js").contains("window.crmAdminShell"));
    }

    #[test]
    fn each_page_reaches_its_own_part_of_the_api_and_no_other() {
        for (method, path) in [("GET", "/api/time/meta"), ("PUT", "/api/time/marks/3/2026-11-02"), ("POST", "/api/time/import/xlsx"), ("DELETE", "/api/time/periods/4"), ("GET", "/api/time/codes/%D0%9A1")] {
            assert_eq!(route(Kind::Time, method, path), Route::Proxy, "{method} {path}");
            assert_eq!(route(Kind::Admin, method, path), Route::Forbidden, "{method} {path}");
        }
        for (method, path) in [("GET", "/api/admin/meta"), ("POST", "/api/admin/users"), ("PATCH", "/api/admin/users/7"), ("DELETE", "/api/admin/users/7")] {
            assert_eq!(route(Kind::Admin, method, path), Route::Proxy, "{method} {path}");
            assert_eq!(route(Kind::Time, method, path), Route::Forbidden, "{method} {path}");
        }
    }

    #[test]
    fn the_session_and_every_other_workspace_stay_out_of_a_pages_reach() {
        let closed = [
            ("POST", "/api/auth/login"),
            ("POST", "/api/auth/refresh"),
            ("POST", "/api/auth/logout"),
            ("GET", "/api/auth/me"),
            ("GET", "/api/transactions"),
            ("GET", "/api/transactions/export"),
            ("GET", "/api/tg/chats"),
            ("GET", "/api/security/sessions"),
            ("PATCH", "/api/system-settings"),
            ("POST", "/api/security/app/verify-launch"),
            ("GET", "/api/timex"),
            ("GET", "/api/time"),
            ("GET", "/api/administrator"),
            ("GET", "/api/admin"),
            ("GET", "/api"),
            ("GET", "/internal/time/telegram/1"),
        ];
        for kind in [Kind::Time, Kind::Admin] {
            for (method, path) in closed {
                assert!(matches!(route(kind, method, path), Route::Forbidden | Route::Missing), "{kind:?} {method} {path}");
                assert_ne!(route(kind, method, path), Route::Proxy, "{kind:?} {method} {path}");
            }
            // Not a method a page has any business sending.
            assert_eq!(route(kind, "CONNECT", &format!("{}/x", kind.api())), Route::Forbidden);
            assert_eq!(route(kind, "TRACE", &format!("{}/x", kind.api())), Route::Forbidden);
        }
    }

    #[test]
    fn traversal_is_refused_in_every_spelling() {
        for kind in [Kind::Time, Kind::Admin] {
            let api = kind.api();
            for tail in ["/../auth/login", "/%2e%2e/auth/login", "/x%2Fy", "/x%5Cy", "/a\\b", "/x%00"] {
                assert_eq!(route(kind, "GET", &format!("{api}{tail}")), Route::Forbidden, "{api}{tail}");
            }
            assert_eq!(route(kind, "GET", "/assets/../secret"), Route::Forbidden);
        }
    }

    #[test]
    fn a_pages_files_are_read_only_and_one_page_does_not_serve_anothers() {
        assert_eq!(route(Kind::Time, "POST", "/index.html"), Route::Forbidden);
        assert_eq!(route(Kind::Admin, "DELETE", "/admin/admin.js"), Route::Forbidden);
        assert_eq!(route(Kind::Time, "GET", "/admin/admin.js"), Route::Missing);
        assert_eq!(route(Kind::Admin, "GET", "/assets/app.js"), Route::Missing);
        assert_eq!(route(Kind::Admin, "GET", "/telegram.html"), Route::Missing);
        assert!(matches!(route(Kind::Time, "HEAD", "/assets/app.css"), Route::Asset { mime: "text/css; charset=utf-8", document: false, .. }));
    }

    #[test]
    fn the_pages_keep_apart_and_load_nothing_from_outside() {
        assert_ne!(Kind::Time.origin(), Kind::Admin.origin());
        assert_ne!(Kind::Time.scheme(), Kind::Admin.scheme());
        for kind in [Kind::Time, Kind::Admin] {
            // The Telegram page is served from `tbs`; neither of these may share its origin.
            assert_ne!(kind.scheme(), "tbs");
            let policy = kind.policy();
            assert!(policy.starts_with("default-src 'none'"), "{policy}");
            for directive in ["script-src 'self'", "connect-src 'self'", "form-action 'none'", "base-uri 'none'"] {
                assert!(policy.contains(directive), "{kind:?}: {directive}");
            }
            assert!(!policy.contains("http") && !policy.contains('*') && !policy.contains("unsafe-eval"), "{policy}");
            assert!(!policy.contains("script-src 'self' 'unsafe-inline'"), "{policy}");
        }
        // The panel's policy is the one the server sends with it, less what only concerns framing.
        assert!(!Kind::Admin.policy().contains("unsafe-inline"));
    }

    #[test]
    fn what_the_program_tells_a_page_cannot_be_broken_out_of() {
        let script = Kind::Time.toast_script("x\");alert(1);//\n</script>", true);
        assert_eq!(script, r#"window.timelistToast&&window.timelistToast("x\");alert(1);//\n</script>",true)"#);
        assert!(Kind::Admin.toast_script("ok", false).ends_with(r#"("ok",false)"#));
        assert_eq!(Kind::Time.greeting(true), "window.TIMELIST_HUB = Object.freeze({ apiBase: '/api/time', native: true, theme: 'dark' });");
        assert!(Kind::Admin.greeting(false).contains("theme: 'light'"));
        assert_eq!(Kind::Admin.theme_script(true), "window.crmAdminShell&&window.crmAdminShell.theme('dark')");
    }
}
