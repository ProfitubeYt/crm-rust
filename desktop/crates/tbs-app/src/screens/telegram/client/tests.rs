use std::sync::Arc;

use iced::widget::image;
use serde_json::json;
use tbs_core::api::{TgButton, TgButtonSource, TgChatPage, TgFile, TgMarkup, TgMessagePage, TgStatus, TgUnlock};

use super::*;

fn ctx(dir: &tempfile::TempDir) -> Ctx {
    Ctx::for_tests(dir.path())
}

fn status(state: &str) -> TgStatus {
    TgStatus { state: state.into(), raw_state: state.into(), is_authorized: state == "ready", ..TgStatus::default() }
}

fn chat(id: i64, title: &str, kind: &str) -> TgChat {
    TgChat { chat_id: id, title: title.into(), chat_type: kind.into(), ..TgChat::default() }
}

fn page(chats: Vec<TgChat>) -> TgChatPage {
    TgChatPage { total: chats.len() as i64, items: chats }
}

fn message(id: i64, date: &str, words: &str) -> TgMessage {
    TgMessage { id: Some(id), date: Some(date.into()), text: Some(words.into()), ..TgMessage::default() }
}

fn messages(items: Vec<TgMessage>) -> Result<TgMessagePage, Refusal> {
    Ok(TgMessagePage { total: items.len() as i64, items })
}

/// Signed in to Telegram, with three chats and the first of them open.
fn working(ctx: &mut Ctx) -> Client {
    let mut client = Client::new();
    let _ = client.update(Msg::Status(Ok(status("ready"))), ctx);
    let hidden = TgChat { is_hidden: true, ..chat(-30, "Архив", "channel") };
    let _ = client.update(Msg::Chats(Ok(page(vec![chat(-10, "Касса чеки", "supergroup"), chat(20, "Платёжный бот", "bot"), hidden]))), ctx);
    client
}

/// The same with three messages of two days in the open chat, as the server sends them.
fn reading(ctx: &mut Ctx) -> Client {
    let mut client = working(ctx);
    let items = vec![message(3, "2026-11-11T10:02:00", "третье"), message(2, "2026-11-11T10:01:00", "второе"), message(1, "2026-11-10T09:00:00", "первое")];
    let _ = client.update(Msg::Messages(-10, client.opening, messages(items)), ctx);
    client
}

fn typed(client: &mut Client, ctx: &mut Ctx, words: &str) {
    let _ = client.update(Msg::Draft(text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(words.to_owned())))), ctx);
}

#[test]
fn telegram_is_signed_in_step_by_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = Client::new();
    assert!(client.step().is_none() && !client.ready());
    let _ = client.update(Msg::Status(Ok(status("wait_phone_number"))), &mut ctx);
    assert_eq!(client.step(), Some(Step::Phone));

    // Nothing typed, nothing sent.
    let _ = client.update(Msg::AuthSubmit, &mut ctx);
    assert!(!client.auth.busy);
    let _ = client.update(Msg::AuthInput("+10000000000".into()), &mut ctx);
    let _ = client.update(Msg::AuthSubmit, &mut ctx);
    assert!(client.auth.busy);
    // While it is on its way the box is not typed over, and it is not sent twice.
    let _ = client.update(Msg::AuthInput("+19999999999".into()), &mut ctx);
    assert_eq!(client.auth.value, "+10000000000");

    let _ = client.update(Msg::AuthDone(Err("Failed to set phone number: PHONE_NUMBER_INVALID".into())), &mut ctx);
    assert!(!client.auth.busy);
    assert_eq!(client.auth.error.as_deref(), Some("Failed to set phone number: PHONE_NUMBER_INVALID"));
    assert_eq!(client.auth.value, "+10000000000", "a refused number stays to be corrected");

    let _ = client.update(Msg::AuthSubmit, &mut ctx);
    let _ = client.update(Msg::AuthDone(Ok(status("wait_code"))), &mut ctx);
    assert_eq!(client.step(), Some(Step::Code));
    assert!(client.auth.value.is_empty() && client.auth.error.is_none(), "the number is not carried into the code's box");

    let _ = client.update(Msg::AuthInput("12345".into()), &mut ctx);
    let _ = client.update(Msg::AuthSubmit, &mut ctx);
    let _ = client.update(Msg::AuthDone(Ok(status("wait_password"))), &mut ctx);
    assert_eq!(client.step(), Some(Step::Password));
    assert!(client.auth.value.is_empty());

    let _ = client.update(Msg::AuthInput("облачный".into()), &mut ctx);
    let _ = client.update(Msg::AuthSubmit, &mut ctx);
    let _ = client.update(Msg::AuthDone(Ok(status("ready"))), &mut ctx);
    assert!(client.ready() && client.step().is_none() && client.auth.value.is_empty());
    assert!(client.chats_busy, "the chats are asked for as soon as Telegram is ready");
}

