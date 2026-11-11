//! The two web views of the Telegram section: the section's own page, and a
//! mini app laid over it. Native window children live on the main thread, so
//! they stay out of the screen's state; everything here is called from there.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use iced::Subscription;
use iced::futures::Stream;
use iced::window::Window;
use serde::Deserialize;
use tbs_core::api::{Api, ApiError, Reply};
use tbs_core::links;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::http::{Request, Response, StatusCode};
use wry::{NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse, Rect, RequestAsyncResponder, WebContext, WebView, WebViewBuilder};

use super::assets::{self, CONTENT_SECURITY_POLICY, ENTRY, ORIGIN};

/// A mini app is someone else's page; one message is capped well above what
/// the bridge carries so it cannot flood the host.
const MINI_MESSAGE_LIMIT: usize = 1 << 20;

const MINI_BRIDGE: &str = r#"(() => {
    const post = window.ipc.postMessage.bind(window.ipc);
    const proxy = Object.freeze({
        postEvent(eventType, eventData) {
            if (typeof eventType !== 'string' || eventType.length > 128) return;
            post(JSON.stringify({ eventType, eventData }));
        },
    });
    Object.defineProperty(window, 'TelegramWebviewProxy', { value: proxy, writable: false, configurable: false });
})();"#;

#[derive(Clone, Debug)]
pub enum Signal {
    Page(String),
    Mini(String),
    MiniStarted,
    MiniFinished,
    External(String),
}

struct Bus {
    sender: UnboundedSender<Signal>,
    receiver: Arc<tokio::sync::Mutex<UnboundedReceiver<Signal>>>,
}

fn bus() -> &'static Bus {
    static BUS: OnceLock<Bus> = OnceLock::new();
    BUS.get_or_init(|| {
        let (sender, receiver) = unbounded_channel();
        Bus { sender, receiver: Arc::new(tokio::sync::Mutex::new(receiver)) }
    })
}

fn emit(signal: Signal) {
    let _ = bus().sender.send(signal);
}

fn stream() -> impl Stream<Item = Signal> {
    iced::futures::stream::unfold(bus().receiver.clone(), |receiver| async move {
        let next = receiver.lock().await.recv().await;
        next.map(|signal| (signal, receiver))
    })
}

/// What the web views report, delivered to the screen as messages.
pub fn signals() -> Subscription<Signal> {
    Subscription::run(stream)
}

/// A rectangle in logical pixels, relative to the window.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Area {
    fn rect(self) -> Rect {
        Rect {
            position: LogicalPosition::new(f64::from(self.x), f64::from(self.y)).into(),
            size: LogicalSize::new(f64::from(self.width.max(1.0)), f64::from(self.height.max(1.0))).into(),
        }
    }
}

#[derive(Default)]
struct Host {
    page: Option<WebView>,
    mini: Option<WebView>,
}

thread_local! {
    static HOST: RefCell<Host> = RefCell::default();
}

#[derive(Clone)]
pub struct PageSpec {
    pub api: Api,
    pub dir: PathBuf,
    pub dark: bool,
    pub area: Area,
}

fn background(dark: bool) -> (u8, u8, u8, u8) {
    if dark { (16, 16, 16, 255) } else { (255, 255, 255, 255) }
}

fn respond_with(status: u16, content_type: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(Cow::Owned(body));
    *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    if let Ok(value) = content_type.parse() {
        response.headers_mut().insert("content-type", value);
    }
    response
}

fn failure(status: u16, detail: &str) -> Response<Cow<'static, [u8]>> {
    respond_with(status, "application/json", serde_json::json!({ "detail": detail }).to_string().into_bytes())
}

/// Hands the server's answer to the page; a failure keeps the shape the page's
/// error handling reads.
fn reply_to_page(outcome: Result<Reply, ApiError>) -> Response<Cow<'static, [u8]>> {
    match outcome {
        Ok(reply) => {
            let content_type = reply.header("content-type").unwrap_or("application/octet-stream").to_owned();
            let disposition = reply.header("content-disposition").map(str::to_owned);
            let mut response = respond_with(reply.status, &content_type, reply.bytes);
            if let Some(value) = disposition.and_then(|v| v.parse().ok()) {
                response.headers_mut().insert("content-disposition", value);
            }
            if let Ok(value) = "no-store".parse() {
                response.headers_mut().insert("cache-control", value);
            }
            response
        }
        Err(ApiError::Http { status, message, .. }) => failure(status, &message),
        Err(ApiError::SessionExpired) => failure(401, "Сессия истекла"),
        Err(ApiError::Network(reason)) => failure(503, &reason),
        Err(ApiError::Decode(reason)) => failure(502, &reason),
    }
}

