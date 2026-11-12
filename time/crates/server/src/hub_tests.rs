//! Режим общих учётных записей: бот, API и расчёт месяца.
//!
//! Telegram и сервер учётных записей здесь подставные (два маленьких сервера на
//! локальных портах), всё остальное — настоящая программа: цикл опроса, база,
//! разбор сообщений, расчёт. Подставной Telegram ведёт себя как настоящий в том,
//! что здесь важно: обновление приходит снова, пока не подтверждено следующим
//! `offset`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::NaiveDate;
use clap::Parser;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::config::Config;
use crate::db::{Db, Employee};
use crate::service::Engine;
use crate::{api, bot};

const HUB_KEY: &str = "hub-key-0123456789abcdef-test";
const API_TOKEN: &str = "api-token-0123456789abcdef-test";
const GROUP: i64 = -100123;
const FOREIGN_GROUP: i64 = -100999;

#[derive(Clone, Default)]
struct Telegram {
    updates: Arc<Mutex<Vec<Value>>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

impl Telegram {
    fn push(&self, update: Value) {
        self.updates.lock().unwrap().push(update);
    }

    fn calls(&self, method: &str) -> Vec<Value> {
        let calls = self.calls.lock().unwrap();
        calls
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, body)| body.clone())
            .collect()
    }

    /// Ответы бота: (чат, текст).
    fn replies(&self) -> Vec<(i64, String)> {
        self.calls("sendMessage")
            .iter()
            .map(|b| {
                (
                    b["chat_id"].as_i64().unwrap(),
                    b["text"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    /// Реакции бота: (сообщение, эмодзи).
    fn reactions(&self) -> Vec<(i64, String)> {
        self.calls("setMessageReaction")
            .iter()
            .map(|b| {
                (
                    b["message_id"].as_i64().unwrap(),
                    b["reaction"][0]["emoji"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    /// Ждёт, пока бот подтвердит всё до `offset` следующим опросом.
    async fn confirmed(&self, offset: i64) {
        for _ in 0..600 {
            let done = self
                .calls("getUpdates")
                .iter()
                .any(|b| b["offset"].as_i64() >= Some(offset));
            if done {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("бот не дошёл до позиции {offset}");
    }
}

async fn telegram_method(
    State(tg): State<Telegram>,
    Path((_bot, method)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Json<Value> {
    tg.calls.lock().unwrap().push((method.clone(), body.clone()));
    let result = match method.as_str() {
        "getMe" => json!({ "id": 1, "is_bot": true, "first_name": "Тайм-лист", "username": "timelist_test_bot" }),
        "getUpdates" => {
            let offset = body["offset"].as_i64().unwrap_or(0);
            let pending: Vec<Value> = {
                let updates = tg.updates.lock().unwrap();
                updates
                    .iter()
                    .filter(|u| u["update_id"].as_i64().unwrap() >= offset)
                    .cloned()
                    .collect()
            };
            if pending.is_empty() {
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
            json!(pending)
        }
        "setMessageReaction" => json!(true),
        _ => json!({ "message_id": 999 }),
    };
    Json(json!({ "ok": true, "result": result }))
}

/// Подставной сервер учётных записей: отвечает то, что ему велели.
#[derive(Clone, Default)]
struct Accounts {
    verdicts: Arc<Mutex<HashMap<i64, &'static str>>>,
    down: Arc<AtomicBool>,
    asked: Arc<Mutex<Vec<i64>>>,
}

impl Accounts {
    fn set(&self, tg_user_id: i64, reason: &'static str) {
        self.verdicts.lock().unwrap().insert(tg_user_id, reason);
    }

    fn asked(&self) -> Vec<i64> {
        self.asked.lock().unwrap().clone()
    }
}

async fn accounts_answer(
    State(hub): State<Accounts>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if headers.get("x-time-hub-key").and_then(|v| v.to_str().ok()) != Some(HUB_KEY) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "detail": { "error": "time_hub_key_refused" } })),
        );
    }
    if hub.down.load(Ordering::SeqCst) {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})));
    }
    hub.asked.lock().unwrap().push(id);
    let reason = hub
        .verdicts
        .lock()
        .unwrap()
        .get(&id)
        .copied()
        .unwrap_or("not_linked");
    (
        StatusCode::OK,
        Json(json!({ "allowed": reason == "ok", "reason": reason })),
    )
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

struct World {
    tg: Telegram,
    hub: Accounts,
    state: AppState,
}

/// Программа в режиме общих учётных записей, подключённая к подставным серверам.
async fn world(db: Db, chat_id: Option<&str>) -> World {
    let tg = Telegram::default();
    let hub = Accounts::default();
    let tg_base = serve(
        Router::new()
            .route("/{bot}/{method}", post(telegram_method))
            .with_state(tg.clone()),
    )
    .await;
    let hub_base = serve(
        Router::new()
            .route("/internal/time/telegram/{id}", get(accounts_answer))
            .with_state(hub.clone()),
    )
    .await;
    let mut args = vec![
        "timelist".to_string(),
        "--bot-token".into(),
        "TEST:SECRET".into(),
        "--telegram-api".into(),
        tg_base,
        "--hub-url".into(),
        hub_base,
        "--hub-key".into(),
        HUB_KEY.into(),
        "--api-token".into(),
        API_TOKEN.into(),
    ];
    if let Some(chat_id) = chat_id {
        args.push(format!("--chat-id={chat_id}"));
    }
    let cfg = Config::parse_from(args).normalized();
    cfg.check().unwrap();
    World {
        tg,
        hub,
        state: AppState::new(db, cfg),
    }
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()
}

/// Время сообщения: 2 ноября 2026, часы и минуты по Ташкенту (UTC+5).
fn at(hour: u32, minute: u32) -> i64 {
    day()
        .and_hms_opt(hour - 5, minute, 0)
        .unwrap()
        .and_utc()
        .timestamp()
}

fn user(id: i64, first: &str, last: &str) -> Value {
    json!({ "id": id, "is_bot": false, "first_name": first, "last_name": last, "username": format!("u{id}") })
}

fn group(id: i64) -> Value {
    json!({ "id": id, "type": "supergroup", "title": "Рабочая группа" })
}

fn private(id: i64) -> Value {
    json!({ "id": id, "type": "private" })
}

fn message(update_id: i64, message_id: i64, chat: Value, from: Value, date: i64, text: &str) -> Value {
    json!({ "update_id": update_id, "message": { "message_id": message_id, "date": date, "chat": chat, "from": from, "text": text } })
}

fn employee(db: &Db, full_name: &str, nick: &str, tg_name: &str, tg_user_id: Option<i64>) -> i64 {
    db.insert_employee(&Employee {
        id: 0,
        full_name: full_name.into(),
        nick: nick.into(),
        tg_name: tg_name.into(),
        tg_user_id,
        aliases: String::new(),
        auto_schedule: false,
        active: true,
        sort: 0,
        works_from: None,
        works_to: None,
    })
    .unwrap()
}

fn texts(state: &AppState) -> Vec<String> {
    let stored = state.db.messages_between(day(), day()).unwrap();
    stored.into_iter().map(|m| m.text).collect()
}

/// Начало дня 2 ноября у сотрудника, как его показывает тайм-лист.
fn start_of(state: &AppState, employee_id: i64) -> Option<i32> {
    let view = state.engine().unwrap().month(2026, 11, day()).unwrap();
    let row = view.employees.iter().find(|e| e.id == employee_id).unwrap();
    row.cells[1].m.start
}

#[tokio::test]
async fn only_accounts_confirmed_right_now_mark_attendance() {
    let db = Db::in_memory().unwrap();
    let oleg = employee(&db, "Сомов О.А.", "ОлС", "", Some(111));
    let anna = employee(&db, "Волкова А.И.", "АнВ", "", Some(333));
    let w = world(db, Some("-100123")).await;
    w.hub.set(111, "ok");
    // Анна привязана к сотруднику, но её учётная запись отключена.
    w.hub.set(333, "inactive");

    let (o, a, n) = (user(111, "Олег", "Сомов"), user(333, "Анна", "Волкова"), user(222, "Новый", "Человек"));
    w.tg.push(message(10, 1, group(GROUP), o.clone(), at(9, 0), "Офис"));
    w.tg.push(message(11, 2, group(GROUP), n.clone(), at(9, 5), "Офис"));
    w.tg.push(message(12, 3, group(GROUP), a.clone(), at(9, 10), "Офис"));
    w.tg.push(message(13, 4, group(FOREIGN_GROUP), o.clone(), at(9, 15), "Офис"));
    w.tg.push(message(14, 5, private(333), a.clone(), at(9, 20), "/me"));
    w.tg.push(message(15, 6, group(GROUP), o.clone(), at(9, 25), "/me"));
    w.tg.push(message(16, 7, group(FOREIGN_GROUP), o.clone(), at(9, 30), "/me"));
    w.tg.push(message(17, 8, private(222), n.clone(), at(9, 35), "/me"));

    let running = tokio::spawn(bot::run(w.state.clone()));
    w.tg.confirmed(18).await;
    running.abort();

    // В базе — только сообщение подтверждённой учётной записи.
    assert_eq!(texts(&w.state), ["Офис"]);
    assert_eq!(start_of(&w.state, oleg), Some(540));
    assert_eq!(start_of(&w.state, anna), None, "отключённая запись отметок не ставит");
    assert_eq!(
        w.tg.reactions(),
        [(1, "✍".to_string()), (2, "🤷".to_string()), (3, "🤷".to_string())]
    );

    // /me: отчёт — своему, причина — тому, кому отказано, тишина — посторонней группе.
    let replies = w.tg.replies();
    assert_eq!(replies.len(), 3, "{replies:?}");
    assert_eq!(replies[0].0, 333);
    assert!(replies[0].1.contains("учётная запись отключена"), "{}", replies[0].1);
    assert_eq!(replies[1].0, GROUP);
    assert!(replies[1].1.starts_with("ОлС"), "{}", replies[1].1);
    assert_eq!(replies[2].0, 222);
    assert!(replies[2].1.contains("Telegram ID 222 не привязан к учётной записи"), "{}", replies[2].1);

    // Сервер учётных записей спрошен о каждом, кроме написавших в постороннюю группу.
    assert_eq!(w.hub.asked(), [111, 222, 333, 333, 111, 222]);

    // Кому отказано, тот виден в «Привязке сотрудников» — по нему находят ID для учётной записи.
    let seen = w.state.db.tg_users().unwrap();
    assert!(seen.iter().any(|u| u.tg_user_id == 222 && u.name == "Новый Человек"));
    let status = w.state.bot_status(|s| s.clone());
    assert!(status.accounts_required);
    assert_eq!(status.denied, 2);
    assert!(status.last_denied.unwrap().contains("Анна Волкова · учётная запись отключена"));
}

#[tokio::test]
async fn a_change_of_the_account_reaches_the_very_next_message() {
    let db = Db::in_memory().unwrap();
    let oleg = employee(&db, "Сомов О.А.", "ОлС", "", Some(111));
    let w = world(db, Some("-100123")).await;
    w.hub.set(111, "ok");
    let o = user(111, "Олег", "Сомов");
    let running = tokio::spawn(bot::run(w.state.clone()));

    w.tg.push(message(10, 1, group(GROUP), o.clone(), at(9, 0), "Офис"));
    w.tg.confirmed(11).await;
    assert_eq!(texts(&w.state), ["Офис"]);

    // Учётную запись отключили: следующее сообщение не принято, ничего не запомнено.
    w.hub.set(111, "inactive");
    w.tg.push(message(11, 2, group(GROUP), o.clone(), at(18, 0), "Уход"));
    // Правка уже принятого сообщения от отключённой записи тоже не проходит.
    w.tg.push(json!({ "update_id": 12, "edited_message": { "message_id": 1, "date": at(9, 0), "chat": group(GROUP), "from": o.clone(), "text": "Офис 7:00" } }));
    w.tg.confirmed(13).await;
    assert_eq!(texts(&w.state), ["Офис"]);
    assert_eq!(start_of(&w.state, oleg), Some(540));

    // Доступ к учёту времени отозван — то же самое.
    w.hub.set(111, "no_time_access");
    w.tg.push(message(13, 3, group(GROUP), o.clone(), at(18, 5), "Уход"));
    w.tg.confirmed(14).await;
    assert_eq!(texts(&w.state), ["Офис"]);

    // Вернули — принимается снова, без перезапуска.
    w.hub.set(111, "ok");
    w.tg.push(message(14, 4, group(GROUP), o, at(18, 30), "Уход"));
    w.tg.confirmed(15).await;
    running.abort();
    assert_eq!(texts(&w.state), ["Офис", "Уход"]);
    assert_eq!(w.state.bot_status(|s| s.denied), 3);
}

#[tokio::test]
async fn no_answer_from_the_accounts_server_neither_accepts_nor_loses_a_mark() {
    let db = Db::in_memory().unwrap();
    let oleg = employee(&db, "Сомов О.А.", "ОлС", "", Some(111));
    let w = world(db, Some("-100123")).await;
    w.hub.set(111, "ok");
    w.hub.down.store(true, Ordering::SeqCst);
    let o = user(111, "Олег", "Сомов");
    w.tg.push(message(10, 1, group(GROUP), o.clone(), at(9, 0), "Офис"));
    w.tg.push(message(11, 2, group(GROUP), o, at(9, 1), "/id"));

    let running = tokio::spawn(bot::run(w.state.clone()));
    // Бот спрашивает Telegram снова и снова с той же позиции.
    for _ in 0..600 {
        if w.tg.calls("getUpdates").len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(w.tg.calls("getUpdates").iter().all(|b| b["offset"] == 0));
    assert!(texts(&w.state).is_empty(), "без ответа сервера отметка не принимается");
    assert_eq!(w.state.db.setting("tg_offset").unwrap(), None);
    // Порядок сохраняется: команда, пришедшая после, тоже ждёт.
    assert!(w.tg.replies().is_empty());
    let error = w.state.bot_status(|s| s.last_error.clone()).unwrap();
    assert!(error.contains("сервер учётных записей недоступен"), "{error}");
    assert!(!error.contains(HUB_KEY), "ключ не попадает в сообщение об ошибке");

    // Сервер вернулся: то же сообщение принято, один раз.
    w.hub.down.store(false, Ordering::SeqCst);
    w.tg.confirmed(12).await;
    running.abort();
    assert_eq!(texts(&w.state), ["Офис"]);
    assert_eq!(start_of(&w.state, oleg), Some(540));
    assert_eq!(w.tg.replies().len(), 1);
    assert_eq!(w.state.bot_status(|s| s.last_error.clone()), None);
}

#[tokio::test]
async fn a_restart_or_a_repeated_update_does_not_count_twice() {
    let db = Db::in_memory().unwrap();
    let oleg = employee(&db, "Сомов О.А.", "ОлС", "", Some(111));
    let w = world(db, Some("-100123")).await;
    w.hub.set(111, "ok");
    let o = user(111, "Олег", "Сомов");
    w.tg.push(message(10, 1, group(GROUP), o.clone(), at(9, 0), "Офис"));
    w.tg.push(message(11, 2, group(GROUP), o.clone(), at(9, 1), "/id"));

    let first = tokio::spawn(bot::run(w.state.clone()));
    w.tg.confirmed(12).await;
    first.abort();
    assert_eq!(w.state.db.setting("tg_offset").unwrap().as_deref(), Some("12"));
    assert_eq!((texts(&w.state).len(), w.tg.replies().len()), (1, 1));

    // Перезапуск: позиция прочитана из базы, старое не запрашивается и не повторяется.
    let asked_before = w.tg.calls("getUpdates").len();
    let second = tokio::spawn(bot::run(w.state.clone()));
    for _ in 0..600 {
        if w.tg.calls("getUpdates").len() > asked_before + 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    second.abort();
    assert!(w.tg.calls("getUpdates")[asked_before..].iter().all(|b| b["offset"] == 12));
    assert_eq!((texts(&w.state).len(), w.tg.replies().len()), (1, 1));

    // Сбой между обработкой и записью позиции: Telegram отдаёт то же самое ещё раз.
    // Сообщение узнаётся по (чат, номер) и второй строкой не становится.
    w.state.db.set_setting("tg_offset", "10").unwrap();
    // То же сообщение под новым номером обновления и его правка.
    w.tg.push(message(12, 1, group(GROUP), o.clone(), at(9, 0), "Офис"));
    w.tg.push(json!({ "update_id": 13, "edited_message": { "message_id": 1, "date": at(9, 0), "chat": group(GROUP), "from": o, "text": "Офис, до банк" } }));
    let third = tokio::spawn(bot::run(w.state.clone()));
    w.tg.confirmed(14).await;
    third.abort();

    let stored = w.state.db.messages_between(day(), day()).unwrap();
    assert_eq!(stored.len(), 1, "повтор не добавляет строк");
    assert_eq!((stored[0].text.as_str(), stored[0].edited), ("Офис, до банк", true));
    assert_eq!(start_of(&w.state, oleg), Some(540));
    // Ответ на команду при таком сбое повторяется: он отправляется до записи позиции.
    assert_eq!(w.tg.replies().len(), 2);
}

#[tokio::test]
async fn an_empty_group_list_accepts_no_group_at_all() {
    let db = Db::in_memory().unwrap();
    employee(&db, "Сомов О.А.", "ОлС", "", Some(111));
    let w = world(db, None).await;
    w.hub.set(111, "ok");
    let o = user(111, "Олег", "Сомов");
    w.tg.push(message(10, 1, group(GROUP), o.clone(), at(9, 0), "Офис"));
    w.tg.push(message(11, 2, group(GROUP), o.clone(), at(9, 1), "/me"));
    // Команда /id нужна как раз затем, чтобы узнать ID группы и внести его в настройки.
    w.tg.push(message(12, 3, group(GROUP), o, at(9, 2), "/id"));

    let running = tokio::spawn(bot::run(w.state.clone()));
    w.tg.confirmed(13).await;
    running.abort();

    assert!(texts(&w.state).is_empty());
    assert!(w.state.db.tg_users().unwrap().is_empty());
    assert!(w.hub.asked().is_empty(), "о посторонней группе сервер учётных записей не спрашивают");
    assert_eq!(
        w.tg.replies(),
        [(GROUP, "Ваш Telegram ID: 111\nID этого чата: -100123".to_string())]
    );
}

#[tokio::test]
async fn a_chat_name_proves_nothing_once_accounts_decide() {
    let db = Db::in_memory().unwrap();
    // Сотрудник из справочника: имя в Telegram известно, ID ещё не привязан.
    let oleg = employee(&db, "Сомов О.А.", "ОлС", "Олег Сомов", None);
    let w = world(db, Some("-100123")).await;
    // У написавшего есть учётная запись с доступом, и назвался он именем Олега.
    w.hub.set(555, "ok");
    w.tg.push(message(10, 1, group(GROUP), user(555, "Олег", "Сомов"), at(9, 0), "Офис"));

    let running = tokio::spawn(bot::run(w.state.clone()));
    w.tg.confirmed(11).await;
    running.abort();

    // Сообщение сохранено и ждёт привязки, но отметок Олега не меняет.
    assert_eq!(texts(&w.state), ["Офис"]);
    assert_eq!(start_of(&w.state, oleg), None);
    assert_eq!(w.tg.reactions(), [(1, "🤷".to_string())]);
    // Без общих учётных записей то же сообщение засчиталось бы по имени — прежнее поведение
    // программы, оно не тронуто.
    let standalone = Engine::new(&w.state.db).unwrap().month(2026, 11, day()).unwrap();
    let row = standalone.employees.iter().find(|e| e.id == oleg).unwrap();
    assert_eq!(row.cells[1].m.start, Some(540));

    // Руководитель привязал ID к сотруднику — отметка появилась задним числом.
    w.state.db.link_tg_user(oleg, Some(555)).unwrap();
    assert_eq!(start_of(&w.state, oleg), Some(540));
}

#[tokio::test]
async fn the_api_answers_only_to_the_gateway_token() {
    let db = Db::in_memory().unwrap();
    let oleg = employee(&db, "Сомов О.А.", "ОлС", "", None);
    let w = world(db, Some("-100123")).await;
    let base = serve(api::router(w.state.clone())).await;
    let http = reqwest::Client::new();
    let get = |path: &str| http.get(format!("{base}{path}"));

    // Без токена видно только то, что нужно проверке живости.
    let meta: Value = get("/api/meta").send().await.unwrap().json().await.unwrap();
    assert_eq!((meta["hub"].as_bool(), meta["authed"].as_bool()), (Some(true), Some(false)));
    assert!(meta.get("revision").is_none() && meta.get("bot").is_none());

    // Входа по паролю программы нет — ни с паролем, ни с самим токеном шлюза.
    for password in ["anything", API_TOKEN] {
        let login = http
            .post(format!("{base}/api/login"))
            .json(&json!({ "password": password }))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), 404);
        assert!(login.headers().get("set-cookie").is_none());
    }
    assert_eq!(get("/api/month/2026-11").send().await.unwrap().status(), 401);
    let with_cookie = get("/api/month/2026-11").header("cookie", "tl_session=anything");
    assert_eq!(with_cookie.send().await.unwrap().status(), 401);
    let wrong = get("/api/month/2026-11").bearer_auth("not-the-token-0123456789abcdef");
    assert_eq!(wrong.send().await.unwrap().status(), 401);
    assert_eq!(get("/api/month/2026-11").bearer_auth(API_TOKEN).send().await.unwrap().status(), 200);

    // Число изменений растёт с каждой правкой: по нему интерфейс узнаёт о переменах.
    let revision = || async {
        let meta: Value = get("/api/meta").bearer_auth(API_TOKEN).send().await.unwrap().json().await.unwrap();
        meta["revision"].as_u64().unwrap()
    };
    let before = revision().await;
    let edit = http
        .put(format!("{base}/api/marks/{oleg}/2026-11-02"))
        .bearer_auth(API_TOKEN)
        // Имя для журнала ставит шлюз; здесь — как он его передаёт.
        .header("x-timelist-user", "%D0%90%D0%BD%D0%BD%D0%B0%20(anna)")
        .json(&json!({ "set": { "start": "9:30" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(edit.status(), 200);
    assert_eq!(revision().await, before + 1);
    let journal: Value = get("/api/audit?limit=5").bearer_auth(API_TOKEN).send().await.unwrap().json().await.unwrap();
    assert_eq!(journal[0]["user"], "Анна (anna)");
}

#[test]
fn half_a_hub_configuration_does_not_start() {
    let parse = |args: &[&str]| {
        let mut all = vec!["timelist"];
        all.extend_from_slice(args);
        Config::parse_from(all).normalized()
    };
    // Сама по себе программа запускается как раньше.
    let standalone = parse(&[]);
    assert!(standalone.check().is_ok() && !standalone.hub_mode());

    let url = ["--hub-url", "http://backend:8000"];
    let key = ["--hub-key", HUB_KEY];
    let token = ["--api-token", API_TOKEN];
    assert!(parse(&url).check().unwrap_err().contains("TIMELIST_HUB_KEY"));
    assert!(parse(&key).check().unwrap_err().contains("TIMELIST_HUB_URL"));
    // Без токена API был бы открыт всем, кто видит порт.
    assert!(parse(&[url, key].concat()).check().unwrap_err().contains("TIMELIST_API_TOKEN"));
    assert!(parse(&[&url[..], &["--hub-key", "short"], &token[..]].concat()).check().is_err());
    assert!(parse(&[&url[..], &key[..], &["--api-token", "short"]].concat()).check().is_err());
    assert!(parse(&[&["--hub-url", "backend:8000"], &key[..], &token[..]].concat()).check().is_err());
    // Пустые значения из окружения — это «не задано».
    assert!(parse(&["--hub-url", " ", "--hub-key", ""]).check().is_ok());

    let full = parse(&[url, key, token].concat());
    assert!(full.check().is_ok() && full.hub_mode() && full.auth_required());
}
