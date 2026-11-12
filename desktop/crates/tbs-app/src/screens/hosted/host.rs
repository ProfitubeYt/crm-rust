//! The web view of a hosted workspace page. Like the Telegram one it is a
//! native child of the window and lives on the main thread, out of the screen's
//! state. There is one at a time: a workspace is left before another opens.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use iced::Subscription;
use iced::futures::Stream;
use iced::window::Window;
use tbs_core::api::Api;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use wry::http::{HeaderValue, Request, Response};
use wry::{NewWindowResponse, PageLoadEvent, PermissionResponse, RequestAsyncResponder, WebContext, WebView, WebViewBuilder};

use super::assets::{self, Kind};
use crate::screens::telegram::host::{Area, background, failure, reply_to_page, without_browser_chrome};

/// A page has little to tell the shell; anything longer is not its message.
const MESSAGE_LIMIT: usize = 4096;

/// How often GTK gets its turn on Linux while a page is open; see `pump`.
pub const PUMP_EVERY: std::time::Duration = std::time::Duration::from_millis(30);

/// GTK's turn. On Linux the web view is WebKitGTK, whose toolkit winit neither
/// starts nor runs: `create` starts it and the screen's clock calls this, or
/// the page would never load, draw or hear its answers. Elsewhere the web view
/// needs nobody's help and this does nothing.
pub fn pump() {
    #[cfg(target_os = "linux")]
    if gtk::is_initialized() {
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
    }
}

/// One opening of a page. Its web view and whatever that view reports carry
/// the number, so news from a page that was closed, or opened again, while it
/// was on its way is nobody's.
pub type Owner = u64;

#[derive(Clone, Debug)]
pub enum Signal {
    /// What the page posted, as it posted it.
    Said(Owner, String),
    Loaded(Owner),
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

struct Host {
    /// The opening a view is expected for, or is there for; 0 with no page open.
    owner: Owner,
    view: Option<WebView>,
}

thread_local! {
    static HOST: RefCell<Host> = const { RefCell::new(Host { owner: 0, view: None }) };
}

/// A page is about to be opened: the number its view will carry. A view
/// still there from before is closed.
pub fn claim() -> Owner {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let owner = NEXT.fetch_add(1, Ordering::Relaxed);
    let before = HOST.with(|host| std::mem::replace(&mut *host.borrow_mut(), Host { owner, view: None }));
    release(before.view);
    owner
}

/// Lets a view go. On Linux GTK takes the view's window down in its own time,
/// and if the app's window, its parent, is gone by then, GTK ends the program
/// over the X error; so GTK does it here and now, while the parent stands.
fn release(view: Option<WebView>) {
    if view.is_none() {
        return;
    }
    drop(view);
    #[cfg(target_os = "linux")]
    {
        pump();
        if let Some(display) = gtk::gdk::Display::default() {
            display.sync();
        }
    }
}

#[derive(Clone)]
pub struct Spec {
    pub kind: Kind,
    pub owner: Owner,
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

/// The server's answer is data for the page's scripts. Were it ever shown as
/// a page of its own, it would be shown as the type it declares and run
/// nothing.
fn sealed(mut response: Response<Cow<'static, [u8]>>) -> Response<Cow<'static, [u8]>> {
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'none'; sandbox"));
    response
}

async fn forward(api: Api, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let (parts, body) = request.into_parts();
    let target = parts.uri.path_and_query().map_or_else(|| parts.uri.path().to_owned(), |p| p.as_str().to_owned());
    let headers = page_headers(&parts);
    sealed(reply_to_page(api.proxy(parts.method, &target, headers, body).await))
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
        assets::Route::Forbidden => responder.respond(sealed(failure(403, "Запрос закрыт политикой клиента"))),
        assets::Route::Missing => responder.respond(sealed(failure(404, "Не найдено"))),
    }
}

/// The page has one document. Its address with another query or fragment is
/// still that document; an address of the API is not a page to show.
fn at_home(entry: &str, url: &str) -> bool {
    url.split(['?', '#']).next() == Some(entry)
}

pub fn create(window: &dyn Window, spec: Spec, runtime: Handle) -> Result<(), String> {
    // Closed, or opened again, while the window was being asked for.
    if HOST.with(|host| host.borrow().owner) != spec.owner {
        return Ok(());
    }
    let (kind, owner) = (spec.kind, spec.owner);
    #[cfg(target_os = "linux")]
    gtk::init().map_err(|error| error.to_string())?;
    let entry = format!("{}/{}", kind.origin(), kind.entry());
    let home = entry.clone();
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
                emit(Signal::Said(owner, raw));
            }
        })
        .with_on_page_load_handler(move |event, _| {
            if matches!(event, PageLoadEvent::Finished) {
                emit(Signal::Loaded(owner));
            }
        })
        // The page stays where it was put: it has no links that lead elsewhere.
        .with_navigation_handler(move |url| at_home(&home, &url))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_permission_handler(|_| PermissionResponse::Deny)
        .with_url(entry);
    let view = builder.build_as_child(&window).map_err(|error| error.to_string())?;
    HOST.with(|host| host.borrow_mut().view = Some(view));
    Ok(())
}