async fn forward(api: Api, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let (parts, body) = request.into_parts();
    let target = parts.uri.path_and_query().map_or_else(|| parts.uri.path().to_owned(), |p| p.as_str().to_owned());
    let headers = parts.headers.iter().filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned()))).collect();
    reply_to_page(api.proxy(parts.method, &target, headers, body).await)
}

fn serve(api: &Api, runtime: &Handle, request: Request<Vec<u8>>, responder: RequestAsyncResponder) {
    match assets::route(request.method().as_str(), request.uri().path()) {
        assets::Route::Asset { bytes, mime, document } => {
            let mut response = Response::new(Cow::Borrowed(bytes));
            let headers = response.headers_mut();
            if let Ok(value) = mime.parse() {
                headers.insert("content-type", value);
            }
            if document && let Ok(value) = CONTENT_SECURITY_POLICY.parse() {
                headers.insert("content-security-policy", value);
            }
            responder.respond(response);
        }
        assets::Route::Proxy => {
            let api = api.clone();
            runtime.spawn(async move { responder.respond(forward(api, request).await) });
        }
        assets::Route::Forbidden => responder.respond(failure(403, "Запрос закрыт политикой клиента")),
        assets::Route::Missing => responder.respond(failure(404, "Не найдено")),
    }
}

/// A photo opens in a window of its own; anything else goes through the link policy.
fn page_new_window(url: String) -> NewWindowResponse {
    if url.starts_with(&format!("blob:{ORIGIN}/")) {
        return NewWindowResponse::Allow;
    }
    emit(Signal::External(url));
    NewWindowResponse::Deny
}

#[cfg(all(windows, feature = "devtools"))]
fn debug_port(offset: u16) -> Option<String> {
    let port: u16 = std::env::var("TBS_WEB_DEBUG").ok()?.parse().ok()?;
    Some(format!("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={}", port + offset))
}

/// The browser's own shortcuts (F5, Ctrl+F…) and its context menu are WebView2
/// switches. The other platforms have none, and the program is only built
/// there to run its tests.
#[cfg(windows)]
fn without_browser_chrome(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    use wry::WebViewBuilderExtWindows;
    builder.with_browser_accelerator_keys(false).with_default_context_menus(false)
}

#[cfg(not(windows))]
fn without_browser_chrome(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    builder
}

pub fn create_page(window: &dyn Window, spec: PageSpec, runtime: Handle) -> Result<(), String> {
    let mut context = WebContext::new(Some(spec.dir.join("webview")));
    let api = spec.api;
    let theme = if spec.dark { "dark" } else { "light" };
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_id("telegram")
        .with_bounds(spec.area.rect())
        .with_background_color(background(spec.dark))
        .with_visible(false)
        .with_devtools(cfg!(feature = "devtools"))
        .with_hotkeys_zoom(false);
    let builder = without_browser_chrome(builder)
        .with_initialization_script("window.__TBS_NATIVE_SHELL__ = true;")
        .with_asynchronous_custom_protocol("tbs".into(), move |_, request, responder| serve(&api, &runtime, request, responder))
        .with_ipc_handler(|request| emit(Signal::Page(request.into_body())))
        .with_navigation_handler(|url| url.starts_with(&format!("{ORIGIN}/")))
        .with_new_window_req_handler(|url, _| page_new_window(url))
        .with_permission_handler(|kind| match kind {
            PermissionKind::ClipboardRead => PermissionResponse::Allow,
            _ => PermissionResponse::Deny,
        })
        .with_url(format!("{ORIGIN}/{ENTRY}?theme={theme}"));
    #[cfg(all(windows, feature = "devtools"))]
    let builder = match debug_port(0) {
        Some(args) => {
            use wry::WebViewBuilderExtWindows;
            builder.with_additional_browser_args(args)
        }
        None => builder,
    };
    let view = builder.build_as_child(&window).map_err(|error| error.to_string())?;
    HOST.with(|host| host.borrow_mut().page = Some(view));
    Ok(())
}

