use iced_test::Simulator;
use iced_test::core::Settings as SimSettings;
use tbs_core::api::{TgButtonSource, TgChat, TgChatPage, TgMessage, TgMessagePage, TgStatus};
use tbs_core::model::UserInfo;

use super::client::{AppRequest, Target};
use super::*;

fn ctx(dir: &tempfile::TempDir, admin: bool) -> Ctx {
    let mut ctx = Ctx::for_tests(dir.path());
    let role = if admin { "admin" } else { "user" };
    ctx.user = serde_json::from_value::<UserInfo>(serde_json::json!({ "id": 1, "username": "u", "role": role })).ok();
    ctx
}

fn scope(id: i64, active: bool, sources: bool) -> AccessScope {
    AccessScope { id, is_active: active, allow_sources: sources, ..AccessScope::default() }
}

/// The section open and on screen.
fn ready() -> Telegram {
    let mut screen = Telegram::new();
    screen.access = Access::Open;
    screen.set_shown(true);
    screen
}

fn status(state: &str) -> Msg {
    Msg::Chats(client::Msg::Status(Ok(TgStatus { state: state.into(), raw_state: state.into(), ..TgStatus::default() })))
}

fn chat(id: i64, title: &str, kind: &str) -> TgChat {
    TgChat { chat_id: id, title: title.into(), chat_type: kind.into(), ..TgChat::default() }
}

/// Telegram signed in, two chats, and two messages in the first.
fn chatting(ctx: &mut Ctx) -> Telegram {
    let mut screen = ready();
    let _ = screen.update(status("ready"), ctx);
    let chats = vec![chat(-10, "Касса чеки", "supergroup"), chat(20, "Платёжный бот", "bot")];
    let _ = screen.update(Msg::Chats(client::Msg::Chats(Ok(TgChatPage { total: 2, items: chats }))), ctx);
    let message = |id: i64, words: &str| TgMessage { id: Some(id), date: Some("2026-11-11T10:00:00".into()), text: Some(words.into()), ..TgMessage::default() };
    let items = vec![message(2, "Оплата прошла"), message(1, "Добрый день")];
    let _ = screen.update(Msg::Chats(client::Msg::Messages(-10, 1, Ok(TgMessagePage { total: 2, items }))), ctx);
    screen
}

fn sees(element: El<'_, Msg>, wanted: &str) -> bool {
    Simulator::with_size(SimSettings::default(), iced::Size::new(1360.0, 860.0), element).find(wanted).is_ok()
}

#[test]
fn an_admin_goes_straight_in() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, true);
    let mut screen = Telegram::new();
    let _ = screen.enter(&mut ctx);
    assert_eq!(screen.access, Access::Open);
}

#[test]
fn a_user_is_checked_first_and_stays_out_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, false);
    let mut screen = Telegram::new();
    screen.set_shown(true);
    let _ = screen.enter(&mut ctx);
    assert_eq!(screen.access, Access::Checking);
    let _ = screen.update(Msg::Status(Err("нет сети".into())), &mut ctx);
    assert!(matches!(screen.access, Access::Closed { title: "Доступ временно ограничен", .. }));
    assert!(!screen.visible());
    assert!(screen.overlay(&ctx).is_none());
}

#[test]
fn no_scopes_means_nothing_to_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, false);
    let mut screen = Telegram::new();
    let _ = screen.update(Msg::Scopes(Ok(vec![scope(1, false, true), scope(2, true, false)])), &mut ctx);
    assert!(matches!(screen.access, Access::Closed { title: "Доступ закрыт", .. }));
}

#[test]
fn the_first_scope_with_sources_is_unlocked_and_asks_for_a_code_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, false);
    let mut screen = Telegram::new();
    let _ = screen.update(Msg::Scopes(Ok(vec![scope(1, true, false), scope(7, true, true)])), &mut ctx);
    assert_eq!(screen.access, Access::Gate);
    assert_eq!(screen.otp.scope, 7);
    assert!(screen.otp.requesting);
    let _ = screen.update(Msg::Requested(Ok(ScopeCode { expires_in: 120, code_length: 6, ..ScopeCode::default() })), &mut ctx);
    assert!(!screen.otp.requesting);
    assert_eq!(screen.otp.seconds_left, 120);
}

