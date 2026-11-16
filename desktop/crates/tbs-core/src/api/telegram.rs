//! Telegram through the server's TDLib client: its sign-in, chats, messages,
//! receipts and mini apps. The shapes are those of the server's `/api/tg`.

use std::sync::Arc;

use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Api, ApiError, Body, Call, attachment_name};
use crate::model::{Transaction, null_default};

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgUser {
    pub id: Option<i64>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub username: Option<String>,
    pub phone_number: Option<String>,
}

/// Where TDLib's own sign-in stands. `state` is `ready`, `wait_phone_number`,
/// `wait_code`, `wait_password`, `tdlib_unavailable`, `misconfigured` or a
/// step in between.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgStatus {
    #[serde(deserialize_with = "null_default")]
    pub state: String,
    #[serde(deserialize_with = "null_default")]
    pub raw_state: String,
    #[serde(deserialize_with = "null_default")]
    pub is_authorized: bool,
    pub phone_number: Option<String>,
    pub user: Option<TgUser>,
    pub code_type: Option<String>,
    pub code_timeout: Option<i64>,
}

impl TgStatus {
    pub fn is_ready(&self) -> bool {
        self.state == "ready"
    }
}

/// A button under a message. `kind` is `web_app`, `url`, `login_url`,
/// `copy_text`, or one that needs the bot's answer.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgButton {
    #[serde(deserialize_with = "null_default")]
    pub text: String,
    #[serde(rename = "type", deserialize_with = "null_default")]
    pub kind: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgMarkup {
    /// `inline` or `keyboard`.
    #[serde(deserialize_with = "null_default")]
    pub kind: String,
    #[serde(deserialize_with = "null_default")]
    pub rows: Vec<Vec<TgButton>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgFile {
    pub file_id: Option<i64>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgMessage {
    pub id: Option<i64>,
    pub date: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub is_outgoing: bool,
    /// A bare number, or TDLib's own `{ user_id }` / `{ chat_id }`.
    pub sender_id: Option<Value>,
    pub text: Option<String>,
    pub document: Option<TgFile>,
    pub photo: Option<TgFile>,
    pub reply_markup: Option<TgMarkup>,
    pub processed: Option<bool>,
    pub processed_status: Option<String>,
    pub transaction_id: Option<i64>,
    pub processed_at: Option<String>,
    pub processing_error: Option<String>,
}

impl TgMessage {
    pub fn sender(&self) -> Option<i64> {
        let sender = self.sender_id.as_ref()?;
        sender.as_i64().or_else(|| sender["user_id"].as_i64()).or_else(|| sender["chat_id"].as_i64())
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgChat {
    pub chat_id: i64,
    #[serde(deserialize_with = "null_default")]
    pub title: String,
    pub username: Option<String>,
    /// `bot`, `user`, `group`, `supergroup` or `channel`.
    #[serde(deserialize_with = "null_default")]
    pub chat_type: String,
    pub member_count: Option<i64>,
    #[serde(deserialize_with = "null_default")]
    pub is_hidden: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_protected: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_monitored: bool,
    #[serde(deserialize_with = "null_default")]
    pub monitor_enabled: bool,
    pub last_message: Option<TgMessage>,
    pub last_processed_message_id: Option<i64>,
    pub last_processed_message_date: Option<String>,
    pub latest_message_id: Option<i64>,
    pub latest_message_date: Option<String>,
    pub backlog_lag_messages: Option<i64>,
    pub backlog_state: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgChatPage {
    #[serde(deserialize_with = "null_default")]
    pub total: i64,
    #[serde(deserialize_with = "null_default")]
    pub items: Vec<TgChat>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgMessagePage {
    #[serde(deserialize_with = "null_default")]
    pub total: i64,
    #[serde(deserialize_with = "null_default")]
    pub items: Vec<TgMessage>,
}

/// What became of one message sent to the receipt worker. `status` is
/// `queued`, `processing`, `done`, `failed` or `not_processed`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgProcess {
    pub chat_id: i64,
    pub message_id: i64,
    pub task_id: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    pub transaction_id: Option<i64>,
    pub error: Option<String>,
}

impl TgProcess {
    pub fn settled(&self) -> bool {
        matches!(self.status.as_str(), "done" | "failed")
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProcessBatch {
    #[serde(deserialize_with = "null_default")]
    results: Vec<TgProcess>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgMonitor {
    pub chat_id: i64,
    #[serde(deserialize_with = "null_default")]
    pub enabled: bool,
    pub last_error: Option<String>,
    pub backlog_state: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgHidden {
    pub chat_id: i64,
    #[serde(deserialize_with = "null_default")]
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgProtection {
    pub chat_id: i64,
    #[serde(deserialize_with = "null_default")]
    pub protected: bool,
}

/// The answer to a chat password. The pass it earns stays inside [`Api`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TgUnlock {
    pub verified: bool,
    pub attempts_left: Option<i64>,
    pub locked: bool,
    pub locked_until: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UnlockReply {
    #[serde(deserialize_with = "null_default")]
    verified: bool,
    session_token: Option<String>,
    attempts_left: Option<i64>,
    #[serde(deserialize_with = "null_default")]
    locked: bool,
    locked_until: Option<String>,
}

/// Which button a mini app was opened from; the server launches each its own way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TgButtonSource {
    Inline,
    Keyboard,
    Menu,
}

impl TgButtonSource {
    pub fn code(self) -> &'static str {
        match self {
            TgButtonSource::Inline => "inline",
            TgButtonSource::Keyboard => "keyboard",
            TgButtonSource::Menu => "menu",
        }
    }
}

/// A mini app ready to load: the address Telegram issued for this launch.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgWebApp {
    #[serde(deserialize_with = "null_default")]
    pub url: String,
    pub launch_id: Option<String>,
}

/// The mini app a bot puts on its menu button.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TgMenuButton {
    #[serde(deserialize_with = "null_default")]
    pub available: bool,
    pub text: Option<String>,
    pub url: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub is_main: bool,
    pub bot_user_id: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TgDownload {
    pub name: Option<String>,
    pub mime: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReceiptParsing {
    pub method: Option<String>,
    pub confidence: Option<f64>,
    pub notes: Option<String>,
}

/// A receipt turned into a transaction right away, without the queue.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReceiptOutcome {
    #[serde(deserialize_with = "null_default")]
    pub created: bool,
    #[serde(deserialize_with = "null_default")]
    pub duplicate: bool,
    pub transaction: Transaction,
    pub parsing: ReceiptParsing,
}

/// The type a browser would give a picked file: the server judges uploads by it.
pub fn upload_mime(filename: &str) -> &'static str {
    let extension = filename.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

fn chat_path(chat: i64, rest: &str) -> String {
    format!("/api/tg/chats/{chat}{rest}")
}

impl Api {
    pub async fn tg_status(&self) -> Result<TgStatus, ApiError> {
        self.get("/api/tg/status", &[]).await
    }

    pub async fn tg_send_phone(&self, phone: &str) -> Result<TgStatus, ApiError> {
        self.post("/api/tg/auth/phone", &json!({ "phone_number": phone })).await
    }

    pub async fn tg_send_code(&self, code: &str) -> Result<TgStatus, ApiError> {
        self.post("/api/tg/auth/code", &json!({ "code": code })).await
    }

    pub async fn tg_send_password(&self, password: &str) -> Result<TgStatus, ApiError> {
        self.post("/api/tg/auth/password", &json!({ "password": password })).await
    }

    pub async fn tg_chats(&self, limit: usize, include_hidden: bool) -> Result<TgChatPage, ApiError> {
        self.get("/api/tg/chats", &[("limit", limit.to_string()), ("include_hidden", include_hidden.to_string())]).await
    }

    pub async fn tg_set_hidden(&self, chat: i64, hidden: bool) -> Result<TgHidden, ApiError> {
        self.put(&chat_path(chat, "/hidden"), &json!({ "hidden": hidden })).await
    }

    pub async fn tg_set_monitor(&self, chat: i64, enabled: bool) -> Result<TgMonitor, ApiError> {
        self.put(&format!("/api/tg/monitors/{chat}"), &json!({ "enabled": enabled, "start_from_latest": true })).await
    }

    pub async fn tg_set_chat_password(&self, chat: i64, password: &str) -> Result<TgProtection, ApiError> {
        self.post(&chat_path(chat, "/password"), &json!({ "password": password })).await
    }

    pub async fn tg_remove_chat_password(&self, chat: i64, password: &str) -> Result<TgProtection, ApiError> {
        let call = Call::new(Method::DELETE, chat_path(chat, "/password")).json(&json!({ "password": password }));
        let reply = self.send(call).await?;
        self.set_chat_access(chat, None);
        Ok(reply)
    }

    /// A right password earns a pass for the chat, kept here and sent with
    /// every later call into it.
    pub async fn tg_unlock_chat(&self, chat: i64, password: &str) -> Result<TgUnlock, ApiError> {
        let reply: UnlockReply = self.post(&chat_path(chat, "/password/verify"), &json!({ "password": password })).await?;
        if reply.verified && reply.session_token.is_some() {
            self.set_chat_access(chat, reply.session_token);
        }
        Ok(TgUnlock { verified: reply.verified, attempts_left: reply.attempts_left, locked: reply.locked, locked_until: reply.locked_until })
    }

    fn in_chat(&self, chat: i64, call: Call) -> Call {
        match self.chat_access(chat) {
            Some(pass) => call.header("X-Chat-Access", pass),
            None => call,
        }
    }

    /// A pass the server stopped honouring is dropped, so the chat asks for
    /// its password again instead of failing the same way forever.
    fn judge<T>(&self, chat: i64, outcome: Result<T, ApiError>) -> Result<T, ApiError> {
        if let Err(ApiError::Http { status: 403, code: Some(code), .. }) = &outcome
            && code == "chat_locked"
        {
            self.set_chat_access(chat, None);
        }
        outcome
    }

    async fn chat_call<T: serde::de::DeserializeOwned>(&self, chat: i64, call: Call) -> Result<T, ApiError> {
        let outcome = self.send(self.in_chat(chat, call)).await;
        self.judge(chat, outcome)
    }

    pub async fn tg_messages(&self, chat: i64, limit: usize) -> Result<TgMessagePage, ApiError> {
        self.chat_call(chat, Call::new(Method::GET, chat_path(chat, "/messages")).query(&[("limit", limit.to_string())])).await
    }

    pub async fn tg_send_message(&self, chat: i64, text: &str) -> Result<TgMessage, ApiError> {
        self.chat_call(chat, Call::new(Method::POST, chat_path(chat, "/messages")).json(&json!({ "text": text }))).await
    }

    pub async fn tg_send_document(&self, chat: i64, filename: &str, bytes: Vec<u8>) -> Result<TgMessage, ApiError> {
        let mut call = Call::new(Method::POST, chat_path(chat, "/documents"));
        call.body = Body::File { field: "file".into(), filename: filename.into(), mime: Some(upload_mime(filename).into()), bytes: Arc::new(bytes) };
        self.chat_call(chat, call).await
    }

    /// Hands one message to the receipt worker.
    pub async fn tg_process_message(&self, chat: i64, message: i64) -> Result<TgProcess, ApiError> {
        self.chat_call(chat, Call::new(Method::POST, chat_path(chat, &format!("/messages/{message}/process"))).json(&json!({}))).await
    }

    pub async fn tg_receipt_status(&self, chat: i64, messages: &[i64]) -> Result<Vec<TgProcess>, ApiError> {
        let ids = messages.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let batch: ProcessBatch = self.chat_call(chat, Call::new(Method::GET, chat_path(chat, "/receipt-status")).query(&[("message_ids", ids)])).await?;
        Ok(batch.results)
    }

    pub async fn tg_menu_button(&self, chat: i64) -> Result<TgMenuButton, ApiError> {
        self.chat_call(chat, Call::new(Method::GET, chat_path(chat, "/menu-button"))).await
    }

    pub async fn tg_open_web_app(&self, chat: i64, url: &str, bot: Option<i64>, source: TgButtonSource) -> Result<TgWebApp, ApiError> {
        let body = json!({ "url": url, "bot_user_id": bot, "button_kind": source.code() });
        self.chat_call(chat, Call::new(Method::POST, chat_path(chat, "/webapp/open")).json(&body)).await
    }

    pub async fn tg_close_web_app(&self, chat: i64, launch_id: &str) -> Result<(), ApiError> {
        let _: Value = self.chat_call(chat, Call::new(Method::POST, chat_path(chat, "/webapp/close")).json(&json!({ "launch_id": launch_id }))).await?;
        Ok(())
    }

    /// What a mini app opened from a keyboard button sends back to its bot.
    pub async fn tg_send_web_app_data(&self, chat: i64, bot: Option<i64>, button_text: &str, data: &str) -> Result<(), ApiError> {
        let body = json!({ "bot_user_id": bot, "button_text": button_text, "data": data });
        let _: Value = self.chat_call(chat, Call::new(Method::POST, chat_path(chat, "/webapp/data")).json(&body)).await?;
        Ok(())
    }

    /// A file of a message, reached only through the chat and the message
    /// that own it.
    pub async fn tg_file(&self, chat: i64, message: i64, file: i64, filename: Option<&str>) -> Result<TgDownload, ApiError> {
        let mut call = Call::new(Method::GET, chat_path(chat, &format!("/messages/{message}/files/{file}")));
        if let Some(name) = filename {
            call = call.query(&[("filename", name.to_owned())]);
        }
        let outcome = self.checked(&self.in_chat(chat, call)).await;
        let reply = self.judge(chat, outcome)?;
        let name = reply.header("content-disposition").and_then(attachment_name);
        let mime = reply.header("content-type").map(str::to_owned);
        Ok(TgDownload { name, mime, bytes: reply.bytes })
    }

    /// Turns one Telegram message into a transaction at once and returns it.
    pub async fn process_receipt(&self, chat: i64, message: i64, force: bool) -> Result<ReceiptOutcome, ApiError> {
        self.post("/api/transactions/process-receipt", &json!({ "chat_id": chat, "message_id": message, "force": force })).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::secret::MemoryStore;

    #[test]
    fn a_status_reads_with_its_nulls() {
        let status: TgStatus = serde_json::from_str(
            r#"{"state":"wait_code","raw_state":"authorizationStateWaitCode","is_authorized":false,"phone_number":null,"user":null,"code_type":"authenticationCodeTypeTelegramMessage","code_timeout":null}"#,
        )
        .unwrap();
        assert_eq!(status.state, "wait_code");
        assert!(!status.is_ready() && !status.is_authorized);
        assert_eq!(status.code_type.as_deref(), Some("authenticationCodeTypeTelegramMessage"));
        let ready: TgStatus = serde_json::from_str(r#"{"state":"ready","raw_state":"authorizationStateReady","is_authorized":true,"user":{"id":7,"first_name":"Тест","username":null}}"#).unwrap();
        assert!(ready.is_ready());
        assert_eq!(ready.user.unwrap().first_name.as_deref(), Some("Тест"));
    }

    #[test]
    fn a_chat_list_reads_as_the_server_sends_it() {
        let page: TgChatPage = serde_json::from_str(
            r#"{"total":2,"items":[
                {"chat_id":-1001,"title":"Касса","username":null,"chat_type":"supergroup","member_count":12,"is_hidden":false,"is_protected":true,
                 "is_monitored":true,"monitor_enabled":null,"last_message":{"id":5,"date":"2026-11-11T10:00:00Z","is_outgoing":false,"sender_id":41,"text":"Оплата","document":null,"photo":null,"reply_markup":null},
                 "last_processed_message_id":4,"latest_message_id":5,"latest_message_date":"2026-11-11T10:00:00Z","backlog_lag_messages":1,"backlog_state":"lagging"},
                {"chat_id":77,"title":"Бот","chat_type":"bot"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(page.total, 2);
        let first = &page.items[0];
        assert!(first.is_protected && first.is_monitored && !first.monitor_enabled && !first.is_hidden);
        assert_eq!(first.last_message.as_ref().unwrap().text.as_deref(), Some("Оплата"));
        assert_eq!(first.backlog_lag_messages, Some(1));
        assert_eq!(page.items[1], TgChat { chat_id: 77, title: "Бот".into(), chat_type: "bot".into(), ..TgChat::default() });
    }

    #[test]
    fn messages_read_with_files_buttons_and_receipt_marks() {
        let page: TgMessagePage = serde_json::from_str(
            r#"{"total":3,"items":[
                {"id":3,"date":"2026-11-11T10:02:00Z","is_outgoing":true,"sender_id":{"user_id":9},"text":"[Photo]","document":null,
                 "photo":{"file_id":31,"file_name":"photo.jpg","mime_type":"image/jpeg","size":2048,"local_path":null,"download_url":"/api/tg/files/31","width":800,"height":600},
                 "reply_markup":null,"processed":true,"processed_status":"done","transaction_id":15,"processed_at":"2026-11-11T10:03:00","processing_error":null},
                {"id":2,"date":"2026-11-11T10:01:00Z","is_outgoing":false,"sender_id":{"chat_id":-5},"text":"чек.pdf",
                 "document":{"file_id":21,"file_name":"чек.pdf","mime_type":"application/pdf","size":10240,"local_path":"","download_url":"/api/tg/files/21"},
                 "photo":null,"reply_markup":{"kind":"inline","rows":[[{"text":"Открыть","type":"web_app","url":"https://app.example.com/"},{"text":null,"type":"callback","url":null}]]},
                 "processed":false,"processed_status":"failed","processing_error":"Парсер не смог распознать чек"},
                {"id":null,"date":null,"text":null}
            ]}"#,
        )
        .unwrap();
        let [photo, document, bare] = page.items.as_slice() else { panic!("three messages") };
        assert_eq!(photo.sender(), Some(9));
        assert_eq!(photo.photo.as_ref().unwrap().width, Some(800));
        assert_eq!((photo.processed, photo.transaction_id), (Some(true), Some(15)));
        assert_eq!(document.sender(), Some(-5));
        assert_eq!(document.document.as_ref().unwrap().file_id, Some(21));
        let markup = document.reply_markup.as_ref().unwrap();
        assert_eq!((markup.kind.as_str(), markup.rows[0].len()), ("inline", 2));
        assert_eq!(markup.rows[0][0], TgButton { text: "Открыть".into(), kind: "web_app".into(), url: Some("https://app.example.com/".into()) });
        assert_eq!(markup.rows[0][1].text, "");
        assert_eq!(document.processed_status.as_deref(), Some("failed"));
        assert_eq!((bare.id, bare.sender(), bare.is_outgoing), (None, None, false));
    }

    #[test]
    fn the_small_answers_read() {
        let process: TgProcess = serde_json::from_str(r#"{"chat_id":1,"message_id":2,"task_id":"t-1","status":"queued","transaction_id":null,"error":null}"#).unwrap();
        assert!(!process.settled());
        let batch: ProcessBatch = serde_json::from_str(r#"{"results":[{"chat_id":1,"message_id":2,"status":"done","transaction_id":8}]}"#).unwrap();
        assert!(batch.results[0].settled());
        let menu: TgMenuButton = serde_json::from_str(r#"{"available":true,"text":"Магазин","url":"https://shop.example.com/","is_main":false,"bot_user_id":77}"#).unwrap();
        assert_eq!((menu.available, menu.bot_user_id), (true, Some(77)));
        let none: TgMenuButton = serde_json::from_str(r#"{"available":false,"text":null,"url":null,"is_main":false,"bot_user_id":null}"#).unwrap();
        assert!(!none.available && none.url.is_none());
        let app: TgWebApp = serde_json::from_str(r#"{"url":"https://app.example.com/?tgWebAppData=x","launch_id":"12345678901234567890"}"#).unwrap();
        assert_eq!(app.launch_id.as_deref(), Some("12345678901234567890"));
        let monitor: TgMonitor = serde_json::from_str(r#"{"chat_id":5,"enabled":true,"last_processed_message_id":0,"last_error":null}"#).unwrap();
        assert!(monitor.enabled);
        let receipt: ReceiptOutcome =
            serde_json::from_str(r#"{"created":true,"duplicate":false,"transaction":{"id":9},"parsing":{"method":"regex","confidence":0.9,"notes":null}}"#).unwrap();
        assert!(receipt.created && receipt.transaction.id == 9);
        assert_eq!(receipt.parsing.method.as_deref(), Some("regex"));
    }

    #[test]
    fn an_upload_is_typed_as_a_browser_would_type_it() {
        assert_eq!(upload_mime("Чек № 4.PDF"), "application/pdf");
        assert_eq!(upload_mime("scan.jpeg"), "image/jpeg");
        assert_eq!(upload_mime("no-extension"), "application/octet-stream");
    }

    struct Seen {
        line: String,
        head: String,
        body: String,
    }

    /// A server that keeps what it was asked and answers by path.
    async fn wire(answer: fn(&str, &str) -> (u16, &'static str)) -> (Api, Arc<Mutex<Vec<Seen>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let log = log.clone();
                tokio::spawn(async move {
                    let mut got = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let head = loop {
                        if let Some(at) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at + 4;
                        }
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => got.extend_from_slice(&chunk[..n]),
                        }
                    };
                    let text = String::from_utf8_lossy(&got[..head]).to_lowercase();
                    let length = text.lines().find_map(|line| line.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    while got.len() < head + length {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => got.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let line = String::from_utf8_lossy(&got[..head]).lines().next().unwrap_or_default().to_owned();
                    let mut parts = line.split_whitespace();
                    let (method, target) = (parts.next().unwrap_or_default().to_owned(), parts.next().unwrap_or("/").to_owned());
                    let (status, body) = answer(&method, target.split('?').next().unwrap_or("/"));
                    log.lock().unwrap().push(Seen { line: format!("{method} {target}"), head: text, body: String::from_utf8_lossy(&got[head..]).into_owned() });
                    let reply = format!("HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-disposition: attachment; filename=\"a.pdf\"\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(reply.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        let api = Api::new(base, None, Arc::new(MemoryStore::default()));
        assert!(api.begin_session(api.epoch(), Some("access-token".into()), Some("refresh-token-0123456789".into()), None));
        (api, seen)
    }

    fn lines(seen: &Arc<Mutex<Vec<Seen>>>) -> Vec<String> {
        seen.lock().unwrap().iter().map(|request| request.line.clone()).collect()
    }

    fn body_of(seen: &Arc<Mutex<Vec<Seen>>>, at: usize) -> Value {
        serde_json::from_str(&seen.lock().unwrap()[at].body).unwrap()
    }

    #[tokio::test]
    async fn the_sign_in_steps_go_where_the_server_listens() {
        let (api, seen) = wire(|_, _| (200, r#"{"state":"wait_code","raw_state":"x","is_authorized":false}"#)).await;
        assert_eq!(api.tg_status().await.unwrap().state, "wait_code");
        api.tg_send_phone("+10000000000").await.unwrap();
        api.tg_send_code("12345").await.unwrap();
        api.tg_send_password("пароль").await.unwrap();
        assert_eq!(lines(&seen), ["GET /api/tg/status", "POST /api/tg/auth/phone", "POST /api/tg/auth/code", "POST /api/tg/auth/password"]);
        assert_eq!(body_of(&seen, 1), json!({ "phone_number": "+10000000000" }));
        assert_eq!(body_of(&seen, 2), json!({ "code": "12345" }));
        assert_eq!(body_of(&seen, 3), json!({ "password": "пароль" }));
        assert!(seen.lock().unwrap()[0].head.contains("authorization: bearer access-token"));
    }

    #[tokio::test]
    async fn chats_and_their_switches_are_asked_for_by_their_paths() {
        let (api, seen) = wire(|_, path| match path {
            "/api/tg/chats" => (200, r#"{"total":0,"items":[]}"#),
            _ => (200, r#"{"chat_id":-10,"hidden":true,"enabled":true,"protected":true,"updated_at":"2026-11-11T10:00:00"}"#),
        })
        .await;
        api.tg_chats(200, true).await.unwrap();
        assert!(api.tg_set_hidden(-10, true).await.unwrap().hidden);
        assert!(api.tg_set_monitor(-10, true).await.unwrap().enabled);
        assert!(api.tg_set_chat_password(-10, "секрет").await.unwrap().protected);
        api.tg_remove_chat_password(-10, "секрет").await.unwrap();
        assert_eq!(
            lines(&seen),
            ["GET /api/tg/chats?limit=200&include_hidden=true", "PUT /api/tg/chats/-10/hidden", "PUT /api/tg/monitors/-10", "POST /api/tg/chats/-10/password", "DELETE /api/tg/chats/-10/password"]
        );
        assert_eq!(body_of(&seen, 1), json!({ "hidden": true }));
        assert_eq!(body_of(&seen, 2), json!({ "enabled": true, "start_from_latest": true }));
        assert_eq!(body_of(&seen, 4), json!({ "password": "секрет" }));
    }

    #[tokio::test]
    async fn work_inside_a_chat_is_asked_for_by_its_paths() {
        let (api, seen) = wire(|_, path| match path {
            "/api/tg/chats/5/messages" => (200, r#"{"total":0,"items":[],"id":1}"#),
            "/api/tg/chats/5/receipt-status" => (200, r#"{"results":[{"chat_id":5,"message_id":8,"status":"done"}]}"#),
            "/api/tg/chats/5/webapp/open" => (200, r#"{"url":"https://app.example.com/","launch_id":null}"#),
            "/api/tg/chats/5/messages/8/files/3" => (200, "%PDF"),
            _ => (200, r#"{"chat_id":5,"message_id":8,"status":"queued","ok":true,"available":false}"#),
        })
        .await;
        api.tg_messages(5, 100).await.unwrap();
        api.tg_send_message(5, "привет").await.unwrap();
        assert_eq!(api.tg_process_message(5, 8).await.unwrap().status, "queued");
        assert_eq!(api.tg_receipt_status(5, &[8, 9]).await.unwrap()[0].status, "done");
        assert!(!api.tg_menu_button(5).await.unwrap().available);
        assert_eq!(api.tg_open_web_app(5, "https://app.example.com/", Some(77), TgButtonSource::Menu).await.unwrap().launch_id, None);
        api.tg_close_web_app(5, "123").await.unwrap();
        api.tg_send_web_app_data(5, None, "Кнопка", "{}").await.unwrap();
        let file = api.tg_file(5, 8, 3, Some("чек.pdf")).await.unwrap();
        assert_eq!((file.name.as_deref(), file.bytes.as_slice()), (Some("a.pdf"), b"%PDF".as_slice()));
        assert_eq!(
            lines(&seen),
            [
                "GET /api/tg/chats/5/messages?limit=100",
                "POST /api/tg/chats/5/messages",
                "POST /api/tg/chats/5/messages/8/process",
                "GET /api/tg/chats/5/receipt-status?message_ids=8%2C9",
                "GET /api/tg/chats/5/menu-button",
                "POST /api/tg/chats/5/webapp/open",
                "POST /api/tg/chats/5/webapp/close",
                "POST /api/tg/chats/5/webapp/data",
                "GET /api/tg/chats/5/messages/8/files/3?filename=%D1%87%D0%B5%D0%BA.pdf",
            ]
        );
        assert_eq!(body_of(&seen, 1), json!({ "text": "привет" }));
        assert_eq!(body_of(&seen, 5), json!({ "url": "https://app.example.com/", "bot_user_id": 77, "button_kind": "menu" }));
        assert_eq!(body_of(&seen, 6), json!({ "launch_id": "123" }));
        assert_eq!(body_of(&seen, 7), json!({ "bot_user_id": null, "button_text": "Кнопка", "data": "{}" }));
    }

    #[tokio::test]
    async fn a_document_goes_as_a_typed_file_part() {
        let (api, seen) = wire(|_, _| (200, r#"{"id":4,"text":"[PDF]"}"#)).await;
        assert_eq!(api.tg_send_document(5, "чек.pdf", b"%PDF-1.7".to_vec()).await.unwrap().id, Some(4));
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].line, "POST /api/tg/chats/5/documents");
        assert!(seen[0].head.contains("content-type: multipart/form-data"));
        let body = seen[0].body.to_lowercase();
        assert!(body.contains("name=\"file\"") && body.contains("content-type: application/pdf") && body.contains("%pdf-1.7"), "{body}");
    }

    #[tokio::test]
    async fn a_chat_pass_is_kept_sent_and_dropped() {
        let (api, seen) = wire(|method, path| match (method, path) {
            ("POST", "/api/tg/chats/5/password/verify") => (200, r#"{"verified":true,"session_token":"chat-pass","expires_in":3600}"#),
            ("POST", "/api/tg/chats/6/password/verify") => (200, r#"{"verified":false,"attempts_left":2,"locked":false,"locked_until":null}"#),
            ("GET", "/api/tg/chats/5/menu-button") => (403, r#"{"detail":{"error":"chat_locked","chat_id":5}}"#),
            _ => (200, r#"{"total":0,"items":[]}"#),
        })
        .await;
        let wrong = api.tg_unlock_chat(6, "не тот").await.unwrap();
        assert_eq!((wrong.verified, wrong.attempts_left), (false, Some(2)));
        api.tg_messages(6, 100).await.unwrap();
        assert!(api.tg_unlock_chat(5, "тот").await.unwrap().verified);
        api.tg_messages(5, 100).await.unwrap();
        api.tg_messages(6, 100).await.unwrap();
        // The server stopped honouring the pass: it is not sent again.
        let refused = api.tg_menu_button(5).await.unwrap_err();
        assert_eq!(refused.status(), Some(403));
        api.tg_messages(5, 100).await.unwrap();
        let passes: Vec<bool> = seen.lock().unwrap().iter().map(|request| request.head.contains("x-chat-access: chat-pass")).collect();
        assert_eq!(passes, [false, false, false, true, false, true, false]);
        // A sign-out takes every pass with it.
        assert!(api.tg_unlock_chat(5, "тот").await.unwrap().verified);
        api.clear_session();
        assert_eq!(api.chat_access(5), None);
    }

    #[tokio::test]
    async fn a_receipt_is_processed_through_the_transactions_route() {
        let (api, seen) = wire(|_, _| (200, r#"{"created":false,"duplicate":true,"transaction":{"id":3},"parsing":{}}"#)).await;
        assert!(api.process_receipt(-7, 12, true).await.unwrap().duplicate);
        assert_eq!(lines(&seen), ["POST /api/transactions/process-receipt"]);
        assert_eq!(body_of(&seen, 0), json!({ "chat_id": -7, "message_id": 12, "force": true }));
    }
}