#[test]
fn a_lost_answer_keeps_what_was_known() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = Client::new();
    let _ = client.update(Msg::Status(Err("Нет связи с сервером".into())), &mut ctx);
    assert!(client.status.is_none() && client.status_error.is_some());
    let mut client = working(&mut ctx);
    let _ = client.update(Msg::Status(Err("Нет связи с сервером".into())), &mut ctx);
    assert!(client.ready(), "one lost answer does not sign Telegram out");
    let _ = client.update(Msg::Status(Ok(status("ready"))), &mut ctx);
    assert!(client.status_error.is_none());
}

#[test]
fn what_is_due_is_asked_once_and_not_while_it_is_on_its_way() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = Client::new();
    let start = Instant::now();
    let _ = client.poll(&ctx, start);
    assert!(client.status_busy && !client.chats_busy, "chats wait for Telegram to be ready");
    let _ = client.update(Msg::Status(Ok(status("ready"))), &mut ctx);
    assert!(!client.status_busy && client.chats_busy);
    let _ = client.update(Msg::Chats(Ok(page(vec![chat(-10, "Касса", "group")]))), &mut ctx);
    assert!(client.messages_busy, "the first chat is opened and its messages asked for");
    let _ = client.update(Msg::Messages(-10, client.opening, messages(Vec::new())), &mut ctx);

    // A second later nothing is due; after their intervals each is.
    let _ = client.poll(&ctx, start + Duration::from_secs(1));
    assert!(!client.status_busy && !client.chats_busy && !client.messages_busy);
    let _ = client.poll(&ctx, Instant::now() + MESSAGES_EVERY);
    assert!(client.messages_busy && !client.chats_busy && !client.status_busy);
    let _ = client.poll(&ctx, Instant::now() + CHATS_EVERY);
    assert!(client.chats_busy && !client.status_busy);
    let _ = client.poll(&ctx, Instant::now() + STATUS_READY);
    assert!(client.status_busy);
    // Coming back to the section asks for everything again, but not twice.
    let _ = client.update(Msg::Status(Ok(status("ready"))), &mut ctx);
    let _ = client.resume(&ctx);
    assert!(client.status_busy);
}

#[test]
fn the_first_listed_chat_opens_by_itself_and_the_tabs_filter() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    assert_eq!(client.active, Some(-10));
    assert_eq!(client.counts(), (3, 2, 1));
    let titles = |client: &Client| client.listed().iter().map(|chat| chat.title.clone()).collect::<Vec<_>>();
    assert_eq!(titles(&client), ["Касса чеки", "Платёжный бот"]);
    let _ = client.update(Msg::Filter(Filter::Hidden), &mut ctx);
    assert_eq!(titles(&client), ["Архив"]);
    let _ = client.update(Msg::Filter(Filter::All), &mut ctx);
    assert_eq!(titles(&client).len(), 3);
    let _ = client.update(Msg::Query("  БОТ ".into()), &mut ctx);
    assert_eq!(titles(&client), ["Платёжный бот"]);
    let _ = client.update(Msg::Query("нет такого".into()), &mut ctx);
    assert!(client.listed().is_empty());
    assert_eq!(client.active, Some(-10), "a search does not close the open chat");
}

#[test]
fn a_chat_found_by_its_username_and_a_return_to_the_list_that_stays() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = Client::new();
    let _ = client.update(Msg::Status(Ok(status("ready"))), &mut ctx);
    let named = TgChat { username: Some("pay_bot".into()), ..chat(20, "Платежи", "bot") };
    let _ = client.update(Msg::Chats(Ok(page(vec![TgChat { is_hidden: true, ..chat(1, "Скрытый", "user") }, named.clone()]))), &mut ctx);
    assert_eq!(client.active, Some(20), "a hidden chat is not the one opened by default");
    let _ = client.update(Msg::Query("PAY".into()), &mut ctx);
    assert_eq!(client.listed().len(), 1);
    let _ = client.update(Msg::Back, &mut ctx);
    assert_eq!(client.active, None);
    let _ = client.update(Msg::Chats(Ok(page(vec![named]))), &mut ctx);
    assert_eq!(client.active, None, "the list stays once the person went back to it");
    let _ = client.update(Msg::Open(20), &mut ctx);
    assert_eq!(client.active, Some(20));
}

