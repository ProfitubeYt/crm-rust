//! HTTP access to the backend.
//!
//! Tokens live here and nowhere else. The embedded Telegram web view reaches
//! the server through [`Api::proxy`], so it never holds a credential.

mod admin;
mod auth;
mod excel;
mod logs;
mod reference;
mod security;
mod transactions;

pub use admin::*;
pub use auth::*;
pub use excel::*;
pub use logs::*;
pub use reference::*;
pub use security::*;
pub use transactions::*;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use base64::Engine;
use reqwest::{Method, StatusCode, header};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, broadcast};

use crate::secret::SecretStore;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Нет связи с сервером")]
    Network(String),
    #[error("{message}")]
    Http { status: u16, code: Option<String>, message: String },
    #[error("Сервер вернул неожиданный ответ")]
    Decode(String),
    #[error("Сессия завершена, войдите снова")]
    SessionExpired,
}

impl ApiError {
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn is_network(&self) -> bool {
        matches!(self, ApiError::Network(_))
    }
}

/// Things the rest of the app must react to, whichever call discovered them.
#[derive(Clone, Debug, PartialEq)]
pub enum ApiEvent {
    SessionRevoked,
    LaunchRequired,
    ScopeExpired,
    /// The server moved to a version this build was not made for.
    VersionMismatch,
}

/// The server version this build expects; unset builds do not compare.
const EXPECTED_API_VERSION: Option<&str> = option_env!("TBS_API_VERSION");

/// Each server version is reported once, however many responses carry it.
fn version_news(expected: Option<&str>, served: Option<&str>, reported: &mut Option<String>) -> bool {
    let (Some(expected), Some(served)) = (expected.filter(|v| !v.is_empty()), served.filter(|v| !v.is_empty())) else {
        return false;
    };
    if served == expected || reported.as_deref() == Some(served) {
        return false;
    }
    *reported = Some(served.to_owned());
    true
}

#[derive(Clone, Debug, Default)]
struct Tokens {
    auth: Option<String>,
    refresh: Option<String>,
    scope: Option<String>,
    launch: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Body {
    None,
    Json(Value),
    File { field: String, filename: String, bytes: Arc<Vec<u8>> },
    Raw { content_type: Option<String>, bytes: Arc<Vec<u8>> },
}

#[derive(Clone, Debug)]
pub struct Call {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Body,
    pub headers: Vec<(String, String)>,
}

impl Call {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Call { method, path: path.into(), query: Vec::new(), body: Body::None, headers: Vec::new() }
    }

    pub fn query(mut self, query: &[(&str, String)]) -> Self {
        self.query = query.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        self
    }

