//! Every write the client makes, against a real server.
//!
//! It creates, changes and deletes real records, so it refuses to run unless
//! `TBS_E2E_WRITES=1` is set and the server is on this machine. The login comes
//! from `TBS_E2E_LOGIN`, as in `live_backend.rs`.
//!
//! Replies are read through `#[serde(default)]`, so a reply of the wrong shape
//! parses into empty values without an error. Each check therefore looks at a
//! value the server must have filled in, not only at `is_ok()`.

use std::future::Future;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use serde_json::{Map, Value, json};
use sha1::Sha1;
use tbs_core::api::{Api, NewTransaction, NewUser};
use tbs_core::secret::{MemoryStore, SecretStore};
use tbs_core::store::Store;
use tbs_core::sync::{Mode, SyncResult, fetch_all, sync_transactions};

struct Creds {
    url: String,
    username: String,
    password: String,
}

fn creds() -> Option<Creds> {
    if std::env::var("TBS_E2E_WRITES").ok()?.as_str() != "1" {
        return None;
    }
    let path = std::env::var("TBS_E2E_LOGIN").ok()?;
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let url = std::env::var("TBS_E2E_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".into());
    assert!(url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost"), "writes run on a local server only, not {url}");
    Some(Creds { url, username: v["username"].as_str()?.into(), password: v["password"].as_str()?.into() })
}

fn stamp() -> String {
    chrono::Utc::now().format("%H%M%S%3f").to_string()
}

fn fields(value: Value) -> Map<String, Value> {
    value.as_object().expect("an object").clone()
}

fn new_row(operator: &str) -> NewTransaction {
    NewTransaction {
        datetime: "2024-03-15T10:20:30".into(),
        operator: operator.into(),
        amount: "125000.50".into(),
        card_last4: "1234".into(),
        transaction_type: "DEBIT".into(),
        currency: "UZS".into(),
        app: None,
        balance: Some("990000".into()),
        is_p2p: false,
        receiver_name: None,
        receiver_card: None,
        raw_text: Some("e2e".into()),
    }
}

/// Runs one group, so a failure reports itself and the others still run.
async fn group(failed: &mut Vec<&'static str>, name: &'static str, body: impl Future<Output = ()> + Send + 'static) {
    match tokio::spawn(body).await {
        Ok(()) => println!("ok    {name}"),
        Err(_) => {
            println!("FAIL  {name}");
            failed.push(name);
        }
    }
}

#[tokio::test]
async fn every_write_reaches_the_server() {
    let Some(c) = creds() else { return };
    let vault: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    let api = Api::new(c.url.clone(), None, vault);
    let reply = api.login(&c.username, &c.password).await.expect("login call");
    assert!(reply.ok, "login refused: {:?}", reply.error);

    let mut failed = Vec::new();
    group(&mut failed, "transactions: create, read, edit, clear", transactions_round_trip(api.clone())).await;
    group(&mut failed, "transactions: bulk edit and delete", transactions_in_bulk(api.clone())).await;
    group(&mut failed, "transactions: a locked period refuses edits", locked_period_refuses(api.clone())).await;
    group(&mut failed, "transactions: sync sees a change", sync_sees_a_change(api.clone())).await;
    group(&mut failed, "reference: operators", operators(api.clone())).await;
    group(&mut failed, "reference: operators file round trip", operators_file(api.clone())).await;
    group(&mut failed, "reference: articles and persons", dictionaries(api.clone())).await;
    group(&mut failed, "reference: descriptions", descriptions(api.clone())).await;
    group(&mut failed, "reference: annotation import and rules", annotations(api.clone())).await;
    group(&mut failed, "logs and audit", logs_and_audit(api.clone())).await;
    group(&mut failed, "settings: system switches", system_switches(api.clone())).await;
    group(&mut failed, "settings: periods", periods(api.clone())).await;
    group(&mut failed, "settings: folders", scopes(api.clone())).await;
    group(&mut failed, "excel: grants and workbook", excel(api.clone(), c.url.clone())).await;
    group(&mut failed, "gate: status, launch, qr", gates(api.clone())).await;
    group(&mut failed, "users: full lifecycle", users(api.clone(), c.url.clone())).await;
    group(&mut failed, "sessions: end one, end the others", sessions(api.clone(), c.url.clone(), c.username.clone(), c.password.clone())).await;
    assert!(failed.is_empty(), "failed groups: {failed:#?}");
}

async fn transactions_round_trip(api: Api) {
    let operator = format!("E2E SHOP {}", stamp());
    let created = api.create_transaction(&new_row(&operator)).await.expect("create");
    assert!(created.id > 0, "reply has no id: {created:?}");
    assert_eq!(created.operator_raw.as_deref(), Some(operator.as_str()));
    assert_eq!(created.currency.as_deref(), Some("UZS"));
    assert!(created.transaction_date.as_deref().unwrap_or_default().starts_with("2024-03-15"), "{:?}", created.transaction_date);
    let amount: f64 = created.amount.as_deref().unwrap_or("0").parse().expect("amount is a number");
    assert!((amount - 125_000.5).abs() < 0.001, "{amount}");

    let read = api.transaction(created.id).await.expect("read back");
    assert_eq!(read.id, created.id);
    assert_eq!(read.card_last_4.as_deref(), Some("1234"));

    // Codes are checked against the dictionaries and the server normalizes them.
    let tag = stamp();
    let person = api.create_person(&format!("П{tag}"), "E2E Лицо").await.expect("person");
    let article = api.create_article(&format!("С{tag}"), "e2e статья").await.expect("article");
    let changed = fields(json!({
        "description": "e2e описание",
        "receiver_name": "Получатель",
        "annotation_details": "деталь",
        "annotation_person_code": person.code,
        "annotation_period": "2024-03",
        "annotation_article_code": article.code,
    }));
    let reply = api.update_transaction(created.id, &changed).await.expect("edit");
    assert!(reply.ok || reply.success, "{reply:?}");
    let read = api.transaction(created.id).await.unwrap();
    assert_eq!(read.description.as_deref(), Some("e2e описание"));
    assert_eq!(read.receiver_name.as_deref(), Some("Получатель"));
    assert_eq!(read.annotation_details.as_deref(), Some("деталь"));
    assert_eq!(read.annotation_person_code.as_deref(), Some(person.code.as_str()));
    assert_eq!(read.annotation_article_code.as_deref(), Some(article.code.as_str()));

    // An emptied cell goes out as null and has to come back empty.
    let cleared = fields(json!({ "receiver_name": null, "annotation_details": null, "annotation_person_code": null, "annotation_article_code": null }));
    api.update_transaction(created.id, &cleared).await.expect("clear");
    let read = api.transaction(created.id).await.unwrap();
    assert_eq!(read.receiver_name.as_deref().unwrap_or(""), "", "receiver stayed");
    assert_eq!(read.annotation_details.as_deref().unwrap_or(""), "", "annotation stayed");
    assert_eq!(read.annotation_person_code.as_deref().unwrap_or(""), "", "person stayed");

    let reply = api.delete_transaction(created.id).await.expect("delete");
    assert!(reply.ok || reply.success, "{reply:?}");
    assert!(api.transaction(created.id).await.is_err(), "deleted row is still readable");
    api.delete_person(person.id).await.expect("delete person");
    api.delete_article(article.id).await.expect("delete article");
}

async fn transactions_in_bulk(api: Api) {
    let tag = stamp();
    let mut ids = Vec::new();
    for n in 0..3 {
        ids.push(api.create_transaction(&new_row(&format!("E2E BULK {tag} {n}"))).await.expect("create").id);
    }
    // A paste across columns: one field the bulk endpoint keeps, one it would lose.
    let updates: Vec<_> = ids.iter().map(|id| (*id, fields(json!({ "description": format!("bulk {id}"), "receiver_name": "Массово" })))).collect();
    let reply = api.bulk_update(&updates).await.expect("bulk edit");
    assert!(reply.failed_ids.is_empty(), "{reply:?}");
    let read = api.transaction(ids[1]).await.unwrap();
    assert_eq!(read.description, Some(format!("bulk {}", ids[1])));
    assert_eq!(read.receiver_name.as_deref(), Some("Массово"));

    // Undoing a description sends it back empty, which has to clear it.
    let undo = vec![(ids[1], fields(json!({ "description": "" })))];
    assert!(api.bulk_update(&undo).await.expect("undo").failed_ids.is_empty());
    assert_eq!(api.transaction(ids[1]).await.unwrap().description.unwrap_or_default(), "");

    let reply = api.bulk_delete(&ids).await.expect("bulk delete");
    assert_eq!(reply.deleted_count, 3, "{reply:?}");
    assert!(reply.failed_ids.is_empty(), "{reply:?}");
}

async fn locked_period_refuses(api: Api) {
    let row = api.create_transaction(&new_row(&format!("E2E LOCK {}", stamp()))).await.expect("create");
    let period = api.create_period("2024-03-15", "2024-03-15", Some("e2e")).await.expect("lock");
    assert!(period.id > 0 && period.is_active, "{period:?}");

    let edit = api.update_transaction(row.id, &fields(json!({ "description": "нельзя" }))).await;
    let delete = api.delete_transaction(row.id).await;
    let released = api.deactivate_period(period.id).await;
    println!("      locked edit -> {:?}", edit.as_ref().map(|r| r.success));
    println!("      locked delete -> {:?}", delete.as_ref().map(|r| r.success));
    assert!(released.is_ok(), "{released:?}");
    // The server exempts an admin from the lock on delete, not on edit.
    assert!(edit.is_err(), "{edit:?}");
    if delete.is_err() {
        api.delete_transaction(row.id).await.expect("cleanup");
    }
}

async fn sync_sees_a_change(api: Api) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("cache.db")).expect("store"));
    let first = sync_transactions(&api, store.clone(), Mode::Quick, |_, _| {}).await.expect("first sync");
    assert!(matches!(first, SyncResult::Updated(_)), "the first sync brought nothing");
    let quiet = sync_transactions(&api, store.clone(), Mode::Quick, |_, _| {}).await.expect("second sync");
    assert!(matches!(quiet, SyncResult::Unchanged { .. }), "an idle sync downloaded again");

    let row = api.create_transaction(&new_row(&format!("E2E SYNC {}", stamp()))).await.expect("create");
    // The manifest stays cached after a write, so only a forced sync sees it now.
    let after = sync_transactions(&api, store.clone(), Mode::Verify, |_, _| {}).await.expect("forced sync");
    assert!(matches!(&after, SyncResult::Updated(rows) if rows.iter().any(|t| t.id == row.id)), "the new row was not fetched");
    assert!(fetch_all(&api, |_, _| {}).await.unwrap().iter().any(|t| t.id == row.id));
    api.delete_transaction(row.id).await.expect("cleanup");
}