#[test]
fn messages_read_oldest_first_and_an_answer_for_another_chat_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    assert!(client.messages_loaded);
    assert_eq!(client.messages.iter().map(|message| message.id.unwrap()).collect::<Vec<_>>(), [1, 2, 3]);

    let stale = client.opening;
    let _ = client.update(Msg::Open(20), &mut ctx);
    assert!(client.messages.is_empty() && !client.messages_loaded);
    let _ = client.update(Msg::Messages(-10, stale, messages(vec![message(9, "2026-11-11T11:00:00", "чужое")])), &mut ctx);
    assert!(client.messages.is_empty(), "the chat that was left does not fill the one that is open");
    let _ = client.update(Msg::Messages(20, client.opening, messages(vec![message(5, "2026-11-11T11:00:00", "своё")])), &mut ctx);
    assert_eq!(client.messages.len(), 1);
    // A failure after messages were shown leaves them on screen.
    let _ = client.update(Msg::Messages(20, client.opening, Err(Refusal { locked: false, barred: false, text: "Нет связи с сервером".into() })), &mut ctx);
    assert_eq!(client.messages.len(), 1);
    assert_eq!(client.messages_error.as_deref(), Some("Нет связи с сервером"));
}

#[test]
fn opening_another_chat_starts_it_clean() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    let _ = client.update(Msg::Pick(2), &mut ctx);
    let _ = client.update(Msg::Preset(Preset::Today), &mut ctx);
    let _ = client.update(Msg::Scrolled(400.0), &mut ctx);
    assert!(client.selecting && client.range.is_set() && client.away);
    let _ = client.update(Msg::Open(20), &mut ctx);
    assert!(!client.selecting && client.selected.is_empty() && !client.range.is_set() && !client.away);
    // The same chat pressed again is not reloaded.
    let opening = client.opening;
    let _ = client.update(Msg::Open(20), &mut ctx);
    assert_eq!(client.opening, opening);
}

#[test]
fn a_period_leaves_the_messages_of_its_days() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    let ids = |client: &Client| client.shown().iter().map(|message| message.id.unwrap()).collect::<Vec<_>>();
    let _ = client.update(Msg::TogglePeriod, &mut ctx);
    assert!(client.period.as_ref().is_some_and(|form| !form.custom));
    let _ = client.update(Msg::PeriodCustom, &mut ctx);
    let _ = client.update(Msg::PeriodFrom("11.11.2026".into()), &mut ctx);
    let _ = client.update(Msg::PeriodTo("вчера".into()), &mut ctx);
    let _ = client.update(Msg::PeriodApply, &mut ctx);
    assert!(client.period.as_ref().is_some_and(|form| form.error.is_some()), "a date that cannot be read is said, not guessed");
    assert_eq!(ids(&client), [1, 2, 3]);
    let _ = client.update(Msg::PeriodTo(String::new()), &mut ctx);
    let _ = client.update(Msg::PeriodApply, &mut ctx);
    assert!(client.period.is_none());
    assert_eq!(ids(&client), [2, 3]);
    assert_eq!(client.selectable(), 2);
    // Opened again it shows what is chosen, as a range of its own.
    let _ = client.update(Msg::TogglePeriod, &mut ctx);
    let form = client.period.as_ref().unwrap();
    assert!(form.custom && form.from == "11.11.2026" && form.to.is_empty());
    let _ = client.update(Msg::PeriodFrom("12.11.2026".into()), &mut ctx);
    let _ = client.update(Msg::PeriodTo("10.11.2026".into()), &mut ctx);
    let _ = client.update(Msg::PeriodApply, &mut ctx);
    assert!(client.period.is_some(), "a period that ends before it starts is not applied");
    let _ = client.update(Msg::ClearRange, &mut ctx);
    assert!(!client.range.is_set() && client.period.is_none());
    assert_eq!(ids(&client), [1, 2, 3]);
}

