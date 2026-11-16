//! A Telegram mini app from the press on its button to its closing: the
//! launch the server arranges, the web view that holds the app's page, and
//! the frame the client draws around it.

use iced::widget::{Space, Stack, button, center, column, container, mouse_area, opaque, pin, row, svg, text};
use iced::{Alignment, Fill, Size, Task, window};
use serde_json::{Value, json};
use tbs_core::api::TgWebApp;
use tbs_core::links;

use super::bridge::{self, Bridge, ButtonKind, Effect, Popup};
use super::client::AppRequest;
use super::host::{self, Area, Signal};
use super::style;
use crate::ctx::{Ctx, text_error};
use crate::theme::{self, control, size, space};
use crate::ui::{self, El, Icon, Kind};

const WIDTH: f32 = 560.0;
const HEIGHT: f32 = 860.0;
const MARGIN: f32 = 16.0;
const HEADER: f32 = 48.0;
const FOOTER: f32 = 56.0;
const NO_APP: &str = "Не удалось открыть мини-приложение";

/// Where the frame stands in the window, and the room it leaves the app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub panel: Area,
    pub body: Area,
}

/// Centred in what the rail leaves. The web view floats above everything the
/// client draws, so its room is worked out here and not left to the layout.
pub fn frame(ctx: &Ctx, footer: bool) -> Frame {
    let room = (ctx.window.width - ctx.rail).max(0.0);
    let width = WIDTH.min(room - 2.0 * MARGIN).max(240.0).floor();
    let height = HEIGHT.min(ctx.window.height - 2.0 * MARGIN).max(240.0).floor();
    let x = (ctx.rail + (room - width) / 2.0).floor().max(0.0);
    let y = ((ctx.window.height - height) / 2.0).floor().max(0.0);
    let below = if footer { FOOTER + 1.0 } else { 0.0 };
    // One pixel of border all round, and one of the line under the header.
    let body = Area { x: x + 1.0, y: y + HEADER + 2.0, width: width - 2.0, height: (height - HEADER - 3.0 - below).max(1.0) };
    Frame { panel: Area { x, y, width, height }, body }
}

struct Launch {
    token: u64,
    request: AppRequest,
    /// The address Telegram issued; `None` while the server is asked for it.
    url: Option<String>,
    launch_id: Option<String>,
    error: Option<String>,
    attached: bool,
    loaded: bool,
    bridge: Bridge,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Resolved(u64, i64, Result<TgWebApp, String>),
    Attached(u64, Result<(), String>),
    Signal(Signal),
    Close,
    Back,
    Main,
    Popup(Option<String>),
    Clipboard(u64, String, u64, Option<String>),
    DataSent(u64),
}

#[derive(Default)]
pub struct MiniApp {
    /// Moves with every opening and closing: an answer for another launch is dropped.
    token: u64,
    current: Option<Launch>,
    /// Whether the web view was last told to show itself.
    showing: bool,
    /// Links the app asked to open; the section's link policy takes them.
    pub links: Vec<String>,
}

impl MiniApp {
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    fn forget(api: tbs_core::api::Api, chat: i64, launch_id: Option<String>) -> Task<Msg> {
        match launch_id {
            // The server keeps a launch until told; a failed telling must not hold up the closing.
            Some(id) => Task::future(async move {
                let _ = api.tg_close_web_app(chat, &id).await;
            })
            .discard(),
            None => Task::none(),
        }
    }

    pub fn open(&mut self, request: AppRequest, ctx: &Ctx) -> Task<Msg> {
        let before = self.close(ctx);
        self.token += 1;
        let token = self.token;
        let (api, chat, url, bot, source) = (ctx.api.clone(), request.chat, request.url.clone(), request.bot, request.source);
        self.current = Some(Launch { token, bridge: Bridge::new(&request.url), request, url: None, launch_id: None, error: None, attached: false, loaded: false });
        let ask = Task::perform(async move { api.tg_open_web_app(chat, &url, bot, source).await.map_err(|e| text_error(&e)) }, move |outcome| Msg::Resolved(token, chat, outcome));
        Task::batch([before, ask])
    }

    pub fn close(&mut self, ctx: &Ctx) -> Task<Msg> {
        self.token += 1;
        self.showing = false;
        host::close_mini();
        match self.current.take() {
            Some(launch) => Self::forget(ctx.api.clone(), launch.request.chat, launch.launch_id),
            None => Task::none(),
        }
    }