async fn operators(api: Api) {
    let name = format!("E2E OP {}", stamp());
    let body = json!({ "operator_name": name, "application_name": "E2E App", "is_p2p": false, "is_active": true });
    let made = api.create_operator(&body).await.expect("create");
    assert!(made.id > 0 && made.operator_name == name && made.is_active, "{made:?}");

    let changed = api.update_operator(made.id, &json!({ "application_name": "E2E App 2" })).await.expect("edit");
    assert_eq!(changed.application_name, "E2E App 2");
    let flipped = api.update_operator(made.id, &json!({ "is_p2p": true })).await.expect("p2p");
    assert!(flipped.is_p2p);
    let off = api.update_operator(made.id, &json!({ "is_active": false })).await.expect("off");
    assert!(!off.is_active);

    let page = api.operators(&[("search", name.clone()), ("page", "1".into()), ("page_size", "20".into())]).await.expect("search");
    assert!(page.items.iter().any(|o| o.id == made.id), "search missed it: {page:?}");
    assert!(page.total >= 1);
    assert!(api.all_operators().await.expect("all").iter().any(|o| o.id == made.id));
    assert!(!api.applications().await.expect("applications").is_empty());

    api.delete_operator(made.id).await.expect("delete");
    assert!(!api.all_operators().await.unwrap().iter().any(|o| o.id == made.id && o.is_active));
}