#[test]
fn a_code_takes_digits_only_and_a_short_one_is_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, false);
    let mut screen = Telegram::new();
    screen.access = Access::Gate;
    let _ = screen.update(Msg::Code("12a3".into()), &mut ctx);
    assert_eq!(screen.otp.code, "123");
    let _ = screen.update(Msg::Verify, &mut ctx);
    assert!(!screen.otp.verifying);
    let _ = screen.update(Msg::Code("12345678".into()), &mut ctx);
    assert_eq!(screen.otp.code, "123456");
    let _ = screen.update(Msg::Verify, &mut ctx);
    assert!(screen.otp.verifying);
    let _ = screen.update(Msg::Verified(Err("неверный код".into())), &mut ctx);
    assert_eq!(screen.access, Access::Gate);
    assert_eq!(screen.otp.error.as_deref(), Some("неверный код"));
}

#[test]
fn a_verified_code_opens_the_section_and_refreshes_the_table() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, false);
    let mut screen = Telegram::new();
    screen.access = Access::Gate;
    let _ = screen.update(Msg::Verified(Ok(UnlockedScope::default())), &mut ctx);
    assert_eq!(screen.access, Access::Open);
    assert!(ctx.requests.iter().any(|r| matches!(r, Request::TransactionsChanged)));
}

#[test]
fn a_skipped_otp_opens_without_a_form() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, false);
    let mut screen = Telegram::new();
    screen.access = Access::Gate;
    let code = ScopeCode { otp_skipped: true, token: Some("t".into()), ..ScopeCode::default() };
    let _ = screen.update(Msg::Requested(Ok(code)), &mut ctx);
    assert_eq!(screen.access, Access::Open);
}

#[test]
fn an_expired_scope_closes_the_section_while_it_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, false);
    let mut screen = ready();
    assert!(screen.visible());
    screen.client.wants.push(Want::App(AppRequest { chat: 5, bot: None, url: "https://app.example.com/".into(), source: TgButtonSource::Inline, text: "Открыть".into() }));
    let _ = screen.update(Msg::Tick, &mut ctx);
    assert!(screen.mini.is_open());
    let _ = screen.scope_expired(&mut ctx);
    assert_eq!(screen.access, Access::Checking);
    assert!(!screen.visible());
    assert!(!screen.mini.is_open(), "a mini app does not outlive the access it was opened with");
}

#[test]
fn leaving_the_section_hides_it() {
    let mut screen = ready();
    assert!(screen.visible());
    screen.set_shown(false);
    assert!(!screen.visible());
    screen.set_shown(true);
    assert!(screen.visible());
}

#[test]
fn only_allowed_links_raise_a_question() {
    let mut screen = ready();
    let _ = screen.ask_external("https://evil.example.com/");
    assert!(!screen.asking);
    let _ = screen.ask_external("https://t.me/x");
    assert!(screen.asking);
}

#[test]
fn a_link_button_goes_through_the_same_policy_and_says_when_it_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, true);
    let mut screen = ready();
    screen.client.wants.push(Want::Link("https://evil.example.com/".into()));
    let _ = screen.update(Msg::Tick, &mut ctx);
    assert!(!screen.asking);
    assert_eq!(ctx.toasts.len(), 1);
    screen.client.wants.push(Want::Link("https://t.me/channel".into()));
    screen.client.wants.push(Want::Link("https://t.me/second".into()));
    let _ = screen.update(Msg::Tick, &mut ctx);
    assert!(screen.asking && screen.client.wants.is_empty());
    assert_eq!(ctx.toasts.len(), 1, "an allowed link is asked about, not announced");
    let _ = screen.update(Msg::Asked("https://t.me/channel".into(), false), &mut ctx);
    assert!(!screen.asking);
}

#[test]
fn a_mini_app_s_links_are_asked_about_one_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, true);
    let mut screen = ready();
    let _ = screen.update(Msg::Signal(Signal::External("https://t.me/x".into())), &mut ctx);
    assert!(!screen.asking, "nothing is open to have asked");
    screen.client.wants.push(Want::App(AppRequest { chat: 5, bot: None, url: "https://app.example.com/".into(), source: TgButtonSource::Menu, text: "Магазин".into() }));
    let _ = screen.update(Msg::Tick, &mut ctx);
    assert!(screen.mini.is_open() && screen.overlay(&ctx).is_some());
    let app = tbs_core::api::TgWebApp { url: "https://app.example.com/start".into(), launch_id: None };
    let _ = screen.update(Msg::Mini(miniapp::Msg::Resolved(screen.mini.token(), 5, Ok(app))), &mut ctx);
    let _ = screen.update(Msg::Signal(Signal::External("https://evil.example.com/".into())), &mut ctx);
    assert!(!screen.asking && ctx.toasts.is_empty(), "what a page tried by itself is dropped quietly");
    let _ = screen.update(Msg::Signal(Signal::External("https://t.me/x".into())), &mut ctx);
    assert!(screen.asking);
}

