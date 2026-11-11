//! What the embedded page may reach: its own files and the Telegram part of the
//! API, nothing else. The web view has no other way out to the server.

include!(concat!(env!("OUT_DIR"), "/telegram_assets.rs"));

/// WebView2 serves the custom `tbs` scheme from this origin.
pub const ORIGIN: &str = "http://tbs.localhost";
pub const ENTRY: &str = "telegram.html";

pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self' data:; connect-src 'self'; \
     frame-src 'none'; object-src 'none'; base-uri 'none'";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Asset { bytes: &'static [u8], mime: &'static str, document: bool },
    Proxy,
    Forbidden,
    Missing,
}

pub fn route(method: &str, path: &str) -> Route {
    route_in(ASSETS, method, path)
}

fn route_in(table: &'static [(&'static str, &'static [u8])], method: &str, path: &str) -> Route {
    let lower = path.to_ascii_lowercase();
    if ["..", "\\", "%2e", "%2f", "%5c"].iter().any(|bad| lower.contains(bad)) {
        return Route::Forbidden;
    }
    if path == "/api" || path.starts_with("/api/") {
        return if may_proxy(method, path) { Route::Proxy } else { Route::Forbidden };
    }
    if method != "GET" && method != "HEAD" {
        return Route::Forbidden;
    }
    let name = path.trim_start_matches('/');
    match table.iter().find(|(file, _)| *file == name) {
        Some((file, bytes)) => Route::Asset { bytes, mime: mime_of(file), document: file.ends_with(".html") },
        None => Route::Missing,
    }
}

fn may_proxy(method: &str, path: &str) -> bool {
    let telegram = path == "/api/tg" || path.starts_with("/api/tg/");
    // Raw file ids belong to the server's own pages; a chat reaches its files
    // through /api/tg/chats/{chat}/messages/{message}/files/{file}.
    (telegram && !path.starts_with("/api/tg/files/")) || (method == "POST" && path == "/api/transactions/process-receipt")
}

fn mime_of(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TABLE: &[(&str, &[u8])] = &[("telegram.html", b"<html>"), ("assets/app-1.js", b"js"), ("assets/app-1.css", b"css")];

    fn at(method: &str, path: &str) -> Route {
        route_in(TABLE, method, path)
    }

    #[test]
    fn own_files_are_served_with_their_type() {
        assert_eq!(at("GET", "/telegram.html"), Route::Asset { bytes: b"<html>", mime: "text/html; charset=utf-8", document: true });
        assert!(matches!(at("GET", "/assets/app-1.js"), Route::Asset { mime: "text/javascript; charset=utf-8", document: false, .. }));
        assert!(matches!(at("HEAD", "/assets/app-1.css"), Route::Asset { mime: "text/css; charset=utf-8", .. }));
        assert_eq!(at("GET", "/assets/none.js"), Route::Missing);
    }

    #[test]
    fn files_cannot_be_written_to() {
        assert_eq!(at("POST", "/telegram.html"), Route::Forbidden);
        assert_eq!(at("DELETE", "/assets/app-1.js"), Route::Forbidden);
    }

    #[test]
    fn only_the_telegram_api_is_forwarded() {
        assert_eq!(at("GET", "/api/tg/chats"), Route::Proxy);
        assert_eq!(at("POST", "/api/tg/chats/1/messages"), Route::Proxy);
        assert_eq!(at("GET", "/api/tg/chats/1/messages/2/files/3"), Route::Proxy);
        assert_eq!(at("POST", "/api/transactions/process-receipt"), Route::Proxy);
    }

    #[test]
    fn the_rest_of_the_api_stays_closed() {
        for (method, path) in [
            ("GET", "/api/transactions"),
            ("GET", "/api/transactions/process-receipt"),
            ("POST", "/api/auth/login"),
            ("GET", "/api/admin/users"),
            ("GET", "/api/tg/files/5"),
            ("GET", "/api/tgx"),
            ("GET", "/api"),
        ] {
            assert_eq!(at(method, path), Route::Forbidden, "{method} {path}");
        }
    }

    #[test]
    fn traversal_is_refused_in_every_spelling() {
        for path in ["/api/tg/../auth/login", "/api/tg/%2e%2e/auth", "/api/tg/x%2Fy", "/api/tg/x%5Cy", "/api/tg/a\\b", "/assets/../secret"] {
            assert_eq!(at("GET", path), Route::Forbidden, "{path}");
        }
    }
}