async fn operators_file(api: Api) {
    let (name, bytes) = api.export_operators().await.expect("export");
    assert_eq!(&bytes[..2], b"PK");
    println!("      operators file: {name:?}, {} bytes", bytes.len());
    let reply = api.import_operators("operators.xlsx", bytes).await.expect("import");
    println!("      imported {}, skipped {}, errors {}", reply.imported, reply.skipped, reply.errors.len());
    assert!(reply.imported + reply.skipped > 0, "{reply:?}");
}

async fn dictionaries(api: Api) {
    let tag = stamp();
    let code = format!("E{tag}");
    let article = api.create_article(&code, "e2e статья").await.expect("article");
    assert!(article.id > 0 && !article.code.is_empty() && article.label == "e2e статья", "{article:?}");
    let edited = api.update_article(article.id, &json!({ "code": article.code, "label": "другая" })).await.expect("edit article");
    assert_eq!(edited.label, "другая");
    assert!(api.annotation_articles(true).await.unwrap().iter().any(|a| a.id == article.id));
    api.delete_article(article.id).await.expect("delete article");

    let person = api.create_person(&code, "Иван Тест").await.expect("person");
    assert!(person.id > 0 && person.full_name.as_deref() == Some("Иван Тест"), "{person:?}");
    let edited = api.update_person(person.id, &json!({ "code": person.code, "full_name": "Пётр" })).await.expect("edit person");
    assert_eq!(edited.full_name.as_deref(), Some("Пётр"));
    assert!(api.annotation_persons(true).await.unwrap().iter().any(|p| p.id == person.id));
    api.delete_person(person.id).await.expect("delete person");
}