#[test]
fn a_message_is_sent_once_and_the_box_is_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    let _ = client.update(Msg::Send, &mut ctx);
    assert!(!client.sending(), "an empty box sends nothing");
    typed(&mut client, &mut ctx, "  привет  ");
    let _ = client.update(Msg::Scrolled(500.0), &mut ctx);
    let _ = client.update(Msg::Send, &mut ctx);
    assert!(client.sending() && client.draft.text().trim().is_empty() && !client.away);
    typed(&mut client, &mut ctx, "ещё");
    let _ = client.update(Msg::Send, &mut ctx);
    assert_eq!(client.draft.text().trim(), "ещё", "the next one waits for the first to go");
    let _ = client.update(Msg::Sent(-10, Err("Failed to send message: CHAT_WRITE_FORBIDDEN".into())), &mut ctx);
    assert!(!client.sending());
    assert_eq!(ctx.toasts, [(Tone::Danger, "Failed to send message: CHAT_WRITE_FORBIDDEN".to_owned())]);
    let _ = client.update(Msg::Send, &mut ctx);
    let _ = client.update(Msg::Sent(-10, Ok(())), &mut ctx);
    assert!(!client.sending() && client.messages_busy, "the chat is read again to show what was sent");
}

#[test]
fn a_file_is_attached_once_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    let _ = client.update(Msg::Attach, &mut ctx);
    assert!(client.sending());
    let _ = client.update(Msg::Attached(-10, Ok(false)), &mut ctx);
    assert!(!client.sending() && ctx.toasts.is_empty(), "a closed dialog is not a failure");
    let _ = client.update(Msg::Attach, &mut ctx);
    let _ = client.update(Msg::Attached(-10, Err("Only PDF files are supported".into())), &mut ctx);
    assert_eq!(ctx.toasts.pop(), Some((Tone::Danger, "Only PDF files are supported".to_owned())));
    let _ = client.update(Msg::Attach, &mut ctx);
    let _ = client.update(Msg::Attached(-10, Ok(true)), &mut ctx);
    assert_eq!(ctx.toasts.pop(), Some((Tone::Success, "Файл отправлен".to_owned())));
}

#[test]
fn a_receipt_goes_to_the_worker_and_its_verdict_is_shown() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    let work = |client: &Client, id: i64| client.work_of(-10, client.messages.iter().find(|message| message.id == Some(id)).unwrap());
    assert_eq!(work(&client, 2), Work::Idle);
    let _ = client.update(Msg::Process(2), &mut ctx);
    assert_eq!(work(&client, 2), Work::Busy);
    assert_eq!(client.in_flight(), 1);
    let _ = client.update(Msg::Process(2), &mut ctx);
    assert_eq!(client.in_flight(), 1, "a message is not handed over twice");
    let _ = client.update(Msg::Processed(-10, 2, false), &mut ctx);
    assert_eq!(work(&client, 2), Work::Failed);
    assert!(ctx.requests.is_empty());
    // A second try that works takes the failure away and tells the table.
    let _ = client.update(Msg::Process(2), &mut ctx);
    let _ = client.update(Msg::Processed(-10, 2, true), &mut ctx);
    assert_eq!(work(&client, 2), Work::Done);
    assert!(ctx.requests.iter().any(|request| matches!(request, Request::TransactionsChanged)));
    // What the server already knows is shown without asking.
    let known = vec![
        TgMessage { processed: Some(true), processed_status: Some("done".into()), ..message(7, "2026-11-11T10:00:00", "готовый") },
        TgMessage { processed: Some(false), processed_status: Some("failed".into()), ..message(8, "2026-11-11T10:01:00", "сбой") },
    ];
    let _ = client.update(Msg::Messages(-10, client.opening, messages(known)), &mut ctx);
    assert_eq!((work(&client, 7), work(&client, 8)), (Work::Done, Work::Failed));
    // The same number in another chat is another message.
    assert_eq!(client.work_of(20, &message(2, "2026-11-11T10:01:00", "чужое")), Work::Idle);
}