    fn room(&self, ctx: &Ctx) -> Size {
        let body = frame(ctx, self.current.as_ref().is_some_and(|launch| launch.bridge.footer())).body;
        Size::new(body.width, body.height)
    }

    fn tell(&self, name: &str, data: &Value) {
        if self.current.as_ref().is_some_and(|launch| launch.attached) {
            host::run_in_mini(&bridge::deliver_script(name, data));
        }
    }

    fn run(&mut self, effects: Vec<Effect>, ctx: &Ctx) -> Task<Msg> {
        let mut tasks = Vec::new();
        for effect in effects {
            match effect {
                Effect::Send(name, data) => self.tell(name, &data),
                Effect::Link(url) => self.links.push(url),
                Effect::Close => tasks.push(self.close(ctx)),
                Effect::SendData(data) => {
                    let Some(launch) = &self.current else { continue };
                    let (api, token, chat, bot, caption) = (ctx.api.clone(), launch.token, launch.request.chat, launch.request.bot, launch.request.text.clone());
                    // The app is closed once its data has gone, whether it arrived or not.
                    tasks.push(Task::perform(
                        async move {
                            let _ = api.tg_send_web_app_data(chat, bot, &caption, &data).await;
                        },
                        move |()| Msg::DataSent(token),
                    ));
                }
                Effect::ReadClipboard { req_id, epoch } => {
                    let token = self.token;
                    tasks.push(iced::clipboard::read().map(move |text| Msg::Clipboard(token, req_id.clone(), epoch, text)));
                }
            }
        }
        Task::batch(tasks)
    }

    /// The web view keeps to the room the frame leaves it.
    pub fn place(&self, ctx: &Ctx) {
        if let Some(launch) = self.current.as_ref().filter(|launch| launch.attached) {
            host::place_mini(frame(ctx, launch.bridge.footer()).body);
        }
    }

    /// Shown while the section is, once its page has loaded and nothing of
    /// the client's has to be drawn over it.
    pub fn show(&mut self, section_shown: bool) {
        let wanted = section_shown && self.current.as_ref().is_some_and(|launch| launch.attached && launch.loaded && launch.error.is_none() && launch.bridge.popup.is_none());
        if wanted != self.showing {
            self.showing = wanted;
            host::show_mini(wanted);
        }
    }

    pub fn theme_changed(&self, ctx: &Ctx) {
        self.tell("theme_changed", &json!({ "theme_params": bridge::theme_params(ctx.dark) }));
    }

    pub fn resized(&self, ctx: &Ctx) {
        self.place(ctx);
        self.tell("viewport_changed", &bridge::viewport(self.room(ctx)));
    }

    /// Escape puts a popup away first, and closes the app only after.
    pub fn escape(&mut self, ctx: &Ctx) -> Task<Msg> {
        let Some(launch) = &mut self.current else { return Task::none() };
        let effects = if launch.bridge.popup.is_some() { launch.bridge.resolve_popup(None) } else { launch.bridge.request_close() };
        self.run(effects, ctx)
    }