async fn descriptions(api: Api) {
    let tag = stamp();
    let text = format!("e2e описание {tag}");
    let keys = [format!("E2E LINK {tag}")];
    let made = api.create_description(&text, &keys).await.expect("create");
    assert!(made.id > 0 && made.text == text, "{made:?}");
    println!("      after create: {} / {:?}", made.operator_count, made.operator_keys);

    let edited = api.update_description(made.id, &format!("{text} 2")).await.expect("edit");
    assert_eq!(edited.text, format!("{text} 2"));

    let more = [format!("E2E LINK2 {tag}")];
    let linked = api.link_operators(made.id, &more).await.expect("link");
    assert!(linked.operator_keys.len() >= made.operator_keys.len() + 1, "{linked:?}");
    let unlinked = api.unlink_operators(made.id, &more).await.expect("unlink");
    assert_eq!(unlinked.operator_keys.len(), linked.operator_keys.len() - 1, "{unlinked:?}");

    let page = api.descriptions(&[("search", tag.clone()), ("page", "1".into()), ("page_size", "20".into())]).await.expect("search");
    assert!(page.items.iter().any(|d| d.id == made.id), "search missed it: {page:?}");
    api.delete_description(made.id).await.expect("delete");
}

/// The client's ledger in miniature: header on row 2, data from row 4.
fn ledger_book(details: &str) -> Vec<u8> {
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    for (col, head) in ["Дата и время", "Оператор", "Сумма", "ПК", "Подробности"].into_iter().enumerate() {
        sheet.write_string(1, col as u16, head).unwrap();
    }
    for (col, value) in ["2024-03-15 10:20:30", "E2E BOOK", "4321.09", "1234", details].into_iter().enumerate() {
        sheet.write_string(3, col as u16, value).unwrap();
    }
    book.save_to_buffer().unwrap()
}

async fn annotations(api: Api) {
    let mut row = new_row(&format!("E2E BOOK {}", stamp()));
    row.amount = "4321.09".into();
    let row = api.create_transaction(&row).await.expect("create");
    let book = ledger_book("e2e из книги");
    let preview = api.import_annotations("book.xlsx", book.clone(), true, false, "manual").await;
    println!("      dry run -> {:?}", preview.as_ref().map(|r| (r.ok, r.status.clone(), r.report.row_count, r.report.matched)));
    let preview = preview.expect("dry run");
    assert_eq!((preview.report.row_count, preview.report.matched), (1, 1), "{preview:?}");
    assert!(preview.batch_id.is_some(), "{preview:?}");
    assert_eq!(api.transaction(row.id).await.unwrap().annotation_details.unwrap_or_default(), "", "a dry run changed the row");

    let applied = api.import_annotations("book.xlsx", book, false, false, "manual").await.expect("apply");
    println!("      applied -> {} / {}", applied.status, applied.report.applied);
    assert_eq!(applied.report.applied, 1, "{applied:?}");
    assert_eq!(api.transaction(row.id).await.unwrap().annotation_details.as_deref(), Some("e2e из книги"));
    api.delete_transaction(row.id).await.expect("cleanup");

    let rules = api.annotation_rules().await.expect("rules");
    let coverage = api.rule_coverage().await.expect("coverage");
    let recomputed = api.recompute_rules().await.expect("recompute");
    let dry = api.apply_rules(true).await.expect("apply dry");
    println!("      rules {}, coverage {:?}", rules.len(), coverage.article.as_ref().map(|c| c.annotated_rows));
    println!("      recompute {recomputed:?}, dry {dry:?}");
    assert!(dry.dry_run, "{dry:?}");
    assert!(rules.is_empty() || dry.scanned > 0, "{dry:?}");
    let real = api.apply_rules(false).await.expect("apply");
    println!("      apply {real:?}");
    assert!(!real.dry_run, "{real:?}");
}