#[test]
fn selected_messages_are_processed_one_after_another() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    let _ = client.update(Msg::BulkStart, &mut ctx);
    assert!(client.bulk.is_none(), "nothing selected, nothing to start");
    let _ = client.update(Msg::SelectAll, &mut ctx);
    assert!(client.selecting);
    assert_eq!(client.selected.iter().copied().collect::<Vec<_>>(), [1, 2, 3]);
    let _ = client.update(Msg::BulkStart, &mut ctx);
    assert!(client.bulk_busy());
    assert_eq!(client.in_flight(), 1, "one at a time");
    // The selection cannot be left while it is being worked through.
    let _ = client.update(Msg::CancelSelection, &mut ctx);
    let _ = client.update(Msg::ToggleSelecting, &mut ctx);
    assert!(client.selecting && client.bulk_busy());
    let _ = client.update(Msg::BulkStart, &mut ctx);
    assert_eq!(client.in_flight(), 1);

    let _ = client.update(Msg::Processed(-10, 1, true), &mut ctx);
    let _ = client.update(Msg::Processed(-10, 2, false), &mut ctx);
    let bulk = client.bulk.clone().unwrap();
    assert_eq!((bulk.processed, bulk.failed, bulk.total, bulk.busy), (1, 1, 3, true));
    assert!(ctx.toasts.is_empty() && ctx.requests.is_empty());
    let _ = client.update(Msg::Processed(-10, 3, true), &mut ctx);
    assert!(!client.bulk_busy());
    assert_eq!(ctx.toasts, [(Tone::Warning, "Готово: 2, ошибок: 1".to_owned())]);
    assert_eq!(ctx.requests.len(), 1, "the table is told once, at the end");
    let _ = client.update(Msg::CancelSelection, &mut ctx);
    assert!(!client.selecting && client.selected.is_empty() && client.bulk.is_none());
}

#[test]
fn messages_are_ticked_by_a_press_in_selection_mode_or_with_ctrl() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    let _ = client.update(Msg::RowPress(2), &mut ctx);
    assert!(!client.selecting, "a plain press on a message does nothing");
    let _ = client.update(Msg::Command(true), &mut ctx);
    let _ = client.update(Msg::RowPress(2), &mut ctx);
    assert!(client.selecting && client.selected.contains(&2));
    let _ = client.update(Msg::Command(false), &mut ctx);
    let _ = client.update(Msg::RowPress(3), &mut ctx);
    let _ = client.update(Msg::RowPress(2), &mut ctx);
    assert_eq!(client.selected.iter().copied().collect::<Vec<_>>(), [3]);
    let _ = client.update(Msg::SelectNone, &mut ctx);
    assert!(client.selecting && client.selected.is_empty());
    // Escape leaves the mode; with nothing open it has nothing to do.
    assert!(client.escape());
    assert!(!client.selecting);
    assert!(!client.escape());
    // The menu's «Добавить в выбор» starts the mode with that message.
    let _ = client.update(Msg::Pointer(Point::new(40.0, 60.0)), &mut ctx);
    let _ = client.update(Msg::Context(Target::Message(1)), &mut ctx);
    assert_eq!(client.menu, Some((Target::Message(1), Point::new(40.0, 60.0))));
    let _ = client.update(Msg::Pick(1), &mut ctx);
    assert!(client.menu.is_none() && client.selecting && client.selected.contains(&1));
}

#[test]
fn a_chat_is_hidden_at_once_and_shown_again_if_the_server_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    let _ = client.update(Msg::Context(Target::Chat(20)), &mut ctx);
    let _ = client.update(Msg::Hide(20, true), &mut ctx);
    assert!(client.menu.is_none());
    assert_eq!(client.counts(), (3, 1, 2), "hidden before the server answered");
    // A list read meanwhile still says "shown"; the choice made here stands.
    let _ = client.update(Msg::Chats(Ok(page(vec![chat(-10, "Касса чеки", "supergroup"), chat(20, "Платёжный бот", "bot")]))), &mut ctx);
    assert!(client.hidden(client.chat(20).unwrap()));
    let _ = client.update(Msg::Hidden(20, true, Ok(())), &mut ctx);
    assert!(client.hidden(client.chat(20).unwrap()));
    assert_eq!(ctx.toasts.pop(), Some((Tone::Success, "Чат скрыт".to_owned())));

    let _ = client.update(Msg::Hide(20, false), &mut ctx);
    assert!(!client.hidden(client.chat(20).unwrap()));
    let _ = client.update(Msg::Hidden(20, false, Err("Failed to update hidden flag".into())), &mut ctx);
    assert!(client.hidden(client.chat(20).unwrap()), "a refused change is taken back");
    assert_eq!(ctx.toasts.pop(), Some((Tone::Danger, "Failed to update hidden flag".to_owned())));
}