#[test]
fn escape_takes_the_topmost_thing_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, true);
    let mut screen = chatting(&mut ctx);
    let _ = screen.update(Msg::Chats(client::Msg::ToggleSelecting), &mut ctx);
    let _ = screen.update(Msg::Chats(client::Msg::Context(Target::Message(1))), &mut ctx);
    screen.client.wants.push(Want::App(AppRequest { chat: -10, bot: None, url: "https://app.example.com/".into(), source: TgButtonSource::Inline, text: "Открыть".into() }));
    let _ = screen.update(Msg::Tick, &mut ctx);
    assert!(screen.mini.is_open());
    let _ = screen.update(Msg::Hotkey(Hotkey::Escape), &mut ctx);
    assert!(!screen.mini.is_open() && screen.client.menu.is_some());
    let _ = screen.update(Msg::Hotkey(Hotkey::Escape), &mut ctx);
    assert!(screen.client.menu.is_none() && screen.client.selecting);
    let _ = screen.update(Msg::Hotkey(Hotkey::Escape), &mut ctx);
    assert!(!screen.client.selecting);
}

#[test]
fn the_selection_keys_work_only_in_selection_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, true);
    let mut screen = chatting(&mut ctx);
    let _ = screen.update(Msg::Hotkey(Hotkey::SelectAll), &mut ctx);
    assert!(!screen.client.selecting && screen.client.selected.is_empty());
    let _ = screen.update(Msg::Chats(client::Msg::ToggleSelecting), &mut ctx);
    let _ = screen.update(Msg::Hotkey(Hotkey::SelectAll), &mut ctx);
    assert_eq!(screen.client.selected.len(), 2);
    let _ = screen.update(Msg::Hotkey(Hotkey::Enter), &mut ctx);
    assert!(screen.client.bulk_busy());
    let _ = screen.update(Msg::Hotkey(Hotkey::Escape), &mut ctx);
    assert!(screen.client.selecting, "a run in progress is not walked away from");
    let _ = screen.update(Msg::Hotkey(Hotkey::SelectNone), &mut ctx);
    assert!(screen.client.selected.is_empty());
    // Ctrl held turns a press on a message into a tick.
    let mut screen = chatting(&mut ctx);
    let _ = screen.update(Msg::Hotkey(Hotkey::Command(true)), &mut ctx);
    let _ = screen.update(Msg::Chats(client::Msg::RowPress(2)), &mut ctx);
    assert!(screen.client.selecting && screen.client.selected.contains(&2));
}

#[test]
fn keys_reach_the_section_as_the_shortcuts_they_are() {
    use iced::keyboard::{Location, Modifiers, key::Code, key::Physical};
    let press = |key: Key, code: Code, modifiers: Modifiers| {
        Event::Keyboard(keyboard::Event::KeyPressed { key: key.clone(), modified_key: key, physical_key: Physical::Code(code), location: Location::Standard, modifiers, text: None, repeat: false })
    };
    let heard = |event: Event, status: event::Status| match hotkey(event, status, window::Id::unique()) {
        Some(Msg::Hotkey(key)) => Some(key),
        _ => None,
    };
    let letter = |letter: &str| Key::Character(letter.into());
    assert_eq!(heard(press(Key::Named(Named::Escape), Code::Escape, Modifiers::empty()), event::Status::Captured), Some(Hotkey::Escape));
    assert_eq!(heard(press(Key::Named(Named::Enter), Code::Enter, Modifiers::empty()), event::Status::Ignored), Some(Hotkey::Enter));
    assert_eq!(heard(press(letter("a"), Code::KeyA, Modifiers::COMMAND), event::Status::Ignored), Some(Hotkey::SelectAll));
    // The Russian layout types «в» on the key of D.
    assert_eq!(heard(press(letter("в"), Code::KeyD, Modifiers::COMMAND), event::Status::Ignored), Some(Hotkey::SelectNone));
    // A key a field took for itself is that field's, and a plain letter is no shortcut.
    assert_eq!(heard(press(Key::Named(Named::Enter), Code::Enter, Modifiers::empty()), event::Status::Captured), None);
    assert_eq!(heard(press(letter("a"), Code::KeyA, Modifiers::COMMAND), event::Status::Captured), None);
    assert_eq!(heard(press(letter("a"), Code::KeyA, Modifiers::empty()), event::Status::Ignored), None);
    assert_eq!(heard(Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::COMMAND)), event::Status::Ignored), Some(Hotkey::Command(true)));
}