async fn logs_and_audit(api: Api) {
    let stats = api.log_stats().await.expect("stats");
    println!("      stats {stats:?}");
    let page = api.logs(&[("page", "1".into()), ("page_size", "20".into())]).await.expect("logs");
    println!("      logs total {}", page.total);
    assert!(page.total > 0 && !page.items.is_empty(), "{page:?}");
    let second = api.logs(&[("page", "2".into()), ("page_size", "20".into())]).await.expect("logs page 2");
    assert_ne!(page.items[0].task_id, second.items.first().map(|i| i.task_id.clone()).unwrap_or_default());
    let status = page.items[0].status.clone();
    let filtered = api.logs(&[("page", "1".into()), ("page_size", "20".into()), ("status", status.clone())]).await.expect("filter");
    assert!(filtered.items.iter().all(|i| i.status == status), "filter ignored");

    let audit = api.audit(&[("limit", "50".into()), ("offset", "0".into())]).await.expect("audit");
    println!("      audit rows {}", audit.len());
    assert!(!audit.is_empty());
    assert!(audit.iter().all(|r| !r.action.is_empty()), "audit rows arrived empty");
}

async fn system_switches(api: Api) {
    let before = api.system_settings().await.expect("read");
    assert!(before.id > 0, "{before:?}");
    let flipped = api.update_system_settings(&json!({ "bot_control_enabled": !before.bot_control_enabled })).await.expect("flip");
    assert_eq!(flipped.bot_control_enabled, !before.bot_control_enabled);
    let back = api.update_system_settings(&json!({ "bot_control_enabled": before.bot_control_enabled })).await.expect("restore");
    assert_eq!(back.bot_control_enabled, before.bot_control_enabled);
}

async fn periods(api: Api) {
    let made = api.create_period("2019-01-01", "2019-01-31", Some("e2e период")).await.expect("create");
    assert!(made.id > 0 && made.is_active && made.date_from.starts_with("2019-01-01"), "{made:?}");
    let me = api.me().await.unwrap();
    let changed = api.update_period(made.id, &json!({ "excluded_user_ids": [me.id] })).await.expect("exclude");
    assert_eq!(changed.excluded_user_ids, vec![me.id]);
    assert!(api.periods().await.unwrap().iter().any(|p| p.id == made.id));
    println!("      visible to the gate: {}", api.locked_periods().await.expect("locked").len());
    api.deactivate_period(made.id).await.expect("deactivate");
    let gone = api.periods().await.unwrap();
    assert!(!gone.iter().any(|p| p.id == made.id && p.is_active), "still active");
}

async fn scopes(api: Api) {
    let status = api.access_status().await.expect("status");
    println!("      scopes_enabled {}, years {:?}", status.scopes_enabled, status.available_years);
    let body = json!({ "name": format!("E2E {}", stamp()), "password": "e2e-Pass-12345", "years": [2019], "allow_transactions": true, "allow_sources": false, "is_active": true });
    let made = api.create_scope(&body).await;
    println!("      create -> {:?}", made.as_ref().map(|s| s.id));
    let made = made.expect("create");
    assert!(made.id > 0 && made.years == vec![2019], "{made:?}");
    let renamed = api.update_scope(made.id, &json!({ "name": "E2E другое", "years": [2019], "allow_transactions": true, "allow_sources": false, "is_active": true })).await.expect("edit");
    assert_eq!(renamed.name, "E2E другое");
    assert!(api.access_scopes().await.unwrap().iter().any(|s| s.id == made.id));
    let wrong = api.verify_scope_code(made.id, "000000").await;
    println!("      wrong code -> {}", wrong.is_err());
    assert!(wrong.is_err(), "a wrong code opened a scope");
    api.delete_scope(made.id).await.expect("delete");
    assert!(!api.access_scopes().await.unwrap().iter().any(|s| s.id == made.id && s.is_active));
}