#[test]
fn a_protected_chat_asks_for_its_password_before_its_messages() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    let locked = || Err(Refusal { locked: true, barred: false, text: "chat_locked".into() });
    let _ = client.update(Msg::Messages(-10, client.opening, locked()), &mut ctx);
    assert!(client.lock.is_some() && client.messages.is_empty());
    let _ = client.poll(&ctx, Instant::now() + MESSAGES_EVERY);
    assert!(!client.messages_busy, "a locked chat is not asked for again until it is opened");

    let _ = client.update(Msg::Unlock, &mut ctx);
    assert!(!client.lock.as_ref().unwrap().busy, "no password, no question");
    let _ = client.update(Msg::LockInput("не тот".into()), &mut ctx);
    let _ = client.update(Msg::Unlock, &mut ctx);
    assert!(client.lock.as_ref().unwrap().busy);
    let wrong = TgUnlock { verified: false, attempts_left: Some(2), locked: false, locked_until: None };
    let _ = client.update(Msg::Unlocked(-10, Ok(wrong)), &mut ctx);
    let form = client.lock.as_ref().unwrap();
    assert!(!form.busy && form.password.is_empty());
    assert_eq!(form.error.as_deref(), Some("Неверный пароль (осталось попыток: 2)"));

    let _ = client.update(Msg::LockInput("тот".into()), &mut ctx);
    let _ = client.update(Msg::Unlock, &mut ctx);
    let _ = client.update(Msg::Unlocked(-10, Ok(TgUnlock { verified: true, ..TgUnlock::default() })), &mut ctx);
    assert!(client.lock.is_none() && client.messages_busy, "the messages are asked for with the pass");
    // Too many wrong tries: the server bars the chat for a while and says so.
    let _ = client.update(Msg::Messages(-10, client.opening, Err(Refusal { locked: true, barred: true, text: "Запрос отклонён (423)".into() })), &mut ctx);
    assert!(client.lock.as_ref().unwrap().error.as_deref().is_some_and(|reason| reason.contains("временно заблокирован")));
    // Another chat has its own lock, or none.
    let _ = client.update(Msg::Open(20), &mut ctx);
    assert!(client.lock.is_none());
}

#[test]
fn a_chat_password_is_checked_before_it_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    let _ = client.update(Msg::AskPassword(-10, PasswordMode::Set), &mut ctx);
    let said = |client: &Client| client.password.as_ref().unwrap().error.clone();
    assert_eq!(client.password.as_ref().unwrap().title, "Касса чеки");
    let _ = client.update(Msg::PasswordSubmit, &mut ctx);
    assert_eq!(said(&client).as_deref(), Some("Введите пароль"));
    let _ = client.update(Msg::PasswordInput("абв".into()), &mut ctx);
    let _ = client.update(Msg::PasswordSubmit, &mut ctx);
    assert_eq!(said(&client).as_deref(), Some("Минимум 4 символа"));
    let _ = client.update(Msg::PasswordInput("абвг".into()), &mut ctx);
    let _ = client.update(Msg::PasswordConfirm("абвд".into()), &mut ctx);
    let _ = client.update(Msg::PasswordSubmit, &mut ctx);
    assert_eq!(said(&client).as_deref(), Some("Пароли не совпадают"));
    assert!(!client.password.as_ref().unwrap().busy);
    let _ = client.update(Msg::PasswordConfirm("абвг".into()), &mut ctx);
    let _ = client.update(Msg::PasswordSubmit, &mut ctx);
    assert!(client.password.as_ref().unwrap().busy && said(&client).is_none());
    // While it is being saved the dialog neither closes nor changes.
    let _ = client.update(Msg::ClosePassword, &mut ctx);
    assert!(client.escape() && client.password.is_some());
    let _ = client.update(Msg::PasswordDone(PasswordMode::Set, Ok(())), &mut ctx);
    assert!(client.password.is_none());
    assert_eq!(ctx.toasts.pop(), Some((Tone::Success, "Пароль установлен".to_owned())));

    // Taking the protection off asks for the password once, and a wrong one is said in the dialog.
    let _ = client.update(Msg::AskPassword(-10, PasswordMode::Remove), &mut ctx);
    let _ = client.update(Msg::PasswordInput("абв".into()), &mut ctx);
    let _ = client.update(Msg::PasswordSubmit, &mut ctx);
    assert!(client.password.as_ref().unwrap().busy, "the length rule is for a new password only");
    let _ = client.update(Msg::PasswordDone(PasswordMode::Remove, Err("invalid_password".into())), &mut ctx);
    assert_eq!(said(&client).as_deref(), Some("invalid_password"));
    let _ = client.update(Msg::ClosePassword, &mut ctx);
    assert!(client.password.is_none());
}