#[test]
fn the_gate_and_the_sign_in_are_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, false);
    let mut screen = Telegram::new();
    screen.access = Access::Gate;
    assert!(sees(screen.view(&ctx), "Доступ ограничен") && sees(screen.view(&ctx), "Подтвердить код"));
    let mut screen = ready();
    assert!(sees(screen.view(&ctx), "Подключаемся"));
    for (state, title) in [("wait_phone_number", "Введите номер телефона"), ("wait_code", "Шестизначный код"), ("wait_password", "Облачный пароль")] {
        let _ = screen.update(status(state), &mut ctx);
        assert!(sees(screen.view(&ctx), title) && sees(screen.view(&ctx), "Продолжить"), "{state}");
    }
    let _ = screen.update(Msg::Chats(client::Msg::AuthDone(Err("PHONE_CODE_INVALID".into()))), &mut ctx);
    assert!(sees(screen.view(&ctx), "PHONE_CODE_INVALID"));
    let _ = screen.update(status("tdlib_unavailable"), &mut ctx);
    assert!(sees(screen.view(&ctx), "Сервис недоступен"));
    let _ = screen.update(status("closed"), &mut ctx);
    assert!(sees(screen.view(&ctx), "Инициализация"));
}

#[test]
fn the_chats_and_what_opens_over_them_are_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir, true);
    let mut screen = chatting(&mut ctx);
    for wanted in ["Чаты", "подключено", "Активные", "Касса чеки", "Платёжный бот", "Оплата прошла", "Добрый день", "Обработать", "группа"] {
        assert!(sees(screen.view(&ctx), wanted), "{wanted}");
    }
    assert!(screen.overlay(&ctx).is_none());

    let _ = screen.update(Msg::Chats(client::Msg::ToggleSelecting), &mut ctx);
    let _ = screen.update(Msg::Chats(client::Msg::Pick(1)), &mut ctx);
    assert!(sees(screen.view(&ctx), "В обработку · 1") && sees(screen.view(&ctx), "Выбрать все · 2"));
    assert!(!sees(screen.view(&ctx), "Обработать"), "one message is not sent off while several are being picked");

    let _ = screen.update(Msg::Chats(client::Msg::Context(Target::Chat(20))), &mut ctx);
    let menu = screen.overlay(&ctx).unwrap();
    assert!(sees(menu, "Скрыть") && sees(screen.overlay(&ctx).unwrap(), "Поставить пароль"));
    let _ = screen.update(Msg::Chats(client::Msg::AskPassword(20, client::PasswordMode::Set)), &mut ctx);
    assert!(sees(screen.overlay(&ctx).unwrap(), "Установить пароль") && sees(screen.overlay(&ctx).unwrap(), "Платёжный бот"));
    let _ = screen.update(Msg::Hotkey(Hotkey::Escape), &mut ctx);
    let _ = screen.update(Msg::Chats(client::Msg::TogglePeriod), &mut ctx);
    assert!(sees(screen.overlay(&ctx).unwrap(), "Последние 7 дней") && sees(screen.overlay(&ctx).unwrap(), "Свой диапазон"));

    // A narrow window shows one pane at a time, with a way back.
    let _ = screen.update(Msg::Hotkey(Hotkey::Escape), &mut ctx);
    ctx.window = iced::Size::new(700.0, 600.0);
    let narrow = |screen: &Telegram, ctx: &Ctx, wanted: &str| Simulator::with_size(SimSettings::default(), iced::Size::new(492.0, 600.0), screen.view(ctx)).find(wanted).is_ok();
    assert!(narrow(&screen, &ctx, "Оплата прошла") && !narrow(&screen, &ctx, "Чаты"));
    let _ = screen.update(Msg::Chats(client::Msg::Back), &mut ctx);
    assert!(narrow(&screen, &ctx, "Чаты") && !narrow(&screen, &ctx, "Оплата прошла"));
}