async fn excel(api: Api, url: String) {
    let created = api.excel_issue_grant("E2E ПК", 30).await.expect("issue");
    assert!(!created.grant.id.is_empty() && !created.token.is_empty(), "the grant came back empty");
    assert!(!created.server_url.is_empty(), "no server url");
    assert!(api.excel_grants().await.unwrap().iter().any(|g| g.id == created.grant.id));

    // A password goes on a book that has connected once, so connect as the book does.
    let id = format!("00000000-0000-4000-8000-000{}", stamp());
    let snapshot = reqwest::Client::new()
        .get(format!("{url}/api/excel/v1/snapshot"))
        .bearer_auth(&created.token)
        .header("X-Excel-Workbook", &id)
        .send()
        .await
        .expect("snapshot call");
    assert!(snapshot.status().is_success(), "{}", snapshot.status());
    let bound = api.excel_grants().await.unwrap();
    assert!(bound.iter().any(|g| g.id == created.grant.id && g.workbook_id.as_deref() == Some(id.as_str())), "the grant did not bind");

    let set = api.excel_set_password(&id, "e2e-Pass-12345").await.expect("set password");
    assert_eq!(set.workbook_id, id);
    assert!(api.excel_activations().await.unwrap().iter().any(|a| a.workbook_id == id));
    api.excel_remove_password(&id).await.expect("remove password");
    assert!(!api.excel_activations().await.unwrap().iter().any(|a| a.workbook_id == id));

    api.excel_revoke_grant(&created.grant.id).await.expect("revoke");
    let after = api.excel_grants().await.unwrap();
    assert!(after.iter().filter(|g| g.id == created.grant.id).all(|g| g.revoked_at.is_some()), "still live");

    let workbook = api.excel_workbook().await;
    println!("      workbook -> {:?}", workbook.as_ref().map(Vec::len));
    assert_eq!(&workbook.expect("workbook")[..2], b"PK");

    // A workbook the server makes: its file is ready, and the password alone
    // connects a computer, the way the macro does it.
    let name = format!("E2E книга {}", stamp());
    let made = api.excel_create_workbook(&name, "e2e-Book-12345").await.expect("create workbook");
    assert!(!made.workbook_id.is_empty(), "the workbook came back without an id");
    assert_eq!(made.name.as_deref(), Some(name.as_str()));
    assert!(api.excel_activations().await.unwrap().iter().any(|a| a.workbook_id == made.workbook_id && a.name.as_deref() == Some(name.as_str())), "the workbook is not listed");
    let (file_name, file) = api.excel_workbook_file(&made.workbook_id).await.expect("workbook file");
    println!("      configured workbook -> {} bytes as {file_name:?}", file.len());
    assert_eq!(&file[..2], b"PK");
    assert!(file_name.is_some_and(|n| n.ends_with(".xlsm")), "the file has no workbook name");

    let http = reqwest::Client::new();
    let device = format!("e2e-device-{}", stamp());
    let activate = |password: &'static str| {
        http.post(format!("{url}/api/excel/v1/activate"))
            .header("X-Excel-Workbook", &made.workbook_id)
            .header("X-Excel-Device", &device)
            .json(&json!({ "device_name": "E2E ПК", "password": password }))
            .send()
    };
    assert_eq!(activate("not-the-password").await.expect("activate call").status().as_u16(), 401);
    let activated = activate("e2e-Book-12345").await.expect("activate call");
    assert_eq!(activated.status().as_u16(), 201);
    let reply: Value = activated.json().await.expect("activation reply");
    let token = reply["token"].as_str().expect("a token for this computer");
    let receipts = http
        .get(format!("{url}/api/excel/v1/snapshot"))
        .bearer_auth(token)
        .header("X-Excel-Workbook", &made.workbook_id)
        .header("X-Excel-Device", &device)
        .send()
        .await
        .expect("snapshot call");
    assert!(receipts.status().is_success(), "{}", receipts.status());

    let grants = api.excel_grants().await.unwrap();
    let computer = grants.iter().find(|g| g.workbook_id.as_deref() == Some(made.workbook_id.as_str()) && g.revoked_at.is_none()).expect("the computer's connection");
    assert!(computer.device_bound, "the connection is not tied to the computer");
    api.excel_revoke_grant(&computer.id).await.expect("disconnect the computer");
    api.excel_remove_password(&made.workbook_id).await.expect("remove the workbook");
    assert!(!api.excel_activations().await.unwrap().iter().any(|a| a.workbook_id == made.workbook_id));
}