#[test]
fn monitoring_is_switched_and_said() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    let _ = client.update(Msg::Chats(Ok(page(vec![chat(-10, "Касса чеки", "supergroup")]))), &mut ctx);
    let _ = client.update(Msg::Monitor(-10, true), &mut ctx);
    assert!(!client.chat(-10).unwrap().monitor_enabled, "not before the server agreed");
    let _ = client.update(Msg::Monitored(-10, true, Ok(())), &mut ctx);
    assert!(client.chat(-10).unwrap().monitor_enabled && client.chat(-10).unwrap().is_monitored);
    assert_eq!(ctx.toasts.pop(), Some((Tone::Success, "Мониторинг включён".to_owned())));
    let _ = client.update(Msg::Monitored(-10, false, Err("source_toggle_forbidden".into())), &mut ctx);
    assert!(client.chat(-10).unwrap().monitor_enabled);
    assert_eq!(ctx.toasts.pop(), Some((Tone::Danger, "source_toggle_forbidden".to_owned())));
}

#[test]
fn the_buttons_under_a_message_do_what_their_kind_says() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    let key = |kind: &str, words: &str, url: Option<&str>| TgButton { text: words.into(), kind: kind.into(), url: url.map(str::to_owned) };
    let rows = vec![
        vec![key("web_app", "Магазин", Some("https://shop.example.com/")), key("url", "Канал", Some("https://t.me/channel"))],
        vec![key("copy_text", "Код 4410", None), key("callback", "Обновить", None), key("web_app", "Пустая", None)],
    ];
    let with_keys = TgMessage { sender_id: Some(json!({ "user_id": 77 })), reply_markup: Some(TgMarkup { kind: "inline".into(), rows }), ..message(4, "2026-11-11T10:00:00", "выберите") };
    let _ = client.update(Msg::Messages(-10, client.opening, messages(vec![with_keys])), &mut ctx);

    let _ = client.update(Msg::Key(4, 0, 0), &mut ctx);
    let _ = client.update(Msg::Key(4, 0, 1), &mut ctx);
    assert_eq!(
        client.wants,
        [
            Want::App(AppRequest { chat: -10, bot: Some(77), url: "https://shop.example.com/".into(), source: TgButtonSource::Inline, text: "Магазин".into() }),
            Want::Link("https://t.me/channel".into()),
        ]
    );
    client.wants.clear();
    let _ = client.update(Msg::Key(4, 1, 0), &mut ctx);
    assert_eq!(client.copied, Some((4, 1, 0)));
    let _ = client.update(Msg::CopiedOff(4, 0, 0), &mut ctx);
    assert!(client.copied.is_some(), "another button's mark is not this one's to clear");
    let _ = client.update(Msg::CopiedOff(4, 1, 0), &mut ctx);
    assert!(client.copied.is_none());
    // A button that needs the bot's answer, one without an address, one that is not there.
    for (row, col) in [(1, 1), (1, 2), (5, 5)] {
        let _ = client.update(Msg::Key(4, row, col), &mut ctx);
    }
    let _ = client.update(Msg::Key(99, 0, 0), &mut ctx);
    assert!(client.wants.is_empty());
}

#[test]
fn a_bot_s_menu_app_is_offered_when_it_has_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    let _ = client.update(Msg::Open(20), &mut ctx);
    assert!(client.menu_app(20).is_none(), "asked for, not known yet");
    let _ = client.update(Msg::MenuApp, &mut ctx);
    assert!(client.wants.is_empty());
    let button = TgMenuButton { available: true, text: None, url: Some("https://shop.example.com/menu".into()), is_main: false, bot_user_id: Some(20) };
    let _ = client.update(Msg::MenuButton(20, Ok(button)), &mut ctx);
    assert!(client.menu_app(20).is_some());
    let _ = client.update(Msg::MenuApp, &mut ctx);
    assert_eq!(client.wants, [Want::App(AppRequest { chat: 20, bot: Some(20), url: "https://shop.example.com/menu".into(), source: TgButtonSource::Menu, text: "Открыть приложение".into() })]);
    // A bot without one, and a button without an address, offer nothing.
    let _ = client.update(Msg::MenuButton(20, Ok(TgMenuButton { available: true, url: None, ..TgMenuButton::default() })), &mut ctx);
    assert!(client.menu_app(20).is_none());
    assert!(client.menu_app(-10).is_none());
}