fn with_view(owner: Owner, act: impl FnOnce(&WebView)) {
    HOST.with(|host| {
        let host = host.borrow();
        if host.owner == owner
            && let Some(view) = &host.view
        {
            act(view);
        }
    });
}

pub fn run(owner: Owner, script: &str) {
    with_view(owner, |view| {
        if let Err(error) = view.evaluate_script(script) {
            tracing::warn!("hosted page script failed: {error}");
        }
    });
}

pub fn place(owner: Owner, area: Area) {
    with_view(owner, |view| {
        let _ = view.set_bounds(area.rect());
    });
}

pub fn show(owner: Owner, visible: bool) {
    with_view(owner, |view| {
        let _ = view.set_visible(visible);
        let _ = if visible { view.focus() } else { view.focus_parent() };
    });
}

/// Loads the page again from its own files, on the session as it is now.
pub fn reload(owner: Owner) {
    with_view(owner, |view| {
        let _ = view.reload();
    });
}

/// Closes this opening's view, and with it the opening: a view still being
/// made for it is not put up. `forget` also empties what the web view kept,
/// for an account that signed out or lost the access.
pub fn close(owner: Owner, forget: bool) {
    let view = HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.owner != owner {
            return None;
        }
        host.owner = 0;
        host.view.take()
    });
    if let Some(view) = &view
        && forget
        && let Err(error) = view.clear_all_browsing_data()
    {
        tracing::warn!("what a page kept was not emptied: {error}");
    }
    release(view);
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

    #[test]
    fn the_page_is_shown_and_an_address_of_the_api_is_not() {
        for kind in [Kind::Time, Kind::Admin] {
            let entry = format!("{}/{}", kind.origin(), kind.entry());
            assert!(at_home(&entry, &entry));
            assert!(at_home(&entry, &format!("{entry}#2026-11")) && at_home(&entry, &format!("{entry}?v=2")));
            let origin = kind.origin();
            for elsewhere in [
                format!("{origin}/api/time/export/2026-11"),
                format!("{origin}/api/admin/users"),
                format!("{origin}/"),
                format!("{entry}/../api/auth/me"),
                format!("{entry}x"),
                "https://example.com/".to_owned(),
                "about:blank".to_owned(),
            ] {
                assert!(!at_home(&entry, &elsewhere), "{elsewhere}");
            }
        }
    }

    #[test]
    fn an_answer_of_the_server_is_never_a_page_that_runs() {
        let response = sealed(failure(403, "x"));
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(response.headers()["content-security-policy"], "default-src 'none'; sandbox");
    }

    #[test]
    fn a_view_belongs_to_one_opening() {
        let first = claim();
        let second = claim();
        assert_ne!(first, second);
        assert_eq!(HOST.with(|host| host.borrow().owner), second);
        // The first opening was replaced: closing it late leaves the second alone.
        close(first, true);
        assert_eq!(HOST.with(|host| host.borrow().owner), second);
        close(second, false);
        assert_eq!(HOST.with(|host| host.borrow().owner), 0);
    }
}
