//! The web view of a hosted workspace page. Like the Telegram one it is a
//! native child of the window and lives on the main thread, out of the screen's
//! state. There is one at a time: a workspace is left before another opens.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use iced::Subscription;
use iced::futures::Stream;
use iced::window::Window;
use tbs_core::api::Api;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use wry::http::{Request, Response};
use wry::{NewWindowResponse, PageLoadEvent, PermissionResponse, RequestAsyncResponder, WebContext, WebView, WebViewBuilder};

use super::assets::{self, Kind};
use crate::screens::telegram::host::{Area, background, failure, reply_to_page, without_browser_chrome};

/// A page has little to tell the shell; anything longer is not its message.
const MESSAGE_LIMIT: usize = 4096;

#[derive(Clone, Debug)]
pub enum Signal {
    /// What the page posted, as it posted it.
    Said(Kind, String),
    Loaded(Kind),
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

/// What the page reports, delivered to the screen as messages.
pub fn signals() -> Subscription<Signal> {
    Subscription::run(stream)
}

thread_local! {
    static HOST: RefCell<Option<(Kind, WebView)>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct Spec {
    pub kind: Kind,
    pub api: Api,
    /// Where the web view keeps what the page stores. One folder per account.
    pub dir: PathBuf,
    pub dark: bool,
    pub area: Area,
}

/// Only what describes the body goes on to the server. The session is the
/// shell's to add, and a header the page made up is not honoured.
fn page_headers(request: &wry::http::request::Parts) -> Vec<(String, String)> {
    request
        .headers
        .iter()
        .filter(|(name, _)| name.as_str().eq_ignore_ascii_case("content-type"))
        .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
        .collect()
}

async fn forward(api: Api, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let (parts, body) = request.into_parts();
    let target = parts.uri.path_and_query().map_or_else(|| parts.uri.path().to_owned(), |p| p.as_str().to_owned());
    let headers = page_headers(&parts);
    reply_to_page(api.proxy(parts.method, &target, headers, body).await)
}

fn serve(kind: Kind, api: &Api, runtime: &Handle, request: Request<Vec<u8>>, responder: RequestAsyncResponder) {
    match assets::route(kind, request.method().as_str(), request.uri().path()) {
        assets::Route::Asset { bytes, mime, document } => {
            let mut response = Response::new(Cow::Borrowed(bytes));
            let headers = response.headers_mut();
            if let Ok(value) = mime.parse() {
                headers.insert("content-type", value);
            }
            if document && let Ok(value) = kind.policy().parse() {
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

pub fn create(window: &dyn Window, spec: Spec, runtime: Handle) -> Result<(), String> {
    let kind = spec.kind;
    let origin = kind.origin();
    let mut context = WebContext::new(Some(spec.dir));
    let api = spec.api;
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_id(kind.scheme())
        .with_bounds(spec.area.rect())
        .with_background_color(background(spec.dark))
        .with_visible(false)
        .with_devtools(cfg!(feature = "devtools"))
        .with_hotkeys_zoom(false);
    let builder = without_browser_chrome(builder)
        .with_initialization_script_for_main_only(kind.greeting(spec.dark), true)
        .with_asynchronous_custom_protocol(kind.scheme().into(), move |_, request, responder| serve(kind, &api, &runtime, request, responder))
        .with_ipc_handler(move |request| {
            let raw = request.into_body();
            if raw.len() <= MESSAGE_LIMIT {
                emit(Signal::Said(kind, raw));
            }
        })
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, PageLoadEvent::Finished) {
                emit(Signal::Loaded(kind));
            }
        })
        // The page stays where it was put: it has no links that lead elsewhere.
        .with_navigation_handler(move |url| url.starts_with(&format!("{origin}/")))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_permission_handler(|_| PermissionResponse::Deny)
        .with_url(format!("{origin}/{}", kind.entry()));
    let view = builder.build_as_child(&window).map_err(|error| error.to_string())?;
    HOST.with(|host| *host.borrow_mut() = Some((kind, view)));
    Ok(())
}

fn with_view(kind: Kind, act: impl FnOnce(&WebView)) {
    HOST.with(|host| {
        if let Some((open, view)) = host.borrow().as_ref()
            && *open == kind
        {
            act(view);
        }
    });
}

pub fn run(kind: Kind, script: &str) {
    with_view(kind, |view| {
        if let Err(error) = view.evaluate_script(script) {
            tracing::warn!("hosted page script failed: {error}");
        }
    });
}

pub fn place(kind: Kind, area: Area) {
    with_view(kind, |view| {
        let _ = view.set_bounds(area.rect());
    });
}

pub fn show(kind: Kind, visible: bool) {
    with_view(kind, |view| {
        let _ = view.set_visible(visible);
        let _ = if visible { view.focus() } else { view.focus_parent() };
    });
}

/// Loads the page again from its own files, on the session as it is now.
pub fn reload(kind: Kind) {
    with_view(kind, |view| {
        let _ = view.reload();
    });
}

/// Closes the page. `forget` also empties what the web view kept for it, for
/// an account that signed out or lost the access.
pub fn close(kind: Kind, forget: bool) {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.as_ref().is_some_and(|(open, _)| *open == kind)
            && let Some((_, view)) = host.take()
            && forget
        {
            let _ = view.clear_all_browsing_data();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_cannot_send_credentials_of_its_own_making() {
        let request = Request::builder()
            .method("PUT")
            .uri("tbstime://localhost/api/time/marks/3/2026-11-02")
            .header("Content-Type", "application/json")
            .header("Authorization", "Bearer made-up-by-the-page")
            .header("X-Timelist-User", "somebody-else")
            .header("X-Access-Token", "made-up")
            .header("X-Launch-Session", "made-up")
            .header("X-System-Access", "made-up")
            .header("Cookie", "refresh_token=made-up")
            .body(Vec::<u8>::new())
            .unwrap();
        let (parts, _) = request.into_parts();
        assert_eq!(page_headers(&parts), [("content-type".to_owned(), "application/json".to_owned())]);
    }
}