    pub fn update(&mut self, message: Msg, ctx: &mut Ctx) -> Task<Msg> {
        match message {
            Msg::Resolved(token, chat, outcome) => {
                let Some(launch) = self.current.as_mut().filter(|launch| launch.token == token) else {
                    // Closed or replaced while the server was answering: the launch it made is let go.
                    return match outcome {
                        Ok(app) => Self::forget(ctx.api.clone(), chat, app.launch_id),
                        Err(_) => Task::none(),
                    };
                };
                match outcome {
                    Ok(app) if links::is_safe_mini_app_url(&app.url) => {
                        launch.launch_id = app.launch_id;
                        launch.bridge = Bridge::new(&app.url);
                        launch.url = Some(app.url.clone());
                        let (area, dir, dark) = (frame(ctx, false).body, ctx.data_dir.clone(), ctx.dark);
                        return window::oldest()
                            .then(move |id| {
                                let (url, dir) = (app.url.clone(), dir.clone());
                                match id {
                                    Some(id) => window::run(id, move |w| host::create_mini(w, &url, area, dir, dark)),
                                    None => Task::done(Err("Окно приложения недоступно".to_owned())),
                                }
                            })
                            .map(move |outcome| Msg::Attached(token, outcome));
                    }
                    Ok(app) => {
                        launch.launch_id = app.launch_id;
                        launch.error = Some("Адрес мини-приложения отклонён".to_owned());
                        ctx.toast(theme::Tone::Danger, "Адрес мини-приложения отклонён");
                    }
                    Err(reason) => launch.error = Some(if reason.is_empty() { NO_APP.to_owned() } else { reason }),
                }
            }
            Msg::Attached(token, outcome) => match (self.current.as_mut().filter(|launch| launch.token == token), outcome) {
                (Some(launch), Ok(())) => launch.attached = true,
                (Some(launch), Err(reason)) => {
                    tracing::warn!("mini app failed: {reason}");
                    launch.error = Some(NO_APP.to_owned());
                    ctx.toast(theme::Tone::Danger, NO_APP);
                }
                (None, _) => {
                    // A view made for a launch that is gone; one made for the current launch stays.
                    if !self.current.as_ref().is_some_and(|launch| launch.attached) {
                        host::close_mini();
                    }
                }
            },
            Msg::Signal(signal) => {
                let room = self.room(ctx);
                let Some(launch) = self.current.as_mut().filter(|launch| launch.url.is_some() && launch.error.is_none()) else { return Task::none() };
                let effects = match signal {
                    Signal::Mini(raw) => match bridge::read_event(&raw) {
                        Some((name, data)) => launch.bridge.on_event(&name, &data, room, ctx.dark),
                        None => Vec::new(),
                    },
                    Signal::MiniStarted => launch.bridge.navigated(),
                    Signal::MiniFinished => {
                        launch.loaded = true;
                        launch.bridge.loaded();
                        Vec::new()
                    }
                    Signal::External(url) => vec![Effect::Link(url)],
                };
                return self.run(effects, ctx);
            }
            Msg::Close => {
                let Some(launch) = &mut self.current else { return Task::none() };
                // An app that never came up has nothing to lose by closing.
                let effects = if launch.error.is_some() || launch.url.is_none() { vec![Effect::Close] } else { launch.bridge.request_close() };
                return self.run(effects, ctx);
            }
            Msg::Back => {
                let effects = self.current.as_ref().map(|launch| launch.bridge.press_back()).unwrap_or_default();
                return self.run(effects, ctx);
            }
            Msg::Main => {
                let effects = self.current.as_ref().map(|launch| launch.bridge.press_main()).unwrap_or_default();
                return self.run(effects, ctx);
            }
            Msg::Popup(button) => {
                let Some(launch) = &mut self.current else { return Task::none() };
                let effects = launch.bridge.resolve_popup(button.as_deref());
                return self.run(effects, ctx);
            }
            Msg::Clipboard(token, req_id, epoch, text) => {
                if let Some(launch) = self.current.as_ref().filter(|_| token == self.token) {
                    let answer = launch.bridge.clipboard_read(&req_id, epoch, text);
                    return self.run(vec![answer], ctx);
                }
            }
            Msg::DataSent(token) => {
                if self.current.as_ref().is_some_and(|launch| launch.token == token) {
                    return self.close(ctx);
                }
            }
        }
        Task::none()
    }