#[test]
fn photos_are_fetched_for_the_open_chat_and_open_at_full_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    let photo = |id: i64, file: i64| TgMessage { photo: Some(TgFile { file_id: Some(file), file_name: Some("photo.jpg".into()), ..TgFile::default() }), ..message(id, "2026-11-11T10:00:00", "[Photo]") };
    let items: Vec<TgMessage> = (1..=6).map(|id| photo(id, 100 + id)).collect();
    let _ = client.update(Msg::Messages(-10, client.opening, messages(items.clone())), &mut ctx);
    assert_eq!(client.photos.len(), 6);
    assert_eq!(client.photo_flights, PHOTOS_AT_ONCE, "a few at a time");
    let key = (-10, 6, 106);
    let _ = client.update(Msg::ViewPhoto(key), &mut ctx);
    assert!(client.viewer.is_none(), "nothing to open before it has come");
    let _ = client.update(Msg::Photo(key, Ok(image::Handle::from_bytes(vec![1, 2, 3]))), &mut ctx);
    assert!(matches!(client.photos.get(&key), Some(Photo::Ready(_))));
    assert_eq!(client.photo_flights, PHOTOS_AT_ONCE, "the next one took the free place");
    let _ = client.update(Msg::Photo((-10, 5, 105), Err("нет".into())), &mut ctx);
    assert!(matches!(client.photos.get(&(-10, 5, 105)), Some(Photo::Failed)));
    let _ = client.update(Msg::ViewPhoto(key), &mut ctx);
    assert_eq!(client.viewer, Some(key));
    assert!(client.layered() && client.escape() && client.viewer.is_none());
    // In selection mode a press on a photo ticks its message instead.
    let _ = client.update(Msg::ToggleSelecting, &mut ctx);
    let _ = client.update(Msg::ViewPhoto(key), &mut ctx);
    assert!(client.viewer.is_none() && client.selected.contains(&6));
    // The same list read again fetches nothing twice; another chat drops them all.
    let before = client.photo_flights;
    let _ = client.update(Msg::Messages(-10, client.opening, messages(items)), &mut ctx);
    assert_eq!((client.photos.len(), client.photo_flights), (6, before));
    let _ = client.update(Msg::Open(20), &mut ctx);
    assert!(client.photos.is_empty());
    let _ = client.update(Msg::Photo((-10, 4, 104), Ok(image::Handle::from_bytes(vec![1]))), &mut ctx);
    assert!(client.photos.is_empty(), "a photo of the chat that was left is not kept");
}

#[test]
fn a_file_is_saved_once_at_a_time_and_ticks_in_selection_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = working(&mut ctx);
    let file = TgMessage { document: Some(TgFile { file_id: Some(21), file_name: Some("чек.pdf".into()), ..TgFile::default() }), ..message(2, "2026-11-11T10:00:00", "чек.pdf") };
    let _ = client.update(Msg::Messages(-10, client.opening, messages(vec![file, message(3, "2026-11-11T10:01:00", "без файла")])), &mut ctx);
    let _ = client.update(Msg::Document(3), &mut ctx);
    assert_eq!(client.saving, None, "a message without a file has nothing to save");
    let _ = client.update(Msg::Document(2), &mut ctx);
    assert_eq!(client.saving, Some(2));
    let _ = client.update(Msg::Saved(Ok(Some("C:\\Чеки\\чек.pdf".into()))), &mut ctx);
    assert_eq!(client.saving, None);
    assert_eq!(ctx.toasts.pop(), Some((Tone::Success, "Файл сохранён: C:\\Чеки\\чек.pdf".to_owned())));
    let _ = client.update(Msg::Saved(Ok(None)), &mut ctx);
    assert!(ctx.toasts.is_empty());
    let _ = client.update(Msg::ToggleSelecting, &mut ctx);
    let _ = client.update(Msg::Document(2), &mut ctx);
    assert!(client.saving.is_none() && client.selected.contains(&2));
}

#[test]
fn the_way_back_to_the_newest_message_is_offered_when_scrolled_away() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ctx(&dir);
    let mut client = reading(&mut ctx);
    let _ = client.update(Msg::Scrolled(40.0), &mut ctx);
    assert!(!client.away);
    let _ = client.update(Msg::Scrolled(300.0), &mut ctx);
    assert!(client.away);
    let _ = client.update(Msg::ToBottom, &mut ctx);
    assert!(!client.away);
}