pub fn create_mini(window: &dyn Window, url: &str, area: Area, dir: PathBuf) -> Result<(), String> {
    if !links::is_safe_mini_app_url(url) {
        return Err("Адрес мини-приложения отклонён".to_owned());
    }
    let mut context = WebContext::new(Some(dir.join("webview-miniapp")));
    let pinned: Arc<Mutex<Option<String>>> = Arc::default();
    let (on_load, on_navigate) = (pinned.clone(), pinned);
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_id("miniapp")
        .with_bounds(area.rect())
        .with_background_color(background(true))
        .with_visible(false)
        .with_devtools(false)
        .with_hotkeys_zoom(false);
    let builder = without_browser_chrome(builder)
        .with_initialization_script_for_main_only(MINI_BRIDGE, true)
        .with_ipc_handler(|request| {
            let raw = request.into_body();
            if raw.len() <= MINI_MESSAGE_LIMIT {
                emit(Signal::Mini(raw));
            }
        })
        .with_on_page_load_handler(move |event, url| match event {
            PageLoadEvent::Started => emit(Signal::MiniStarted),
            PageLoadEvent::Finished => {
                let mut origin = on_load.lock().unwrap();
                if origin.is_none() {
                    *origin = links::origin_of(&url);
                }
                drop(origin);
                emit(Signal::MiniFinished);
            }
        })
        .with_navigation_handler(move |url| {
            let allowed = links::mini_app_may_navigate(&url, on_navigate.lock().unwrap().as_deref());
            if !allowed {
                emit(Signal::External(url));
            }
            allowed
        })
        .with_new_window_req_handler(|url, _| {
            emit(Signal::External(url));
            NewWindowResponse::Deny
        })
        .with_permission_handler(|_| PermissionResponse::Deny)
        .with_url(url);
    #[cfg(all(windows, feature = "devtools"))]
    let builder = match debug_port(1) {
        Some(args) => {
            use wry::WebViewBuilderExtWindows;
            builder.with_additional_browser_args(args).with_devtools(true)
        }
        None => builder,
    };
    let view = builder.build_as_child(&window).map_err(|error| error.to_string())?;
    HOST.with(|host| host.borrow_mut().mini = Some(view));
    Ok(())
}

fn with_page(act: impl FnOnce(&WebView)) {
    HOST.with(|host| {
        if let Some(view) = &host.borrow().page {
            act(view);
        }
    });
}

fn with_mini(act: impl FnOnce(&WebView)) {
    HOST.with(|host| {
        if let Some(view) = &host.borrow().mini {
            act(view);
        }
    });
}

pub fn run_in_page(script: &str) {
    with_page(|view| {
        if let Err(error) = view.evaluate_script(script) {
            tracing::warn!("telegram page script failed: {error}");
        }
    });
}

pub fn run_in_mini(script: &str) {
    with_mini(|view| {
        if let Err(error) = view.evaluate_script(script) {
            tracing::warn!("mini app script failed: {error}");
        }
    });
}

pub fn place_page(area: Area) {
    with_page(|view| {
        let _ = view.set_bounds(area.rect());
    });
}

pub fn place_mini(area: Area) {
    with_mini(|view| {
        let _ = view.set_bounds(area.rect());
    });
}

pub fn show_page(visible: bool) {
    with_page(|view| {
        let _ = view.set_visible(visible);
        let _ = if visible { view.focus() } else { view.focus_parent() };
    });
}

pub fn show_mini(visible: bool) {
    with_mini(|view| {
        let _ = view.set_visible(visible);
    });
}

pub fn close_mini() {
    HOST.with(|host| host.borrow_mut().mini = None);
}

pub fn close_all() {
    HOST.with(|host| *host.borrow_mut() = Host::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_failure_reaches_the_page_as_its_own_status() {
        let response = reply_to_page(Err(ApiError::Http { status: 423, code: None, message: "заперто".into() }));
        assert_eq!(response.status().as_u16(), 423);
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["detail"], "заперто");
    }

    #[test]
    fn a_lost_connection_is_a_503_with_a_reason() {
        let response = reply_to_page(Err(ApiError::Network("нет сети".into())));
        assert_eq!(response.status().as_u16(), 503);
        assert_eq!(reply_to_page(Err(ApiError::SessionExpired)).status().as_u16(), 401);
        assert_eq!(reply_to_page(Err(ApiError::Decode("x".into()))).status().as_u16(), 502);
    }

    #[test]
    fn a_reply_keeps_what_the_page_reads_and_is_not_cached() {
        let reply = Reply {
            status: 200,
            headers: vec![("content-type".into(), "image/png".into()), ("content-disposition".into(), "attachment; filename=a.png".into()), ("set-cookie".into(), "a=b".into())],
            bytes: vec![1, 2],
        };
        let response = reply_to_page(Ok(reply));
        assert_eq!(response.headers()["content-type"], "image/png");
        assert_eq!(response.headers()["content-disposition"], "attachment; filename=a.png");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(response.headers().get("set-cookie").is_none());
        assert_eq!(response.body().as_ref(), &[1, 2]);
    }

    #[test]
    fn only_the_pages_own_blobs_open_a_window() {
        assert!(matches!(page_new_window("blob:http://tbs.localhost/abc".into()), NewWindowResponse::Allow));
        assert!(matches!(page_new_window("blob:https://evil.example.com/abc".into()), NewWindowResponse::Deny));
        assert!(matches!(page_new_window("https://t.me/x".into()), NewWindowResponse::Deny));
    }

    #[test]
    fn an_area_never_collapses_to_nothing() {
        let rect = Area { x: 208.0, y: 0.0, width: -5.0, height: 0.0 }.rect();
        let size = rect.size.to_logical::<f64>(1.0);
        assert_eq!((size.width, size.height), (1.0, 1.0));
    }
}