    fn popup<'a>(popup: &'a Popup) -> El<'a, Msg> {
        let mut body = column![].spacing(space::SM);
        if let Some(title) = &popup.title {
            body = body.push(ui::strong(title.as_str()));
        }
        body = body.push(text(popup.message.as_str()).size(size::BASE));
        let buttons: Vec<El<'a, Msg>> = popup
            .buttons
            .iter()
            .map(|choice| {
                let kind = match choice.kind {
                    ButtonKind::Destructive => Kind::Danger,
                    ButtonKind::Cancel => Kind::Ghost,
                    ButtonKind::Ok | ButtonKind::Default => Kind::Primary,
                };
                ui::btn_small(choice.text.as_str(), kind, Some(Msg::Popup(Some(choice.id.clone())))).into()
            })
            .collect();
        body = body.push(container(ui::chip_row(buttons)).align_right(Fill));
        ui::modal(Space::new(), body, 360.0, Some(Msg::Popup(None)))
    }

    /// The frame over the section while an app is open: its name and its
    /// buttons above, its own main button below, and between them the room
    /// the web view covers once the page has loaded.
    pub fn view(&self, ctx: &Ctx) -> Option<El<'_, Msg>> {
        let launch = self.current.as_ref()?;
        let frame = frame(ctx, launch.bridge.footer());
        let title = launch.url.as_deref().map_or_else(|| bridge::title_of(&launch.request.url), bridge::title_of);
        let back: El<'_, Msg> = if launch.bridge.back { ui::icon_btn(Icon::ChevronLeft, "Назад", Some(Msg::Back)) } else { Space::new().width(control::MD).into() };
        let name = container(text(title).size(size::BASE).font(theme::FONT_BOLD).wrapping(text::Wrapping::None)).width(Fill).clip(true);
        let head = container(row![back, name, ui::icon_btn(Icon::Close, "Закрыть", Some(Msg::Close))].spacing(space::SM).align_y(Alignment::Center))
            .height(HEADER)
            .center_y(HEADER)
            .padding([0.0, space::SM])
            .width(Fill)
            .style(style::bar);
        let inside: El<'_, Msg> = match &launch.error {
            Some(reason) => center(column![ui::muted(reason.as_str()).center(), ui::btn("Закрыть", Kind::Secondary, Some(Msg::Close))].spacing(space::MD).align_x(Alignment::Center)).padding(space::XL).into(),
            None => center(ui::icon(Icon::Loader, 22.0)).into(),
        };
        let mut panel = column![head, ui::divider(), container(inside).width(Fill).height(frame.body.height).style(style::app_body(ctx.dark))];
        if let Some(main) = launch.bridge.main.as_ref().filter(|main| main.visible) {
            let ink = main.text_color;
            let mut words = row![].spacing(space::XS).align_y(Alignment::Center);
            if main.progress {
                words = words.push(ui::icon(Icon::Loader, 14.0).style(move |theme: &iced::Theme, _| svg::Style { color: Some(ink.unwrap_or(theme::p(theme).on_accent)) }));
            }
            words = words.push(text(if main.text.is_empty() { "Продолжить" } else { main.text.as_str() }).size(size::BASE).font(theme::FONT_MEDIUM));
            let press = button(container(words).center_x(Fill).center_y(Fill)).width(Fill).height(Fill).padding(0).style(style::app_button(main.color, ink)).on_press_maybe(main.active.then_some(Msg::Main));
            panel = panel.push(ui::divider()).push(container(press).padding(10).width(Fill).height(FOOTER).style(style::bar));
        }
        let framed = container(panel).padding(1).width(frame.panel.width).height(frame.panel.height).style(style::app_panel);
        let scrim = mouse_area(container(Space::new()).width(Fill).height(Fill).style(theme::scrim)).on_press(Msg::Close);
        let mut layers: Vec<El<'_, Msg>> = vec![opaque(scrim), pin(opaque(framed)).x(frame.panel.x).y(frame.panel.y).into()];
        if let Some(popup) = &launch.bridge.popup {
            layers.push(Self::popup(popup));
        }
        Some(Stack::with_children(layers).width(Fill).height(Fill).into())
    }
}

#[cfg(test)]
mod tests {
    use tbs_core::api::TgButtonSource;

    use super::*;

    fn ctx(dir: &tempfile::TempDir) -> Ctx {
        Ctx::for_tests(dir.path())
    }

    fn request(url: &str) -> AppRequest {
        AppRequest { chat: 5, bot: Some(77), url: url.into(), source: TgButtonSource::Inline, text: "Открыть".into() }
    }

    fn app(url: &str) -> Result<TgWebApp, String> {
        Ok(TgWebApp { url: url.into(), launch_id: Some("900".into()) })
    }

    /// An app whose page is up, as far as the screen knows.
    fn running(ctx: &mut Ctx) -> MiniApp {
        let mut mini = MiniApp::default();
        let _ = mini.open(request("https://app.example.com/"), ctx);
        let token = mini.token;
        let _ = mini.update(Msg::Resolved(token, 5, app("https://app.example.com/start")), ctx);
        let _ = mini.update(Msg::Attached(token, Ok(())), ctx);
        mini
    }

    fn event(name: &str, data: Value) -> Msg {
        Msg::Signal(Signal::Mini(json!({ "eventType": name, "eventData": data.to_string() }).to_string()))
    }