    pub fn json<B: Serialize>(mut self, body: &B) -> Self {
        self.body = Body::Json(serde_json::to_value(body).expect("request serialises"));
        self
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub bytes: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

struct Inner {
    http: reqwest::Client,
    base: RwLock<String>,
    system_token: RwLock<Option<String>>,
    tokens: RwLock<Tokens>,
    generation: AtomicU64,
    refresh_gate: Mutex<()>,
    vault: Arc<dyn SecretStore>,
    events: broadcast::Sender<ApiEvent>,
    reported_version: std::sync::Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct Api(Arc<Inner>);

/// Calls that must never trigger a refresh: they are how a session starts or ends.
fn refresh_exempt(path: &str) -> bool {
    ["/api/auth/login", "/api/auth/refresh", "/api/auth/logout"].iter().any(|p| path.starts_with(p))
}

/// A 401 that judges what was typed rather than the token: a wrong code or
/// password, or Telegram's own login. Taking one for a dead session would sign
/// the person out over a typo and spend a second attempt on the retry.
fn rejects_input(reply: &Reply) -> bool {
    let parsed: Value = serde_json::from_slice(&reply.bytes).unwrap_or(Value::Null);
    let detail = &parsed["detail"];
    matches!(detail["error"].as_str(), Some("invalid_code" | "invalid_password" | "invalid_current_password"))
        || detail.as_str().is_some_and(|text| text.starts_with("TDLib client"))
}

/// Pulls the human sentence out of any of the server's error shapes.
/// Errors whose numbers matter to the person reading them.
fn spelled_out(code: Option<&str>, detail: &Value) -> Option<String> {
    match code? {
        "invalid_code" => Some(match detail["attempts_left"].as_i64() {
            Some(left) => format!("Неверный код (осталось: {left})"),
            None => "Неверный код".to_string(),
        }),
        "locked" => Some(match detail["locked_seconds"].as_i64() {
            Some(seconds) => format!("Слишком много попыток. Повторите через {} мин", (seconds + 59) / 60),
            None => "Слишком много попыток. Повторите позже".to_string(),
        }),
        "export_limit_exceeded" => {
            let (total, limit) = (detail["total"].as_i64()?, detail["limit"].as_i64()?);
            Some(format!("Слишком много строк для экспорта ({total}). Лимит: {limit}. Уточните фильтры."))
        }
        _ => None,
    }
}

pub fn error_from_body(status: u16, bytes: &[u8]) -> ApiError {
    let parsed: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let detail = &parsed["detail"];
    let code = detail["error"]
        .as_str()
        .or(detail["code"].as_str())
        .or(parsed["error_code"].as_str())
        .map(str::to_string);
    let list_message = detail.as_array().and_then(|items| items.first()).and_then(|e| e["msg"].as_str());
    if let Some(message) = spelled_out(code.as_deref(), detail) {
        return ApiError::Http { status, code, message };
    }
    let message = detail
        .as_str()
        .or(detail["message"].as_str())
        .or(list_message)
        .or(detail["error"].as_str())
        .or(parsed["error"].as_str())
        .or(parsed["message"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| match status {
            401 => "Требуется вход".to_string(),
            403 => "Доступ запрещён".to_string(),
            404 => "Не найдено".to_string(),
            429 => "Слишком много попыток, подождите".to_string(),
            s if s >= 500 => format!("Ошибка сервера ({s})"),
            s => format!("Запрос отклонён ({s})"),
        });
    ApiError::Http { status, code, message }
}

/// Reads `exp` without verifying: this only decides whether to send a token,
/// the server still judges it.
pub fn jwt_expired(token: &str, now_unix: i64) -> bool {
    let Some(payload) = token.split('.').nth(1) else { return false };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')) else {
        return false;
    };
    let Ok(claims) = serde_json::from_slice::<Value>(&bytes) else { return false };
    claims["exp"].as_i64().is_some_and(|exp| exp <= now_unix)
}

/// The id of the session a token belongs to, so the session list can mark it.
pub fn jwt_session_id(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims = serde_json::from_slice::<Value>(&bytes).ok()?;
    ["sid", "session_id"].iter().find_map(|key| claims[*key].as_str().map(str::to_owned))
}

impl Api {
    pub fn new(base: String, system_token: Option<String>, vault: Arc<dyn SecretStore>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("TBSparcer-Desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client builds");
        let (events, _) = broadcast::channel(32);
        let api = Api(Arc::new(Inner {
            http,
            base: RwLock::new(base),
            system_token: RwLock::new(system_token),
            tokens: RwLock::new(Tokens::default()),
            generation: AtomicU64::new(0),
            refresh_gate: Mutex::new(()),
            vault,
            events,
            reported_version: std::sync::Mutex::new(None),
        }));
        let saved = api.0.vault.get(&api.vault_key());
        api.0.tokens.write().unwrap().refresh = saved;
        api
    }

    pub fn base(&self) -> String {
        self.0.base.read().unwrap().clone()
    }

    pub fn has_system_token(&self) -> bool {
        self.0.system_token.read().unwrap().is_some()
    }

    pub fn has_refresh_token(&self) -> bool {
        self.0.tokens.read().unwrap().refresh.is_some()
    }

    pub fn has_auth_token(&self) -> bool {
        self.0.tokens.read().unwrap().auth.is_some()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ApiEvent> {
        self.0.events.subscribe()
    }

    fn vault_key(&self) -> String {
        format!("refresh:{}", self.base())
    }

    fn emit(&self, event: ApiEvent) {
        let _ = self.0.events.send(event);
    }

    pub(crate) fn store_session(&self, auth: Option<String>, refresh: Option<String>, scope: Option<String>) {
        {
            let mut t = self.0.tokens.write().unwrap();
            if auth.is_some() {
                t.auth = auth;
            }
            if scope.is_some() {
                t.scope = scope;
            }
            if let Some(r) = refresh {
                self.0.vault.set(&self.vault_key(), &r);
                t.refresh = Some(r);
            }
        }
        self.0.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn set_scope_token(&self, token: Option<String>) {
        self.0.tokens.write().unwrap().scope = token;
    }

    pub fn set_launch_token(&self, token: Option<String>) {
        self.0.tokens.write().unwrap().launch = token;
    }

    pub fn clear_session(&self) {
        *self.0.tokens.write().unwrap() = Tokens::default();
        self.0.vault.delete(&self.vault_key());
        self.0.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Lets integration tests simulate an access token the server stopped accepting.
    #[doc(hidden)]
    pub fn break_auth_token_for_test(&self) {
        self.0.tokens.write().unwrap().auth = Some("expired".into());
    }

    #[doc(hidden)]
    pub fn break_refresh_token_for_test(&self) {
        self.0.tokens.write().unwrap().refresh = Some("x".repeat(32));
    }

    /// Which entry of the session list is this very client.
    pub fn current_session_id(&self) -> Option<String> {
        let token = self.0.tokens.read().unwrap().auth.clone()?;
        jwt_session_id(&token)
    }

    pub(crate) fn refresh_token(&self) -> Option<String> {
        self.0.tokens.read().unwrap().refresh.clone()
    }

    fn scope_token(&self) -> Option<String> {
        let token = self.0.tokens.read().unwrap().scope.clone()?;
        let now = chrono::Utc::now().timestamp();
        if jwt_expired(&token, now) {
            self.0.tokens.write().unwrap().scope = None;
            return None;
        }
        Some(token)
    }

    async fn dispatch(&self, call: &Call) -> Result<Reply, ApiError> {
        let url = format!("{}{}", self.base(), call.path);
        let mut req = self.0.http.request(call.method.clone(), url);
        if !call.query.is_empty() {
            req = req.query(&call.query);
        }
        if let Some(system) = self.0.system_token.read().unwrap().as_deref() {
            req = req.header("X-System-Access", system);
        }
        let (auth, launch) = {
            let t = self.0.tokens.read().unwrap();
            (t.auth.clone(), t.launch.clone())
        };
        if let Some(auth) = auth {
            req = req.bearer_auth(auth);
        }
        if let Some(scope) = self.scope_token() {
            req = req.header("X-Access-Token", scope);
        }
        if let Some(launch) = launch {
            req = req.header("X-Launch-Session", launch);
        }
        for (k, v) in &call.headers {
            req = req.header(k, v);
        }
        req = match &call.body {
            Body::None => req,
            Body::Json(v) => req.json(v),
            Body::File { field, filename, bytes } => {
                let part = reqwest::multipart::Part::bytes(bytes.as_ref().clone()).file_name(filename.clone());
                req.multipart(reqwest::multipart::Form::new().part(field.clone(), part))
            }
            Body::Raw { content_type, bytes } => {
                if let Some(ct) = content_type {
                    req = req.header(header::CONTENT_TYPE, ct);
                }
                req.body(bytes.as_ref().clone())
            }
        };
        let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;
        let served = resp.headers().get("x-api-version").and_then(|v| v.to_str().ok());
        if version_news(EXPECTED_API_VERSION, served, &mut self.0.reported_version.lock().unwrap()) {
            let _ = self.0.events.send(ApiEvent::VersionMismatch);
        }
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect();
        let bytes = resp.bytes().await.map_err(|e| ApiError::Network(e.to_string()))?.to_vec();
        Ok(Reply { status, headers, bytes })
    }

    /// One refresh at a time. A caller that arrives after another finished
    /// finds the generation moved and simply retries with the new token.
    async fn refresh_after(&self, seen_generation: u64) -> Result<bool, ApiError> {
        let _gate = self.0.refresh_gate.lock().await;
        if self.0.generation.load(Ordering::SeqCst) != seen_generation {
            return Ok(self.has_auth_token());
        }
        let Some(token) = self.refresh_token() else { return Ok(false) };
        let call = Call::new(Method::POST, "/api/auth/refresh")
            .json(&serde_json::json!({ "refresh_token": token }))
            .header("X-Refresh-Token", token);
        let reply = self.dispatch(&call).await?;
        if reply.status >= 500 {
            return Err(error_from_body(reply.status, &reply.bytes));
        }
        let parsed: LoginResponse = serde_json::from_slice(&reply.bytes).unwrap_or_default();
        if reply.status == 200 && parsed.ok && parsed.token.is_some() {
            self.store_session(parsed.token, parsed.refresh_token, parsed.scope_token);
            return Ok(true);
        }
        Ok(false)
    }

    fn session_lost(&self) {
        self.clear_session();
        self.emit(ApiEvent::SessionRevoked);
    }

    fn inspect_forbidden(&self, reply: &Reply) {
        let ApiError::Http { code, message, .. } = error_from_body(reply.status, &reply.bytes) else { return };
        let code = code.unwrap_or_default();
        if code == "launch_expired" || code == "launch_required" {
            {
                let mut t = self.0.tokens.write().unwrap();
                t.launch = None;
                t.scope = None;
            }
            self.emit(ApiEvent::LaunchRequired);
        } else if code.contains("scope") || message.to_lowercase().contains("scope") {
            self.0.tokens.write().unwrap().scope = None;
            self.emit(ApiEvent::ScopeExpired);
        }
    }

    /// Runs a call with the whole session protocol and returns the raw answer,
    /// error statuses included.
    pub async fn execute(&self, call: &Call) -> Result<Reply, ApiError> {
        let seen = self.0.generation.load(Ordering::SeqCst);
        let mut reply = self.dispatch(call).await?;
        if reply.status == StatusCode::UNAUTHORIZED.as_u16() && !refresh_exempt(&call.path) && !rejects_input(&reply) {
            if self.refresh_after(seen).await? {
                reply = self.dispatch(call).await?;
            }
            if reply.status == StatusCode::UNAUTHORIZED.as_u16() && !rejects_input(&reply) {
                self.session_lost();
            }
        }
        if reply.status == StatusCode::FORBIDDEN.as_u16() {
            self.inspect_forbidden(&reply);
        }
        Ok(reply)
    }

    pub async fn send<T: DeserializeOwned>(&self, call: Call) -> Result<T, ApiError> {
        let reply = self.checked(&call).await?;
        let bytes = if reply.bytes.is_empty() { b"null".as_slice() } else { reply.bytes.as_slice() };
        serde_json::from_slice(bytes).map_err(|e| ApiError::Decode(format!("{}: {e}", call.path)))
    }

    pub async fn checked(&self, call: &Call) -> Result<Reply, ApiError> {
        let reply = self.execute(call).await?;
        if (200..300).contains(&reply.status) {
            Ok(reply)
        } else {
            Err(error_from_body(reply.status, &reply.bytes))
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T, ApiError> {
        self.send(Call::new(Method::GET, path).query(query)).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T, ApiError> {
        self.send(Call::new(Method::POST, path).json(body)).await
    }

    pub async fn put<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T, ApiError> {
        self.send(Call::new(Method::PUT, path).json(body)).await
    }

    pub async fn patch<T: DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T, ApiError> {
        self.send(Call::new(Method::PATCH, path).json(body)).await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.send(Call::new(Method::DELETE, path)).await
    }

    pub async fn upload<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        filename: &str,
        bytes: Vec<u8>,
    ) -> Result<T, ApiError> {
        let mut call = Call::new(Method::POST, path).query(query);
        call.body = Body::File { field: "file".into(), filename: filename.into(), bytes: Arc::new(bytes) };
        self.send(call).await
    }

    /// Downloads a file and works out its server-given name.
    pub async fn download(&self, path: &str, query: &[(&str, String)]) -> Result<(Option<String>, Vec<u8>), ApiError> {
        let reply = self.checked(&Call::new(Method::GET, path).query(query)).await?;
        let name = reply.header("content-disposition").and_then(attachment_name);
        Ok((name, reply.bytes))
    }

    /// True if `/health` answers at all; used for the online indicator.
    pub async fn ping(&self) -> bool {
        let url = format!("{}/health", self.base());
        let req = self.0.http.get(url).timeout(Duration::from_secs(5));
        matches!(req.send().await, Ok(r) if r.status().is_success())
    }

    /// Forwards a request from the embedded web view with this session's
    /// credentials. The page's own auth headers are not honoured.
    pub async fn proxy(&self, method: Method, path_and_query: &str, headers: Vec<(String, String)>, body: Vec<u8>) -> Result<Reply, ApiError> {
        let (path, query) = path_and_query.split_once('?').unwrap_or((path_and_query, ""));
        let mut call = Call::new(method, path);
        call.query = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (percent_decode(k), percent_decode(v))
            })
            .collect();
        let mut content_type = None;
        for (k, v) in headers {
            let lower = k.to_ascii_lowercase();
            if lower == "content-type" {
                content_type = Some(v);
            } else if lower == "x-chat-access" {
                call.headers.push((k, v));
            }
        }
        if !body.is_empty() {
            call.body = Body::Raw { content_type, bytes: Arc::new(body) };
        }
        self.execute(&call).await
    }
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `filename*=UTF-8''...` wins over the plain form.
pub fn attachment_name(header: &str) -> Option<String> {
    for part in header.split(';').map(str::trim) {
        if let Some(rest) = part.strip_prefix("filename*=") {
            let encoded = rest.rsplit("''").next().unwrap_or(rest);
            return Some(percent_decode(&encoded.replace('+', "%2B")));
        }
    }
    header.split(';').map(str::trim).find_map(|p| p.strip_prefix("filename=")).map(|v| v.trim_matches('"').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::MemoryStore;

    #[test]
    fn a_new_server_version_is_reported_once() {
        let mut seen = None;
        assert!(!version_news(None, Some("2"), &mut seen));
        assert!(!version_news(Some("1"), None, &mut seen));
        assert!(!version_news(Some("1"), Some("1"), &mut seen));
        assert!(version_news(Some("1"), Some("2"), &mut seen));
        assert!(!version_news(Some("1"), Some("2"), &mut seen));
        assert!(version_news(Some("1"), Some("3"), &mut seen));
    }

    #[test]
    fn error_shapes() {
        let msg = |b: &str, s| error_from_body(s, b.as_bytes()).to_string();
        assert_eq!(msg(r#"{"detail":"Неверный логин"}"#, 401), "Неверный логин");
        assert_eq!(msg(r#"{"detail":{"error":"launch_expired","message":"Нужен запуск"}}"#, 403), "Нужен запуск");
        assert_eq!(msg(r#"{"ok":false,"error":"Плохо","error_code":"x"}"#, 400), "Плохо");
        assert_eq!(msg(r#"{"detail":[{"msg":"field required"}]}"#, 422), "field required");
        assert_eq!(msg("<html>", 502), "Ошибка сервера (502)");
        assert_eq!(msg("", 403), "Доступ запрещён");
        let ApiError::Http { code, .. } = error_from_body(403, br#"{"detail":{"error":"launch_expired"}}"#) else { panic!() };
        assert_eq!(code.as_deref(), Some("launch_expired"));
    }

    #[test]
    fn jwt_expiry() {
        let enc = |v: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        let token = format!("h.{}.s", enc(r#"{"exp":100}"#));
        assert!(jwt_expired(&token, 100));
        assert!(!jwt_expired(&token, 99));
        assert!(!jwt_expired("opaque", 100));
        assert!(!jwt_expired(&format!("h.{}.s", enc("{}")), 100));
    }

    #[test]
    fn attachment_names() {
        assert_eq!(attachment_name("attachment; filename=\"a b.xlsx\"").as_deref(), Some("a b.xlsx"));
        assert_eq!(
            attachment_name("attachment; filename=\"x.xlsx\"; filename*=UTF-8''%D0%A7%D0%B5%D0%BA%D0%B8.xlsx").as_deref(),
            Some("Чеки.xlsx")
        );
        assert_eq!(attachment_name("inline"), None);
    }

    #[test]
    fn percent_decoding_tolerates_bad_input() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn refresh_token_is_loaded_and_cleared_through_the_vault() {
        let vault = Arc::new(MemoryStore::default());
        vault.set("refresh:http://127.0.0.1:1", "r1");
        let api = Api::new("http://127.0.0.1:1".into(), None, vault.clone());
        assert!(api.has_refresh_token());
        api.clear_session();
        assert!(!api.has_refresh_token());
        assert_eq!(vault.get("refresh:http://127.0.0.1:1"), None);
    }

    #[test]
    fn session_endpoints_never_refresh() {
        assert!(refresh_exempt("/api/auth/login"));
        assert!(refresh_exempt("/api/auth/login/2fa"));
        assert!(refresh_exempt("/api/auth/logout"));
        assert!(!refresh_exempt("/api/auth/me"));
    }

    #[test]
    fn a_wrong_code_is_not_a_dead_session() {
        let reply = |body: &str| Reply { status: 401, headers: Vec::new(), bytes: body.as_bytes().to_vec() };
        assert!(rejects_input(&reply(r#"{"detail":{"error":"invalid_code","attempts_left":4}}"#)));
        assert!(rejects_input(&reply(r#"{"detail":{"verified":false,"error":"invalid_password"}}"#)));
        assert!(rejects_input(&reply(r#"{"detail":{"error":"invalid_current_password"}}"#)));
        assert!(rejects_input(&reply(r#"{"detail":"TDLib client is not authorized"}"#)));

        assert!(!rejects_input(&reply(r#"{"detail":"not_authenticated"}"#)));
        assert!(!rejects_input(&reply(r#"{"detail":"Invalid or expired token"}"#)));
        assert!(!rejects_input(&reply(r#"{"detail":"session_inactive"}"#)));
        assert!(!rejects_input(&reply("")));
    }

    #[test]
    fn a_lockout_says_how_long() {
        let error = error_from_body(429, br#"{"detail":{"error":"locked","locked_seconds":61}}"#);
        assert!(error.to_string().contains("через 2 мин"), "{error}");
    }
}