async fn gates(api: Api) {
    let entitled = api.entitlement().await.expect("entitlement");
    assert!(entitled.ok && entitled.username.is_some(), "{entitled:?}");
    let launch = api.launch_status().await.expect("launch");
    println!("      launch {launch:?}");
    let wrong = api.verify_launch("definitely-wrong").await;
    println!("      wrong launch password -> {:?}", wrong.as_ref().map(|v| v.verified));
    // With no password set the server opens the gate to anyone, by design.
    assert!(!launch.password_set || !matches!(&wrong, Ok(v) if v.verified), "a wrong password opened the gate");

    // QR login needs the Telegram bot, which a local server may not have.
    match api.qr_generate().await {
        Ok(qr) => {
            assert!(!qr.session_id.is_empty() && qr.expires_in > 0, "{qr:?}");
            assert_eq!(api.qr_status(&qr.session_id).await.expect("qr status").status, "pending");
        }
        Err(error) => {
            println!("      qr skipped: {error}");
            assert!(error.to_string().contains("bot not configured"), "{error:?}");
        }
    }
}

/// The six-digit code an authenticator shows for this secret right now.
fn totp(secret: &str) -> String {
    let mut bits: u64 = 0;
    let mut held = 0;
    let mut key = Vec::new();
    for ch in secret.trim_end_matches('=').chars() {
        let value = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".find(ch.to_ascii_uppercase()).expect("base32") as u64;
        bits = (bits << 5) | value;
        held += 5;
        if held >= 8 {
            held -= 8;
            key.push((bits >> held) as u8);
            bits &= (1 << held) - 1;
        }
    }
    let step = (chrono::Utc::now().timestamp() / 30) as u64;
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).unwrap();
    mac.update(&step.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let at = (hash[19] & 0x0f) as usize;
    let number = u32::from_be_bytes([hash[at] & 0x7f, hash[at + 1], hash[at + 2], hash[at + 3]]);
    format!("{:06}", number % 1_000_000)
}