    #[test]
    fn the_app_gets_the_room_the_frame_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        ctx.window = Size::new(1360.0, 860.0);
        ctx.rail = 208.0;
        let plain = frame(&ctx, false);
        assert_eq!(plain.panel, Area { x: 504.0, y: 16.0, width: 560.0, height: 828.0 });
        assert_eq!(plain.body, Area { x: 505.0, y: 66.0, width: 558.0, height: 777.0 });
        // The app's main button takes its strip from the bottom.
        assert_eq!(frame(&ctx, true).body.height, 777.0 - FOOTER - 1.0);
        // A small window still leaves the frame inside it.
        ctx.window = Size::new(700.0, 400.0);
        let small = frame(&ctx, false);
        assert_eq!((small.panel.width, small.panel.height), (460.0, 368.0));
        assert!(small.panel.x >= ctx.rail && small.panel.x + small.panel.width <= 700.0);
    }

    #[test]
    fn an_app_opens_on_the_address_telegram_issued() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = MiniApp::default();
        let _ = mini.open(request("https://app.example.com/"), &ctx);
        assert!(mini.is_open());
        let launch = mini.current.as_ref().unwrap();
        assert!(launch.url.is_none() && launch.bridge.loading);
        let token = mini.token;
        let _ = mini.update(Msg::Resolved(token, 5, app("https://app.example.com/start?tgWebAppData=x")), &mut ctx);
        let launch = mini.current.as_ref().unwrap();
        assert_eq!(launch.url.as_deref(), Some("https://app.example.com/start?tgWebAppData=x"));
        assert_eq!(launch.launch_id.as_deref(), Some("900"));
        assert!(!launch.attached);
        let _ = mini.update(Msg::Attached(token, Ok(())), &mut ctx);
        assert!(mini.current.as_ref().unwrap().attached);
    }

    #[test]
    fn unsafe_mini_apps_never_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = MiniApp::default();
        let _ = mini.open(request("https://app.example.com/"), &ctx);
        let token = mini.token;
        let _ = mini.update(Msg::Resolved(token, 5, app("http://evil.example.com/")), &mut ctx);
        let launch = mini.current.as_ref().unwrap();
        assert!(launch.url.is_none() && launch.error.is_some());
        assert_eq!(ctx.toasts.len(), 1);
        // Nothing of an app that was refused is listened to.
        let _ = mini.update(event("web_app_open_link", json!({ "url": "https://t.me/x" })), &mut ctx);
        assert!(mini.links.is_empty());
        let _ = mini.update(Msg::Close, &mut ctx);
        assert!(!mini.is_open());
    }

    #[test]
    fn a_launch_that_failed_says_so_and_closes() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = MiniApp::default();
        let _ = mini.open(request("https://app.example.com/"), &ctx);
        let token = mini.token;
        let _ = mini.update(Msg::Resolved(token, 5, Err("bot_forbidden".into())), &mut ctx);
        assert_eq!(mini.current.as_ref().unwrap().error.as_deref(), Some("bot_forbidden"));
        let _ = mini.update(Msg::Close, &mut ctx);
        assert!(!mini.is_open());
    }

    #[test]
    fn a_stale_mini_app_does_not_take_over_the_current_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = MiniApp::default();
        let _ = mini.open(request("https://one.example.com/"), &ctx);
        let first = mini.token;
        let _ = mini.open(request("https://two.example.com/"), &ctx);
        let second = mini.token;
        assert_ne!(first, second);
        // The first launch answers late: it neither opens nor is attached.
        let _ = mini.update(Msg::Resolved(first, 5, app("https://one.example.com/start")), &mut ctx);
        assert!(mini.current.as_ref().unwrap().url.is_none());
        let _ = mini.update(Msg::Attached(first, Ok(())), &mut ctx);
        assert!(!mini.current.as_ref().unwrap().attached);
        let _ = mini.update(Msg::Resolved(second, 5, app("https://two.example.com/start")), &mut ctx);
        let _ = mini.update(Msg::Attached(second, Ok(())), &mut ctx);
        let launch = mini.current.as_ref().unwrap();
        assert!(launch.attached && launch.url.as_deref() == Some("https://two.example.com/start"));
        // An answer for a launch that was closed leaves nothing open.
        let _ = mini.update(Msg::Close, &mut ctx);
        let _ = mini.update(Msg::Resolved(second, 5, app("https://two.example.com/start")), &mut ctx);
        assert!(!mini.is_open());
    }

    #[test]
    fn what_the_app_asks_reaches_the_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = running(&mut ctx);
        assert!(mini.current.as_ref().unwrap().bridge.loading);
        let _ = mini.update(event("web_app_ready", json!({})), &mut ctx);
        assert!(!mini.current.as_ref().unwrap().bridge.loading);
        let _ = mini.update(event("web_app_setup_main_button", json!({ "is_visible": true, "text": "Оплатить" })), &mut ctx);
        let _ = mini.update(event("web_app_setup_back_button", json!({ "is_visible": true })), &mut ctx);
        let bridge = &mini.current.as_ref().unwrap().bridge;
        assert!(bridge.footer() && bridge.back);
        assert_eq!(bridge.main.as_ref().unwrap().text, "Оплатить");
        // A message that is not an event of the protocol changes nothing.
        let _ = mini.update(Msg::Signal(Signal::Mini("{\"run\":\"anything\"}".into())), &mut ctx);
        assert!(mini.is_open());
    }

    #[test]
    fn links_of_the_app_are_left_to_the_link_policy() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = running(&mut ctx);
        let _ = mini.update(event("web_app_open_tg_link", json!({ "path_full": "/channel" })), &mut ctx);
        let _ = mini.update(event("web_app_open_link", json!({ "url": "https://evil.example.com/" })), &mut ctx);
        let _ = mini.update(Msg::Signal(Signal::External("https://t.me/other".into())), &mut ctx);
        assert_eq!(mini.links, ["https://t.me/channel", "https://evil.example.com/", "https://t.me/other"]);
    }

    #[test]
    fn the_web_view_is_shown_only_when_nothing_has_to_cover_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = running(&mut ctx);
        mini.show(true);
        assert!(!mini.showing, "a page that has not loaded shows the loader instead");
        let _ = mini.update(Msg::Signal(Signal::MiniFinished), &mut ctx);
        mini.show(true);
        assert!(mini.showing);
        // The client's popup would land under the web view.
        let _ = mini.update(event("web_app_open_popup", json!({ "message": "Сохранить?" })), &mut ctx);
        mini.show(true);
        assert!(!mini.showing);
        let _ = mini.update(Msg::Popup(Some("ok".into())), &mut ctx);
        mini.show(true);
        assert!(mini.showing);
        // Another section is on screen.
        mini.show(false);
        assert!(!mini.showing);
    }

    #[test]
    fn closing_asks_first_when_the_app_wants_and_escape_takes_the_popup_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = running(&mut ctx);
        let _ = mini.update(event("web_app_setup_closing_behavior", json!({ "need_confirmation": true })), &mut ctx);
        let _ = mini.update(Msg::Close, &mut ctx);
        assert!(mini.is_open() && mini.current.as_ref().unwrap().bridge.popup.is_some());
        let _ = mini.escape(&ctx);
        assert!(mini.is_open() && mini.current.as_ref().unwrap().bridge.popup.is_none());
        let _ = mini.update(Msg::Close, &mut ctx);
        let _ = mini.update(Msg::Popup(Some("__confirm_close_yes".into())), &mut ctx);
        assert!(!mini.is_open());
        // Without the app's wish Escape closes at once.
        let mut mini = running(&mut ctx);
        let _ = mini.escape(&ctx);
        assert!(!mini.is_open());
    }

    #[test]
    fn an_app_that_sent_its_data_is_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = running(&mut ctx);
        let token = mini.token;
        let _ = mini.update(event("web_app_data_send", json!({ "data": "{}" })), &mut ctx);
        assert!(mini.is_open(), "it stays until the data has gone");
        let _ = mini.update(Msg::DataSent(token + 1), &mut ctx);
        assert!(mini.is_open());
        let _ = mini.update(Msg::DataSent(token), &mut ctx);
        assert!(!mini.is_open());
    }

    #[test]
    fn the_frame_draws_in_every_state() {
        use iced_test::Simulator;
        use iced_test::simulator::Settings as SimSettings;
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = ctx(&dir);
        let mut mini = MiniApp::default();
        assert!(mini.view(&ctx).is_none());
        let _ = mini.open(request("https://www.app.example.com/"), &ctx);
        let token = mini.token;
        let sees = |mini: &MiniApp, ctx: &Ctx, wanted: &str| Simulator::with_size(SimSettings::default(), Size::new(1360.0, 860.0), mini.view(ctx).unwrap()).find(wanted).is_ok();
        assert!(sees(&mini, &ctx, "app.example.com"));
        let _ = mini.update(Msg::Resolved(token, 5, Err("Бот недоступен".into())), &mut ctx);
        assert!(sees(&mini, &ctx, "Бот недоступен") && sees(&mini, &ctx, "Закрыть"));
        let mut mini = running(&mut ctx);
        let _ = mini.update(event("web_app_setup_main_button", json!({ "is_visible": true, "text": "Оплатить" })), &mut ctx);
        let _ = mini.update(event("web_app_open_popup", json!({ "title": "Удалить?", "message": "Это навсегда", "buttons": [{ "id": "y", "type": "destructive", "text": "Удалить" }] })), &mut ctx);
        for wanted in ["Оплатить", "Удалить?", "Это навсегда", "Удалить"] {
            assert!(sees(&mini, &ctx, wanted), "{wanted}");
        }
    }
}
