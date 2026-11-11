//! Runs against a real backend. Skipped unless `TBS_E2E_LOGIN` points at a
//! JSON file with `username` and `password`; `TBS_E2E_URL` sets the server.
//!
//! One test on purpose: the server allows ten logins a minute per address,
//! so the scenarios share a session instead of signing in separately.

use std::sync::Arc;

use tbs_core::api::{Api, ApiError, ApiEvent, Call};
use tbs_core::columns::Column;
use tbs_core::query::{Dataset, Filters, Scope, Sort};
use tbs_core::reqwest;
use tbs_core::secret::{MemoryStore, SecretStore};
use tbs_core::store::Store;
use tbs_core::sync::{Mode, SyncResult, fetch_all, sync_transactions};

struct Creds {
    url: String,
    username: String,
    password: String,
}

fn creds() -> Option<Creds> {
    let path = std::env::var("TBS_E2E_LOGIN").ok()?;
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some(Creds {
        url: std::env::var("TBS_E2E_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".into()),
        username: v["username"].as_str()?.into(),
        password: v["password"].as_str()?.into(),
    })
}

async fn signed_in(c: &Creds, vault: Arc<dyn SecretStore>) -> Api {
    let api = Api::new(c.url.clone(), None, vault);
    let reply = api.login(&c.username, &c.password).await.expect("login call");
    assert!(reply.ok, "login refused: {:?}", reply.error);
    api
}

#[tokio::test]
async fn live_backend() {
    let Some(c) = creds() else { return };
    let vault: Arc<MemoryStore> = Arc::new(MemoryStore::default());

    // A wrong password is an answer, not a crash, and leaves no session.
    let bad = Api::new(c.url.clone(), None, vault.clone());
    let refused = bad.login(&c.username, "definitely-wrong").await;
    assert!(matches!(&refused, Ok(r) if !r.ok) || matches!(&refused, Err(ApiError::Http { .. })), "{refused:?}");
    assert!(!bad.has_auth_token());

    let api = signed_in(&c, vault.clone()).await;
    assert!(api.has_refresh_token());
    assert_eq!(api.me().await.unwrap().username, c.username);

    concurrent_calls_recover_from_a_dead_access_token(&api).await;
    sync_downloads_then_stays_quiet(&api).await;
    real_rows_survive_the_pipeline(&api).await;
    proxy_uses_the_session_not_the_page(&api).await;
    export_arrives_as_a_named_workbook(&api).await;
    missing_row_is_a_readable_error(&api).await;

    // A fresh process finds the session through the vault alone.
    let restarted = Api::new(c.url.clone(), None, vault.clone());
    assert!(!restarted.has_auth_token());
    let user = restarted.resume().await.unwrap().expect("session resumes");
    assert_eq!(user.username, c.username);

    dead_session_is_reported(&restarted).await;
}

async fn concurrent_calls_recover_from_a_dead_access_token(api: &Api) {
    api.break_auth_token_for_test();
    let calls: Vec<_> = (0..6)
        .map(|_| {
            let api = api.clone();
            tokio::spawn(async move { api.me().await })
        })
        .collect();
    for call in calls {
        call.await.unwrap().expect("each call recovers through one refresh");
    }
}

async fn sync_downloads_then_stays_quiet(api: &Api) {
    let store = Arc::new(Store::in_memory().unwrap());
    let first = sync_transactions(api, store.clone(), Mode::Quick, |_, _| {}).await.unwrap();
    let SyncResult::Updated(rows) = first else { panic!("first sync must download") };
    assert!(!rows.is_empty());
    assert_eq!(store.load_transactions().unwrap().len(), rows.len());

    let second = sync_transactions(api, store.clone(), Mode::Quick, |_, _| {}).await.unwrap();
    assert!(matches!(second, SyncResult::Unchanged { .. }), "{second:?}");
}

async fn real_rows_survive_the_pipeline(api: &Api) {
    let rows = fetch_all(api, |_, _| {}).await.unwrap();
    let total = rows.len();
    let ds = Dataset::build(rows);
    let all = ds.view(&Filters::default(), &Scope::default(), &Sort::default());
    assert_eq!(all.len(), total);
    let dated = ds.rows.iter().filter(|r| r.when.is_some()).count();
    assert!(dated * 10 >= total * 9, "dates must parse: {dated}/{total}");
    assert!(ds.rows.iter().all(|r| r.cell(Column::Amount) != "NaN"));
    let init = api.transactions_init().await.unwrap();
    assert!(!init.years.is_empty());
}

async fn proxy_uses_the_session_not_the_page(api: &Api) {
    let headers = vec![("Authorization".to_string(), "Bearer forged".to_string())];
    let reply = api.proxy(reqwest::Method::GET, "/api/auth/me?x=1%202", headers, Vec::new()).await.unwrap();
    assert_eq!(reply.status, 200);
}

async fn export_arrives_as_a_named_workbook(api: &Api) {
    let (name, bytes) = api.download("/api/transactions/export", &[]).await.unwrap();
    assert!(name.unwrap_or_default().ends_with(".xlsx"));
    assert_eq!(&bytes[..2], b"PK");
}

async fn missing_row_is_a_readable_error(api: &Api) {
    let call = Call::new(reqwest::Method::GET, "/api/transactions/999999999");
    let err = api.checked(&call).await.unwrap_err();
    assert!(matches!(err, ApiError::Http { status: 404 | 405, .. }), "{err:?}");
}

async fn dead_session_is_reported(api: &Api) {
    let mut events = api.subscribe();
    api.break_auth_token_for_test();
    api.break_refresh_token_for_test();
    assert!(api.me().await.is_err());
    assert_eq!(events.try_recv().unwrap(), ApiEvent::SessionRevoked);
    assert!(!api.has_refresh_token());
}
