//! The web view of a Telegram mini app. A mini app is somebody else's page, so
//! it is the one thing of the section that is not drawn by the client itself.
//! A native window child lives on the main thread, so it stays out of the
//! screen's state; everything here is called from there.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use iced::Subscription;
use iced::futures::Stream;
use iced::window::Window;
use tbs_core::links;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{NewWindowResponse, PageLoadEvent, PermissionResponse, Rect, WebContext, WebView, WebViewBuilder};

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
    /// What the app posted to its host, as it posted it.
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

/// What the web view reports, delivered to the screen as messages.
pub fn signals() -> Subscription<Signal> {
    Subscription::run(stream)
}

/// A rectangle in logical pixels, relative to the window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Area {
    pub(crate) fn rect(self) -> Rect {
        Rect {
            position: LogicalPosition::new(f64::from(self.x), f64::from(self.y)).into(),
            size: LogicalSize::new(f64::from(self.width.max(1.0)), f64::from(self.height.max(1.0))).into(),
        }
    }
}

thread_local! {
    static MINI: RefCell<Option<WebView>> = const { RefCell::new(None) };
}

pub(crate) fn background(dark: bool) -> (u8, u8, u8, u8) {
    if dark { (16, 16, 16, 255) } else { (255, 255, 255, 255) }
}

#[cfg(all(windows, feature = "devtools"))]
fn debug_port() -> Option<String> {
    let port: u16 = std::env::var("TBS_WEB_DEBUG").ok()?.parse().ok()?;
    Some(format!("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={}", port + 1))
}

/// The browser's own shortcuts (F5, Ctrl+F…) and its context menu are WebView2
/// switches. The other platforms have none, and the program is only built
/// there to run its tests.
#[cfg(windows)]
pub(crate) fn without_browser_chrome(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    use wry::WebViewBuilderExtWindows;
    builder.with_browser_accelerator_keys(false).with_default_context_menus(false)
}

#[cfg(not(windows))]
pub(crate) fn without_browser_chrome(builder: WebViewBuilder<'_>) -> WebViewBuilder<'_> {
    builder
}

pub fn create_mini(window: &dyn Window, url: &str, area: Area, dir: PathBuf, dark: bool) -> Result<(), String> {
    if !links::is_safe_mini_app_url(url) {
        return Err("Адрес мини-приложения отклонён".to_owned());
    }
    #[cfg(target_os = "linux")]
    gtk::init().map_err(|error| error.to_string())?;
    let mut context = WebContext::new(Some(dir.join("webview-miniapp")));
    let pinned: Arc<Mutex<Option<String>>> = Arc::default();
    let (on_load, on_navigate) = (pinned.clone(), pinned);
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_id("miniapp")
        .with_bounds(area.rect())
        .with_background_color(background(dark))
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
    let builder = match debug_port() {
        Some(args) => {
            use wry::WebViewBuilderExtWindows;
            builder.with_additional_browser_args(args).with_devtools(true)
        }
        None => builder,
    };
    let view = builder.build_as_child(&window).map_err(|error| error.to_string())?;
    MINI.with(|mini| *mini.borrow_mut() = Some(view));
    Ok(())
}

fn with_mini(act: impl FnOnce(&WebView)) {
    MINI.with(|mini| {
        if let Some(view) = mini.borrow().as_ref() {
            act(view);
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

pub fn place_mini(area: Area) {
    with_mini(|view| {
        let _ = view.set_bounds(area.rect());
    });
}

/// The keyboard stays with the window until the app is pressed on, so Escape
/// still closes it; a view that is hidden or gone hands the keyboard back.
pub fn show_mini(visible: bool) {
    with_mini(|view| {
        let _ = view.set_visible(visible);
        if !visible {
            let _ = view.focus_parent();
        }
    });
}

pub fn close_mini() {
    MINI.with(|mini| {
        if let Some(view) = mini.borrow_mut().take() {
            let _ = view.focus_parent();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_never_collapses_to_nothing() {
        let rect = Area { x: 208.0, y: 0.0, width: -5.0, height: 0.0 }.rect();
        let size = rect.size.to_logical::<f64>(1.0);
        assert_eq!((size.width, size.height), (1.0, 1.0));
    }
}