async fn users(api: Api, url: String) {
    let username = format!("e2e{}", stamp());
    let password = "e2e-Pass-12345";
    let new = NewUser {
        username: username.clone(),
        password: password.into(),
        role: "operator".into(),
        display_name: "E2E Пользователь".into(),
        is_active: true,
        allowed_tabs: vec!["dashboard".into()],
        allowed_folders: vec![],
        forbidden_periods: vec![],
        allowed_sources: vec![],
        can_toggle_sources: false,
        force_2fa: false,
        session_ttl_days: 30,
        telegram_id: None,
        telegram_username: None,
    };
    let made = api.create_user(&new).await.expect("create");
    assert!(made.id > 0 && made.username == username && made.is_active, "{made:?}");
    assert_eq!(made.allowed_tabs, vec!["dashboard".to_string()]);

    let changes = json!({
        "display_name": "Другое имя",
        "role": "operator",
        "is_active": true,
        "allowed_tabs": ["dashboard", "reference"],
        "allowed_folders": [],
        "forbidden_periods": [{ "from": "2020-01-01", "to": "2020-01-31" }],
        "allowed_sources": [],
        "can_toggle_sources": true,
        "force_2fa": false,
        "session_ttl_days": 14,
    });
    let edited = api.update_user(made.id, &changes).await.expect("edit");
    assert_eq!(edited.display_name, "Другое имя");
    assert_eq!(edited.session_ttl_days, 14);
    assert_eq!(edited.allowed_tabs.len(), 2);
    assert_eq!(edited.forbidden_periods.len(), 1, "{:?}", edited.forbidden_periods);
    assert!(edited.can_toggle_sources);

    assert_eq!(api.link_telegram(made.id, 777_000_111, Some("e2e_user")).await.expect("link").telegram_id, Some(777_000_111));
    assert_eq!(api.unlink_telegram(made.id).await.expect("unlink").telegram_id, None);
    assert!(api.force_two_factor(made.id, true).await.expect("force").force_2fa);
    assert!(!api.force_two_factor(made.id, false).await.expect("unforce").force_2fa);
    api.unlock_user(made.id).await.expect("unlock");
    api.reset_password(made.id, "e2e-Pass-67890").await.expect("reset password");

    // The new account's own session, and its two-factor setup from end to end.
    let vault: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    let own = Api::new(url.clone(), None, vault);
    assert!(!own.login(&username, password).await.expect("old password call").ok, "the old password still works");
    let reply = own.login(&username, "e2e-Pass-67890").await.expect("login");
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(own.me().await.expect("me").username, username);
    assert!(own.admin_users().await.is_err(), "a plain user read the admin list");

    let status = own.two_factor_status().await.expect("2fa status");
    assert!(!status.enabled, "{status:?}");
    let setup = own.two_factor_setup(None).await.expect("2fa setup");
    assert!(!setup.secret.is_empty() && !setup.otpauth_uri.is_empty(), "the setup came back empty");
    println!("      2fa setup: {} backup codes, qr {} bytes", setup.backup_codes.len(), setup.qr_code_base64.len());
    let wrong = own.two_factor_confirm("000000", None).await;
    assert!(!matches!(&wrong, Ok(r) if r.ok), "a wrong code enabled 2fa");
    assert_eq!(own.me().await.expect("a wrong code signed the user out").username, username);
    let confirmed = own.two_factor_confirm(&totp(&setup.secret), None).await.expect("2fa confirm");
    assert!(confirmed.ok || confirmed.enabled, "{confirmed:?}");
    assert!(own.two_factor_status().await.unwrap().enabled);

    // Both sensitive actions ask for a code again, and a wrong one changes nothing.
    assert!(own.two_factor_backup_codes("000000").await.is_err(), "new codes without a valid code");
    assert!(own.two_factor_disable("000000").await.is_err(), "2fa went off without a valid code");
    assert!(own.two_factor_status().await.expect("a wrong code signed the user out").enabled);
    let spare = setup.backup_codes.first().expect("a backup code");
    let codes = own.two_factor_backup_codes(spare).await.expect("new backup codes").backup_codes;
    println!("      backup codes -> {}", codes.len());
    assert!(!codes.is_empty() && !codes.contains(spare), "the codes were not replaced");

    own.two_factor_disable(&codes[0]).await.expect("disable 2fa");
    assert!(!own.two_factor_status().await.unwrap().enabled);
    let setup = own.two_factor_setup(None).await.expect("2fa setup again");
    let confirmed = own.two_factor_confirm(&totp(&setup.secret), None).await.expect("2fa confirm again");
    assert!(confirmed.ok || confirmed.enabled, "{confirmed:?}");

    // An operator is asked for the second factor only when an admin demands it.
    api.force_two_factor(made.id, true).await.expect("force 2fa");

    let second: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    let again = Api::new(url, None, second);
    let step = again.login(&username, "e2e-Pass-67890").await.expect("login step one");
    assert!(step.requires_2fa, "two-factor was not asked for: {:?}", step.error);
    let temp = step.temp_token.expect("temp token");
    tokio::time::sleep(std::time::Duration::from_secs(31)).await; // a TOTP code works once
    let done = again.login_2fa(&temp, &totp(&setup.secret)).await.expect("login step two");
    assert!(done.ok, "{:?}", done.error);
    assert_eq!(again.me().await.unwrap().username, username);


    let cleared = api.reset_two_factor(made.id).await.expect("admin resets 2fa");
    assert!(!cleared.totp_enabled, "{cleared:?}");
    let off = api.deactivate_user(made.id).await.expect("deactivate");
    assert!(!off.is_active, "{off:?}");
    assert!(api.admin_users().await.unwrap().iter().any(|u| u.id == made.id && !u.is_active));
}

async fn sessions(api: Api, url: String, username: String, password: String) {
    let vault: Arc<dyn SecretStore> = Arc::new(MemoryStore::default());
    let other = Api::new(url, None, vault);
    assert!(other.login(&username, &password).await.expect("second login").ok);
    assert!(other.me().await.is_ok());

    let list = api.sessions().await.expect("sessions");
    println!("      sessions {}", list.len());
    assert!(list.len() >= 2, "{list:?}");
    assert!(list.iter().all(|s| !s.session_id.is_empty()), "sessions arrived without ids");

    let current = api.current_session_id();
    println!("      current known: {}", current.is_some());
    let killed = api.kill_other_sessions(current.as_deref().unwrap_or_default()).await.expect("end others");
    println!("      ended {}", killed.killed);
    assert!(killed.ok && killed.killed >= 1, "{killed:?}");
    assert!(api.me().await.is_ok(), "ending the others ended this session too");
}
