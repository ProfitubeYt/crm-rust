//! HTTP access to the backend.
//!
//! Tokens live here and nowhere else. The workspaces that are web pages reach
//! the server through [`Api::proxy`], so they never hold a credential.

mod admin;
mod auth;
mod excel;
mod logs;
mod reference;
mod security;
mod telegram;
mod time;
mod transactions;

pub use admin::*;
pub use auth::*;
pub use excel::*;
pub use logs::*;
pub use reference::*;
pub use security::*;
pub use telegram::*;
pub use time::*;
pub use transactions::*;

use std::collections::HashMap;
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
    /// The account no longer has the workspace a request was made in. The
    /// server names it; the session itself stands.
    WorkspaceRevoked(String),
    /// CRM administration was asked for by an account that is not, or is no
    /// longer, a CRM administrator.
    AdminRevoked,
    /// An administrator switched the account off.
    AccountDisabled,
    /// The account's term ran out while the client was at work.
    AccessExpired,
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
    /// One file of a form. Without `mime` the part goes untyped.
    File { field: String, filename: String, mime: Option<String>, bytes: Arc<Vec<u8>> },
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
    /// Passes to password-protected Telegram chats, by chat.
    chat_access: RwLock<HashMap<i64, String>>,
    generation: AtomicU64,
    /// Which sign-in the tokens belong to. Unlike `generation` it stays put
    /// while the same session only renews its tokens.
    epoch: AtomicU64,
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
        "workspace_forbidden" => Some("Доступ к этому рабочему пространству закрыт администратором".to_string()),
        "access_expired" => Some("Срок доступа учётной записи истёк".to_string()),
        "time_role_forbidden" => Some("Недостаточно прав: в «Учёте времени» открыт только просмотр".to_string()),
        "time_workspace_unavailable" => Some("Учёт времени на сервере не настроен".to_string()),
        "time_upstream_unavailable" | "time_upstream_refused" => Some("Сервис учёта времени не отвечает, попробуйте позже".to_string()),
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
            chat_access: RwLock::default(),
            generation: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
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

    /// Signing in or out moves it. What was asked before that speaks for a
    /// session that is no longer the one there is: its refusals, its renewed
    /// tokens and its sign-in answer are all let go.
    pub(crate) fn epoch(&self) -> u64 {
        self.0.epoch.load(Ordering::SeqCst)
    }

    /// The tokens of a sign-in that set out at `epoch`. `false`: another
    /// sign-in or a sign-out got there first, and nothing was kept.
    pub(crate) fn begin_session(&self, epoch: u64, auth: Option<String>, refresh: Option<String>, scope: Option<String>) -> bool {
        self.keep_tokens(epoch, true, auth, refresh, scope)
    }

    fn keep_tokens(&self, epoch: u64, begins: bool, auth: Option<String>, refresh: Option<String>, scope: Option<String>) -> bool {
        {
            let mut t = self.0.tokens.write().unwrap();
            if self.0.epoch.load(Ordering::SeqCst) != epoch {
                return false;
            }
            if begins {
                self.0.epoch.fetch_add(1, Ordering::SeqCst);
            }
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
        true
    }

    pub fn set_scope_token(&self, token: Option<String>) {
        self.0.tokens.write().unwrap().scope = token;
    }

    pub fn set_launch_token(&self, token: Option<String>) {
        self.0.tokens.write().unwrap().launch = token;
    }

    pub fn clear_session(&self) {
        {
            let mut t = self.0.tokens.write().unwrap();
            *t = Tokens::default();
            self.0.epoch.fetch_add(1, Ordering::SeqCst);
        }
        self.0.chat_access.write().unwrap().clear();
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

    pub(crate) fn chat_access(&self, chat: i64) -> Option<String> {
        self.0.chat_access.read().unwrap().get(&chat).cloned()
    }

    pub(crate) fn set_chat_access(&self, chat: i64, pass: Option<String>) {
        let mut passes = self.0.chat_access.write().unwrap();
        match pass {
            Some(pass) => passes.insert(chat, pass),
            None => passes.remove(&chat),
        };
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
            Body::File { field, filename, mime, bytes } => {
                let mut part = reqwest::multipart::Part::bytes(bytes.as_ref().clone()).file_name(filename.clone());
                if let Some(mime) = mime {
                    part = part.mime_str(mime).map_err(|e| ApiError::Network(e.to_string()))?;
                }
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
    /// finds the generation moved and simply retries with the new token. A
    /// caller whose session has ended or been replaced does neither: what it
    /// asked is not asked again on somebody else's session.
    async fn refresh_after(&self, seen_generation: u64, epoch: u64) -> Result<bool, ApiError> {
        let _gate = self.0.refresh_gate.lock().await;
        if self.epoch() != epoch {
            return Ok(false);
        }
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
            // Signed out while the server was answering: nobody holds the
            // session these tokens renew.
            return Ok(self.keep_tokens(epoch, false, parsed.token, parsed.refresh_token, parsed.scope_token));
        }
        Ok(false)
    }

    fn session_lost(&self) {
        self.clear_session();
        self.emit(ApiEvent::SessionRevoked);
    }

    /// A 403 keeps the session and says what was refused: a code in
    /// `detail.error`, or the whole of a `detail` that is one word.
    fn inspect_forbidden(&self, path: &str, reply: &Reply) {
        let ApiError::Http { code, message, .. } = error_from_body(reply.status, &reply.bytes) else { return };
        let code = code.unwrap_or_default();
        let parsed: Value = serde_json::from_slice(&reply.bytes).unwrap_or(Value::Null);
        let refusal = if code.is_empty() { parsed["detail"].as_str().unwrap_or_default() } else { code.as_str() };
        // The sign-in form reads its own refusals: nobody is at work yet.
        let at_work = !refresh_exempt(path);
        match refusal {
            "launch_expired" | "launch_required" => {
                {
                    let mut t = self.0.tokens.write().unwrap();
                    t.launch = None;
                    t.scope = None;
                }
                self.emit(ApiEvent::LaunchRequired);
            }
            "workspace_forbidden" if at_work => {
                let workspace = parsed["detail"]["workspace"].as_str().unwrap_or_default().to_owned();
                self.emit(ApiEvent::WorkspaceRevoked(workspace));
            }
            "global_admin_required" if at_work => self.emit(ApiEvent::AdminRevoked),
            "inactive_user" if at_work => {
                // The server ended every session of the account with it.
                self.clear_session();
                self.emit(ApiEvent::AccountDisabled);
            }
            "access_expired" if at_work => self.emit(ApiEvent::AccessExpired),
            _ if code.contains("scope") || message.to_lowercase().contains("scope") => {
                self.0.tokens.write().unwrap().scope = None;
                self.emit(ApiEvent::ScopeExpired);
            }
            _ => {}
        }
    }

    /// Runs a call with the whole session protocol and returns the raw answer,
    /// error statuses included.
    pub async fn execute(&self, call: &Call) -> Result<Reply, ApiError> {
        let (seen, epoch) = (self.0.generation.load(Ordering::SeqCst), self.epoch());
        let mut reply = self.dispatch(call).await?;
        if reply.status == StatusCode::UNAUTHORIZED.as_u16() && !refresh_exempt(&call.path) && !rejects_input(&reply) {
            if self.refresh_after(seen, epoch).await? {
                reply = self.dispatch(call).await?;
            }
            if reply.status == StatusCode::UNAUTHORIZED.as_u16() && !rejects_input(&reply) && self.epoch() == epoch {
                self.session_lost();
            }
        }
        // A refusal that arrives after the account signed out, or another
        // signed in, was meant for the session that asked.
        if reply.status == StatusCode::FORBIDDEN.as_u16() && self.epoch() == epoch {
            self.inspect_forbidden(&call.path, &reply);
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
        call.body = Body::File { field: "file".into(), filename: filename.into(), mime: None, bytes: Arc::new(bytes) };
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

    /// What a 403 with this body tells the app, asked of a signed-in client.
    fn refused(path: &str, body: &str) -> (Vec<ApiEvent>, bool) {
        let vault = Arc::new(MemoryStore::default());
        vault.set("refresh:http://127.0.0.1:1", "saved-session");
        let api = Api::new("http://127.0.0.1:1".into(), None, vault);
        let mut events = api.subscribe();
        api.inspect_forbidden(path, &Reply { status: 403, headers: Vec::new(), bytes: body.as_bytes().to_vec() });
        let mut told = Vec::new();
        while let Ok(event) = events.try_recv() {
            told.push(event);
        }
        (told, api.has_refresh_token())
    }

    #[test]
    fn a_revoked_workspace_is_named_and_the_session_stands() {
        let (told, session) = refused("/api/time/meta", r#"{"detail":{"error":"workspace_forbidden","workspace":"time"}}"#);
        assert_eq!(told, [ApiEvent::WorkspaceRevoked("time".into())]);
        assert!(session, "the account keeps the workspaces it still has");
        let (told, _) = refused("/api/transactions", r#"{"detail":{"error":"workspace_forbidden","workspace":"checks"}}"#);
        assert_eq!(told, [ApiEvent::WorkspaceRevoked("checks".into())]);
        // The wording must never read as an expired period: that is another event.
        assert!(!told.contains(&ApiEvent::ScopeExpired));
    }

    #[test]
    fn administration_refused_is_not_a_lost_session() {
        let (told, session) = refused("/api/admin/users", r#"{"detail":"global_admin_required"}"#);
        assert_eq!(told, [ApiEvent::AdminRevoked]);
        assert!(session);
        // The Checks administrator's own refusal is the screen's to show.
        assert_eq!(refused("/api/periods", r#"{"detail":"admin_only"}"#).0, []);
    }

    #[test]
    fn a_disabled_account_loses_its_saved_session() {
        let (told, session) = refused("/api/auth/me", r#"{"detail":"inactive_user"}"#);
        assert_eq!(told, [ApiEvent::AccountDisabled]);
        assert!(!session, "a session of a disabled account is not kept for the next launch");
    }

    #[test]
    fn an_expired_term_is_told_apart_from_an_expired_period() {
        let (told, session) = refused("/api/time/meta", r#"{"detail":{"error":"access_expired","expired_at":"2026-11-01T00:00:00"}}"#);
        assert_eq!(told, [ApiEvent::AccessExpired]);
        assert!(session);
        assert_eq!(refused("/api/transactions", r#"{"detail":{"error":"scope_token_expired"}}"#).0, [ApiEvent::ScopeExpired]);
        assert_eq!(refused("/api/transactions", r#"{"detail":{"error":"launch_required"}}"#).0, [ApiEvent::LaunchRequired]);
    }

    #[test]
    fn the_sign_in_form_reads_its_own_refusals() {
        // A wrong account at the form is the form's to explain; nobody is signed out by it.
        for body in [r#"{"detail":"inactive_user"}"#, r#"{"ok":false,"error":"inactive_user"}"#, r#"{"detail":{"error":"access_expired"}}"#] {
            let (told, session) = refused("/api/auth/login", body);
            assert_eq!(told, [], "{body}");
            assert!(session);
        }
        assert_eq!(refused("/api/auth/login", r#"{"detail":{"error":"launch_required"}}"#).0, [ApiEvent::LaunchRequired]);
    }

    #[test]
    fn refusals_are_put_into_words() {
        let said = |body: &str| error_from_body(403, body.as_bytes()).to_string();
        assert_eq!(said(r#"{"detail":{"error":"workspace_forbidden","workspace":"time"}}"#), "Доступ к этому рабочему пространству закрыт администратором");
        assert_eq!(said(r#"{"detail":{"error":"time_role_forbidden","required":"manager"}}"#), "Недостаточно прав: в «Учёте времени» открыт только просмотр");
        let ApiError::Http { code, .. } = error_from_body(403, br#"{"detail":{"error":"workspace_forbidden","workspace":"time"}}"#) else { panic!() };
        assert_eq!(code.as_deref(), Some("workspace_forbidden"));
    }

    /// A server of canned answers: for a path, how long it thinks and what it says.
    async fn canned(answer: fn(&str) -> (u64, u16, &'static str)) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut seen = Vec::new();
                    let mut chunk = [0u8; 2048];
                    let head = loop {
                        if let Some(at) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at + 4;
                        }
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => seen.extend_from_slice(&chunk[..n]),
                        }
                    };
                    let text = String::from_utf8_lossy(&seen[..head]).to_lowercase();
                    let length = text.lines().find_map(|line| line.strip_prefix("content-length:")).and_then(|value| value.trim().parse::<usize>().ok()).unwrap_or(0);
                    while seen.len() < head + length {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => seen.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let target = String::from_utf8_lossy(&seen[..head]).split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let (thinks, status, body) = answer(target.split('?').next().unwrap_or("/"));
                    tokio::time::sleep(std::time::Duration::from_millis(thinks)).await;
                    let reply = format!("HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(reply.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        base
    }

    fn signs_in(api: &Api, who: &str) {
        assert!(api.begin_session(api.epoch(), Some(format!("access-{who}")), Some(format!("refresh-{who}-0123456789")), None));
    }

    fn told(events: &mut broadcast::Receiver<ApiEvent>) -> Vec<ApiEvent> {
        let mut told = Vec::new();
        while let Ok(event) = events.try_recv() {
            told.push(event);
        }
        told
    }

    async fn a_moment(millis: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
    }

    #[tokio::test]
    async fn late_news_of_a_session_that_ended_is_not_news_of_the_next() {
        let base = canned(|path| match path {
            "/api/time/meta" => (200, 403, r#"{"detail":{"error":"workspace_forbidden","workspace":"time"}}"#),
            "/api/transactions" => (200, 401, r#"{"detail":"Not authenticated"}"#),
            "/api/admin/users" => (200, 403, r#"{"detail":"inactive_user"}"#),
            // Nothing else is to be asked: a refresh here would be on the wrong session.
            _ => (0, 500, "{}"),
        })
        .await;
        let api = Api::new(base, None, Arc::new(MemoryStore::default()));
        signs_in(&api, "anna");
        let mut events = api.subscribe();
        let asked: Vec<_> = ["/api/time/meta", "/api/transactions", "/api/admin/users"]
            .into_iter()
            .map(|path| {
                let api = api.clone();
                tokio::spawn(async move { api.execute(&Call::new(Method::GET, path)).await })
            })
            .collect();
        a_moment(60).await;
        // Anna signs out and Boris signs in while her three requests are on their way.
        api.clear_session();
        signs_in(&api, "boris");
        let mut statuses = Vec::new();
        for answer in asked {
            statuses.push(answer.await.unwrap().expect("asked again, on the session of the next account").status);
        }
        assert_eq!(statuses, [403, 401, 403]);
        assert_eq!(told(&mut events), [], "the next account was told of the last one's refusals");
        assert_eq!(api.refresh_token().as_deref(), Some("refresh-boris-0123456789"), "the next account's session was ended by the last one's answers");
        assert!(api.has_auth_token());
    }

    #[tokio::test]
    async fn the_same_news_in_time_is_still_told() {
        let base = canned(|path| match path {
            "/api/time/meta" => (0, 403, r#"{"detail":{"error":"workspace_forbidden","workspace":"time"}}"#),
            "/api/auth/refresh" => (0, 401, r#"{"ok":false,"error":"revoked"}"#),
            _ => (0, 401, r#"{"detail":"Not authenticated"}"#),
        })
        .await;
        let api = Api::new(base, None, Arc::new(MemoryStore::default()));
        signs_in(&api, "anna");
        let mut events = api.subscribe();
        assert_eq!(api.execute(&Call::new(Method::GET, "/api/time/meta")).await.unwrap().status, 403);
        assert_eq!(told(&mut events), [ApiEvent::WorkspaceRevoked("time".into())]);
        assert!(api.has_refresh_token());
        assert_eq!(api.execute(&Call::new(Method::GET, "/api/transactions")).await.unwrap().status, 401);
        assert_eq!(told(&mut events), [ApiEvent::SessionRevoked]);
        assert!(!api.has_auth_token() && !api.has_refresh_token());
    }

    #[tokio::test]
    async fn tokens_renewed_for_a_session_that_ended_meanwhile_are_not_kept() {
        let base = canned(|path| match path {
            "/api/auth/refresh" => (200, 200, r#"{"ok":true,"token":"access-renewed","refresh_token":"refresh-renewed-0123456789"}"#),
            _ => (0, 401, r#"{"detail":"Not authenticated"}"#),
        })
        .await;
        let vault = Arc::new(MemoryStore::default());
        let api = Api::new(base.clone(), None, vault.clone());
        signs_in(&api, "anna");
        let mut events = api.subscribe();
        let asking = api.clone();
        let asked = tokio::spawn(async move { asking.execute(&Call::new(Method::GET, "/api/transactions")).await });
        a_moment(80).await;
        // Signed out while the renewal was on its way.
        api.clear_session();
        assert_eq!(asked.await.unwrap().unwrap().status, 401);
        assert!(!api.has_auth_token() && !api.has_refresh_token());
        assert_eq!(vault.get(&format!("refresh:{base}")), None, "the renewed session went into the keyring after the sign-out");
        assert_eq!(told(&mut events), []);
    }

    #[tokio::test]
    async fn a_sign_in_overtaken_by_another_is_not_a_sign_in() {
        let base = canned(|path| match path {
            "/api/auth/login" => (200, 200, r#"{"ok":true,"token":"access-anna","refresh_token":"refresh-anna-0123456789","user":{"id":1,"username":"anna"}}"#),
            _ => (0, 500, "{}"),
        })
        .await;
        let api = Api::new(base, None, Arc::new(MemoryStore::default()));
        let slow = api.clone();
        let first = tokio::spawn(async move { slow.login("anna", "secret").await });
        a_moment(60).await;
        // Somebody else's sign-in, by QR say, finishes first.
        signs_in(&api, "boris");
        let answer = first.await.unwrap();
        assert!(matches!(&answer, Err(ApiError::Http { code: Some(code), .. }) if code == "sign_in_overtaken"), "{answer:?}");
        assert_eq!(api.refresh_token().as_deref(), Some("refresh-boris-0123456789"));

        // Nobody in the way: the same answer signs Anna in.
        api.clear_session();
        let answer = api.login("anna", "secret").await.unwrap();
        assert!(answer.ok && api.refresh_token().as_deref() == Some("refresh-anna-0123456789"));
    }
}
