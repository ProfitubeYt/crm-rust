//! Telegram's WebApp protocol, spoken for a mini app: what the app asks of its
//! host and what the host tells it back. Nothing here touches a window or the
//! network; it says what should happen and the screen does it.

use iced::{Color, Size};
use serde_json::{Value, json};
use tbs_core::links;

use crate::theme::{DARK, LIGHT};

const CONFIRM_CLOSE_YES: &str = "__confirm_close_yes";
const CONFIRM_CLOSE_NO: &str = "__confirm_close_no";
const CLIPBOARD_ALLOW: &str = "__clipboard_allow";
const CLIPBOARD_DENY: &str = "__clipboard_deny";

#[derive(Clone, Debug, PartialEq)]
pub struct MainButton {
    pub visible: bool,
    pub active: bool,
    pub text: String,
    pub color: Option<Color>,
    pub text_color: Option<Color>,
    pub progress: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Default,
    Ok,
    Cancel,
    Destructive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupButton {
    pub id: String,
    pub kind: ButtonKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popup {
    pub title: Option<String>,
    pub message: String,
    pub buttons: Vec<PopupButton>,
}

/// What the screen is asked to do for the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// An event for the app: its name and its data.
    Send(&'static str, Value),
    /// A link the app wants opened outside; the link policy decides.
    Link(String),
    Close,
    /// Data for the bot the app was opened from; the app closes after it.
    SendData(String),
    /// The person allowed one reading of the clipboard.
    ReadClipboard { req_id: String, epoch: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Consent {
    req_id: String,
    epoch: u64,
}

pub struct Bridge {
    pub loading: bool,
    pub main: Option<MainButton>,
    pub back: bool,
    pub popup: Option<Popup>,
    host: String,
    confirm_close: bool,
    consent: Option<Consent>,
    /// Moves with every navigation, so an answer asked for by one page is
    /// never given to the next.
    epoch: u64,
}

fn hex(color: Color) -> String {
    let [r, g, b, _] = color.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn color_of(raw: &Value) -> Option<Color> {
    let digits = raw.as_str()?.strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let part = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some(Color::from_rgb8(part(0)?, part(2)?, part(4)?))
}

/// The colours a mini app paints itself in, taken from the app's own theme.
pub fn theme_params(dark: bool) -> Value {
    let c = if dark { &DARK } else { &LIGHT };
    json!({
        "bg_color": hex(c.surface),
        "secondary_bg_color": hex(c.raised),
        "text_color": hex(c.text),
        "hint_color": hex(c.text3),
        "link_color": hex(c.accent),
        "button_color": hex(c.accent),
        "button_text_color": hex(c.on_accent),
        "header_bg_color": hex(c.surface),
        "accent_text_color": hex(c.accent),
        "section_bg_color": hex(c.surface),
        "section_header_text_color": hex(c.text2),
        "subtitle_text_color": hex(c.text2),
        "destructive_text_color": hex(c.danger),
    })
}

pub fn viewport(view: Size) -> Value {
    json!({ "width": view.width.round(), "height": view.height.round(), "is_expanded": true, "is_state_stable": true })
}

fn no_insets() -> Value {
    json!({ "top": 0, "bottom": 0, "left": 0, "right": 0 })
}

pub fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

/// The script that hands one event to the app's own Telegram library.
pub fn deliver_script(event_type: &str, event_data: &Value) -> String {
    format!(
        "try{{window.Telegram&&window.Telegram.WebView&&window.Telegram.WebView.receiveEvent({},{})}}catch(e){{}}",
        js_string(event_type),
        serde_json::to_string(event_data).unwrap_or_else(|_| "null".to_owned()),
    )
}

/// The app's library sends its data as JSON text; a page of its own may send
/// an object. Anything else is nothing.
fn event_data(raw: &Value) -> Value {
    match raw {
        Value::Object(_) => raw.clone(),
        Value::String(text) => serde_json::from_str::<Value>(text.trim()).ok().filter(Value::is_object).unwrap_or_else(|| json!({})),
        _ => json!({}),
    }
}

/// What the web view posts: `{ eventType, eventData }`.
pub fn read_event(raw: &str) -> Option<(String, Value)> {
    let parsed: Value = serde_json::from_str(raw).ok()?;
    let name = parsed["eventType"].as_str()?.to_owned();
    Some((name, event_data(&parsed["eventData"])))
}

/// The name over a mini app: its host, without the `www.`.
pub fn title_of(url: &str) -> String {
    links::origin_of(url)
        .and_then(|origin| origin.split_once("://").map(|(_, host)| host.trim_start_matches("www.").to_owned()))
        .unwrap_or_else(|| "Мини-приложение".to_owned())
}

fn popup_button(raw: &Value, at: usize) -> PopupButton {
    let kind = match raw["type"].as_str() {
        Some("ok") => ButtonKind::Ok,
        Some("cancel" | "close") => ButtonKind::Cancel,
        Some("destructive") => ButtonKind::Destructive,
        _ => ButtonKind::Default,
    };
    let fallback = if kind == ButtonKind::Ok { "OK" } else { "Закрыть" };
    PopupButton {
        id: raw["id"].as_str().map_or_else(|| format!("button-{at}"), str::to_owned),
        kind,
        text: raw["text"].as_str().filter(|text| !text.is_empty()).unwrap_or(fallback).to_owned(),
    }
}

impl Bridge {
    pub fn new(url: &str) -> Self {
        Self { loading: true, main: None, back: false, popup: None, host: title_of(url), confirm_close: false, consent: None, epoch: 0 }
    }

    fn refuse_clipboard(&mut self) -> Option<Effect> {
        let consent = self.consent.take()?;
        Some(Effect::Send("clipboard_text_received", json!({ "req_id": consent.req_id, "data": "" })))
    }

    /// One event from the app. `view` is the room the app has, `dark` the theme.
    pub fn on_event(&mut self, name: &str, data: &Value, view: Size, dark: bool) -> Vec<Effect> {
        let theme = || Effect::Send("theme_changed", json!({ "theme_params": theme_params(dark) }));
        let room = || Effect::Send("viewport_changed", viewport(view));
        match name {
            "web_app_ready" | "iframe_ready" => {
                self.loading = false;
                // Primed with its theme and its room, so it lays out right the first time.
                vec![theme(), room(), Effect::Send("safe_area_changed", no_insets()), Effect::Send("content_safe_area_changed", no_insets())]
            }
            "web_app_request_theme" => vec![theme()],
            "web_app_request_viewport" | "web_app_expand" => vec![room()],
            "web_app_request_safe_area" => vec![Effect::Send("safe_area_changed", no_insets())],
            "web_app_request_content_safe_area" => vec![Effect::Send("content_safe_area_changed", no_insets())],
            "web_app_setup_main_button" => {
                self.main = Some(MainButton {
                    visible: data["is_visible"].as_bool().unwrap_or(false),
                    active: data["is_active"].as_bool() != Some(false),
                    text: data["text"].as_str().unwrap_or_default().to_owned(),
                    color: color_of(&data["color"]),
                    text_color: color_of(&data["text_color"]),
                    progress: data["is_progress_visible"].as_bool().unwrap_or(false),
                });
                Vec::new()
            }
            "web_app_setup_back_button" => {
                self.back = data["is_visible"].as_bool().unwrap_or(false);
                Vec::new()
            }
            "web_app_setup_closing_behavior" => {
                self.confirm_close = data["need_confirmation"].as_bool().unwrap_or(false);
                Vec::new()
            }
            "web_app_open_link" => data["url"].as_str().filter(|url| !url.is_empty()).map(|url| Effect::Link(url.to_owned())).into_iter().collect(),
            "web_app_open_tg_link" => {
                data["path_full"].as_str().filter(|path| !path.is_empty()).map(|path| Effect::Link(format!("https://t.me{path}"))).into_iter().collect()
            }
            "web_app_open_popup" => {
                let mut buttons: Vec<PopupButton> = data["buttons"].as_array().map(|list| list.iter().enumerate().map(|(at, raw)| popup_button(raw, at)).collect()).unwrap_or_default();
                if buttons.is_empty() {
                    buttons.push(PopupButton { id: "ok".to_owned(), kind: ButtonKind::Ok, text: "OK".to_owned() });
                }
                self.popup = Some(Popup { title: data["title"].as_str().map(str::to_owned), message: data["message"].as_str().unwrap_or_default().to_owned(), buttons });
                Vec::new()
            }
            "web_app_close" => self.request_close(),
            "web_app_data_send" => vec![Effect::SendData(data["data"].as_str().unwrap_or_default().to_owned())],
            "web_app_read_text_from_clipboard" => {
                let Some(req_id) = data["req_id"].as_str().filter(|id| !id.is_empty()) else { return Vec::new() };
                // One question at a time: the earlier one is answered with nothing.
                let earlier = self.refuse_clipboard();
                self.consent = Some(Consent { req_id: req_id.to_owned(), epoch: self.epoch });
                self.popup = Some(Popup {
                    title: Some("Доступ к буферу обмена".to_owned()),
                    message: format!("{} запрашивает однократное чтение буфера обмена.", self.host),
                    buttons: vec![
                        PopupButton { id: CLIPBOARD_DENY.to_owned(), kind: ButtonKind::Cancel, text: "Запретить".to_owned() },
                        PopupButton { id: CLIPBOARD_ALLOW.to_owned(), kind: ButtonKind::Ok, text: "Разрешить один раз".to_owned() },
                    ],
                });
                earlier.into_iter().collect()
            }
            // This is the person's own client.
            "web_app_request_write_access" => vec![Effect::Send("write_access_requested", json!({ "status": "allowed" }))],
            // The phone number is not shared.
            "web_app_request_phone" => vec![Effect::Send("phone_requested", json!({ "status": "cancelled" }))],
            // Colours of the frame, a settings button, haptics, sensors, storage,
            // biometry: the frame is the client's own and a desktop has none of the rest.
            _ => Vec::new(),
        }
    }

    /// The cross, Escape, a press outside, or the app's own wish to close.
    pub fn request_close(&mut self) -> Vec<Effect> {
        if !self.confirm_close {
            return vec![Effect::Close];
        }
        self.popup = Some(Popup {
            title: None,
            message: "Закрыть приложение? Несохранённые данные будут потеряны.".to_owned(),
            buttons: vec![
                PopupButton { id: CONFIRM_CLOSE_NO.to_owned(), kind: ButtonKind::Cancel, text: "Отмена".to_owned() },
                PopupButton { id: CONFIRM_CLOSE_YES.to_owned(), kind: ButtonKind::Destructive, text: "Закрыть".to_owned() },
            ],
        });
        Vec::new()
    }

    /// A popup answered with one of its buttons, or put away with none.
    pub fn resolve_popup(&mut self, button: Option<&str>) -> Vec<Effect> {
        let id = button.unwrap_or_default();
        self.popup = None;
        if id == CLIPBOARD_ALLOW || id == CLIPBOARD_DENY || self.consent.is_some() {
            let Some(consent) = self.consent.clone() else { return Vec::new() };
            if id != CLIPBOARD_ALLOW || consent.epoch != self.epoch {
                return self.refuse_clipboard().into_iter().collect();
            }
            self.consent = None;
            return vec![Effect::ReadClipboard { req_id: consent.req_id, epoch: consent.epoch }];
        }
        match id {
            CONFIRM_CLOSE_YES => {
                self.confirm_close = false;
                vec![Effect::Close]
            }
            CONFIRM_CLOSE_NO => Vec::new(),
            _ => vec![Effect::Send("popup_closed", json!({ "button_id": id }))],
        }
    }

    /// The clipboard as it was read. A page that navigated since gets nothing.
    pub fn clipboard_read(&self, req_id: &str, epoch: u64, text: Option<String>) -> Effect {
        let data = if epoch == self.epoch { text.unwrap_or_default() } else { String::new() };
        Effect::Send("clipboard_text_received", json!({ "req_id": req_id, "data": data }))
    }

    pub fn press_main(&self) -> Vec<Effect> {
        if self.main.as_ref().is_some_and(|button| button.visible && button.active) { vec![Effect::Send("main_button_pressed", json!({}))] } else { Vec::new() }
    }

    pub fn press_back(&self) -> Vec<Effect> {
        vec![Effect::Send("back_button_pressed", json!({}))]
    }

    /// The app's page began to load another address.
    pub fn navigated(&mut self) -> Vec<Effect> {
        self.epoch += 1;
        let refused = self.refuse_clipboard();
        if refused.is_some() {
            self.popup = None;
        }
        refused.into_iter().collect()
    }

    /// Some apps never say they are ready; a page that finished loading is.
    pub fn loaded(&mut self) {
        self.loading = false;
    }

    pub fn footer(&self) -> bool {
        self.main.as_ref().is_some_and(|button| button.visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Size = Size::new(558.0, 700.0);

    fn bridge() -> Bridge {
        Bridge::new("https://www.app.example.com/start?x=1")
    }

    fn names(effects: &[Effect]) -> Vec<&'static str> {
        effects.iter().filter_map(|effect| if let Effect::Send(name, _) = effect { Some(*name) } else { None }).collect()
    }

    #[test]
    fn scripts_cannot_be_broken_out_of() {
        let script = deliver_script("x');alert(1);//", &json!({ "a": "\"</script>" }));
        assert!(script.contains(r#"receiveEvent("x');alert(1);//","#));
        assert!(script.contains(r#"{"a":"\"</script>"}"#));
        assert_eq!(js_string("a\"\nb"), r#""a\"\nb""#);
    }

    #[test]
    fn events_read_whether_their_data_is_text_or_an_object() {
        assert_eq!(read_event(r#"{"eventType":"web_app_ready","eventData":""}"#), Some(("web_app_ready".into(), json!({}))));
        assert_eq!(read_event(r#"{"eventType":"a","eventData":"{\"is_visible\":true}"}"#), Some(("a".into(), json!({ "is_visible": true }))));
        assert_eq!(read_event(r#"{"eventType":"a","eventData":{"x":1}}"#), Some(("a".into(), json!({ "x": 1 }))));
        assert_eq!(read_event(r#"{"eventType":"a","eventData":"[1,2]"}"#), Some(("a".into(), json!({}))));
        assert_eq!(read_event(r#"{"eventType":"a"}"#), Some(("a".into(), json!({}))));
        assert_eq!(read_event(r#"{"eventType":7}"#), None);
        assert_eq!(read_event("not json"), None);
    }

    #[test]
    fn a_ready_app_is_primed_with_its_theme_and_its_room() {
        let mut bridge = bridge();
        assert!(bridge.loading);
        let effects = bridge.on_event("web_app_ready", &json!({}), VIEW, true);
        assert!(!bridge.loading);
        assert_eq!(names(&effects), ["theme_changed", "viewport_changed", "safe_area_changed", "content_safe_area_changed"]);
        let Effect::Send(_, room) = &effects[1] else { panic!() };
        assert_eq!(room, &json!({ "width": 558.0, "height": 700.0, "is_expanded": true, "is_state_stable": true }));
        let Effect::Send(_, theme) = &effects[0] else { panic!() };
        assert_eq!(theme["theme_params"]["bg_color"], hex(DARK.surface));
        assert_eq!(names(&bridge.on_event("web_app_request_theme", &json!({}), VIEW, false)), ["theme_changed"]);
        assert_eq!(names(&bridge.on_event("web_app_expand", &json!({}), VIEW, false)), ["viewport_changed"]);
        assert_eq!(names(&bridge.on_event("web_app_request_safe_area", &json!({}), VIEW, false)), ["safe_area_changed"]);
    }

    #[test]
    fn the_frame_follows_what_the_app_sets_up() {
        let mut bridge = bridge();
        assert!(bridge.press_main().is_empty());
        let setup = json!({ "is_visible": true, "text": "Оплатить", "color": "#2481cc", "text_color": "#ffffff", "is_progress_visible": true });
        assert!(bridge.on_event("web_app_setup_main_button", &setup, VIEW, false).is_empty());
        let button = bridge.main.clone().unwrap();
        assert!(button.visible && button.active && button.progress);
        assert_eq!((button.text.as_str(), button.color), ("Оплатить", Some(Color::from_rgb8(0x24, 0x81, 0xcc))));
        assert!(bridge.footer());
        assert_eq!(names(&bridge.press_main()), ["main_button_pressed"]);
        let _ = bridge.on_event("web_app_setup_main_button", &json!({ "is_visible": true, "is_active": false, "color": "red" }), VIEW, false);
        assert!(bridge.press_main().is_empty());
        assert_eq!(bridge.main.as_ref().unwrap().color, None);
        let _ = bridge.on_event("web_app_setup_back_button", &json!({ "is_visible": true }), VIEW, false);
        assert!(bridge.back);
        assert_eq!(names(&bridge.press_back()), ["back_button_pressed"]);
    }

    #[test]
    fn links_go_to_the_link_policy_and_nowhere_else() {
        let mut bridge = bridge();
        assert_eq!(bridge.on_event("web_app_open_link", &json!({ "url": "https://example.com/a" }), VIEW, false), [Effect::Link("https://example.com/a".into())]);
        assert_eq!(bridge.on_event("web_app_open_tg_link", &json!({ "path_full": "/channel?start=1" }), VIEW, false), [Effect::Link("https://t.me/channel?start=1".into())]);
        assert!(bridge.on_event("web_app_open_link", &json!({ "url": "" }), VIEW, false).is_empty());
        assert!(bridge.on_event("web_app_open_tg_link", &json!({}), VIEW, false).is_empty());
    }

    #[test]
    fn a_popup_reports_the_button_it_was_closed_with() {
        let mut bridge = bridge();
        let asked = json!({ "title": "Удалить?", "message": "Это навсегда", "buttons": [{ "id": "del", "type": "destructive", "text": "Удалить" }, { "type": "cancel" }] });
        assert!(bridge.on_event("web_app_open_popup", &asked, VIEW, false).is_empty());
        let popup = bridge.popup.clone().unwrap();
        assert_eq!(popup.title.as_deref(), Some("Удалить?"));
        assert_eq!(popup.buttons[0], PopupButton { id: "del".into(), kind: ButtonKind::Destructive, text: "Удалить".into() });
        assert_eq!(popup.buttons[1], PopupButton { id: "button-1".into(), kind: ButtonKind::Cancel, text: "Закрыть".into() });
        assert_eq!(bridge.resolve_popup(Some("del")), [Effect::Send("popup_closed", json!({ "button_id": "del" }))]);
        assert!(bridge.popup.is_none());
        // Without buttons of its own a popup gets one, and Escape closes it with none.
        let _ = bridge.on_event("web_app_open_popup", &json!({ "message": "Готово" }), VIEW, false);
        assert_eq!(bridge.popup.as_ref().unwrap().buttons.len(), 1);
        assert_eq!(bridge.resolve_popup(None), [Effect::Send("popup_closed", json!({ "button_id": "" }))]);
    }

    #[test]
    fn closing_asks_first_when_the_app_wants_it_to() {
        let mut bridge = bridge();
        assert_eq!(bridge.request_close(), [Effect::Close]);
        let _ = bridge.on_event("web_app_setup_closing_behavior", &json!({ "need_confirmation": true }), VIEW, false);
        assert!(bridge.on_event("web_app_close", &json!({}), VIEW, false).is_empty());
        assert!(bridge.popup.is_some());
        assert!(bridge.resolve_popup(Some(CONFIRM_CLOSE_NO)).is_empty());
        assert!(bridge.request_close().is_empty());
        assert_eq!(bridge.resolve_popup(Some(CONFIRM_CLOSE_YES)), [Effect::Close]);
    }

    #[test]
    fn data_for_the_bot_is_handed_over() {
        let mut bridge = bridge();
        assert_eq!(bridge.on_event("web_app_data_send", &json!({ "data": "{\"sum\":5}" }), VIEW, false), [Effect::SendData("{\"sum\":5}".into())]);
    }

    #[test]
    fn the_clipboard_is_read_once_and_only_when_allowed() {
        let mut bridge = bridge();
        assert!(bridge.on_event("web_app_read_text_from_clipboard", &json!({}), VIEW, false).is_empty());
        assert!(bridge.popup.is_none());
        assert!(bridge.on_event("web_app_read_text_from_clipboard", &json!({ "req_id": "r1" }), VIEW, false).is_empty());
        assert!(bridge.popup.as_ref().unwrap().message.starts_with("app.example.com "));
        // A second question answers the first with nothing.
        let earlier = bridge.on_event("web_app_read_text_from_clipboard", &json!({ "req_id": "r2" }), VIEW, false);
        assert_eq!(earlier, [Effect::Send("clipboard_text_received", json!({ "req_id": "r1", "data": "" }))]);
        assert_eq!(bridge.resolve_popup(Some(CLIPBOARD_ALLOW)), [Effect::ReadClipboard { req_id: "r2".into(), epoch: 0 }]);
        assert_eq!(bridge.clipboard_read("r2", 0, Some("текст".into())), Effect::Send("clipboard_text_received", json!({ "req_id": "r2", "data": "текст" })));
        // Denied, or put away with Escape: nothing is read.
        let _ = bridge.on_event("web_app_read_text_from_clipboard", &json!({ "req_id": "r3" }), VIEW, false);
        assert_eq!(bridge.resolve_popup(Some(CLIPBOARD_DENY)), [Effect::Send("clipboard_text_received", json!({ "req_id": "r3", "data": "" }))]);
        let _ = bridge.on_event("web_app_read_text_from_clipboard", &json!({ "req_id": "r4" }), VIEW, false);
        assert_eq!(bridge.resolve_popup(None), [Effect::Send("clipboard_text_received", json!({ "req_id": "r4", "data": "" }))]);
    }

    #[test]
    fn a_page_that_navigated_gets_no_clipboard() {
        let mut bridge = bridge();
        let _ = bridge.on_event("web_app_read_text_from_clipboard", &json!({ "req_id": "r1" }), VIEW, false);
        assert_eq!(bridge.navigated(), [Effect::Send("clipboard_text_received", json!({ "req_id": "r1", "data": "" }))]);
        assert!(bridge.popup.is_none());
        // Allowed, then the page moved on while the clipboard was being read.
        let _ = bridge.on_event("web_app_read_text_from_clipboard", &json!({ "req_id": "r2" }), VIEW, false);
        let Some(Effect::ReadClipboard { req_id, epoch }) = bridge.resolve_popup(Some(CLIPBOARD_ALLOW)).pop() else { panic!() };
        assert!(bridge.navigated().is_empty());
        assert_eq!(bridge.clipboard_read(&req_id, epoch, Some("секрет".into())), Effect::Send("clipboard_text_received", json!({ "req_id": "r2", "data": "" })));
    }

    #[test]
    fn small_requests_get_the_desktop_answers() {
        let mut bridge = bridge();
        assert_eq!(bridge.on_event("web_app_request_write_access", &json!({}), VIEW, false), [Effect::Send("write_access_requested", json!({ "status": "allowed" }))]);
        assert_eq!(bridge.on_event("web_app_request_phone", &json!({}), VIEW, false), [Effect::Send("phone_requested", json!({ "status": "cancelled" }))]);
        for ignored in ["web_app_trigger_haptic_feedback", "web_app_set_header_color", "web_app_setup_settings_button", "web_app_switch_inline_query", "anything_else"] {
            assert!(bridge.on_event(ignored, &json!({ "color": "#000000" }), VIEW, false).is_empty(), "{ignored}");
        }
    }

    #[test]
    fn an_app_is_named_by_its_host() {
        assert_eq!(title_of("https://www.app.example.com/start"), "app.example.com");
        assert_eq!(title_of("https://shop.example.com:8443/"), "shop.example.com:8443");
        assert_eq!(title_of("nonsense"), "Мини-приложение");
        let mut bridge = bridge();
        bridge.loaded();
        assert!(!bridge.loading && !bridge.footer());
    }
}
