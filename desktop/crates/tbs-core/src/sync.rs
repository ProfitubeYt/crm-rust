//! Keeps the local copy of transactions current.
//!
//! The table is drawn from a local snapshot, so everything hangs on knowing
//! when that snapshot stopped being true. There are two ways to find out:
//!
//! * [`Mode::Quick`] asks the server's manifest. It is cheap, but it is a hint
//!   and not proof: its checksum covers the newest hundred rows, the server
//!   keeps the answer for minutes, and nothing the server resolves while
//!   reading a row (an operator's description, a chat's title) shows in it.
//! * [`Mode::Verify`] downloads the rows and compares what they add up to
//!   with the snapshot in hand. That is proof, at the price of one download.
//!
//! The server's sync rows are raw database shape, without the display fields
//! the table needs, so both modes read the regular list endpoint.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::api::{Api, ApiError};
use crate::model::Transaction;
use crate::store::Store;

pub const PAGE_SIZE: usize = 1000;
/// How often a table that moved under the walk is read again.
const WALK_ATTEMPTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Quick,
    Verify,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TableManifest {
    pub row_count: i64,
    pub updated_at: Option<String>,
    pub checksum: Option<String>,
    pub latest_transaction_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub tables: HashMap<String, TableManifest>,
    pub server_time: Option<String>,
    pub full_resync_required: Option<bool>,
}

#[derive(Debug)]
pub enum SyncResult {
    Unchanged { rows: i64 },
    Updated(Vec<Transaction>),
}

/// What the manifest is compared by. The checksum alone covers only the
/// newest hundred rows, so a change to an older one (an edit, a backfill, the
/// server's annotation pass) also needs the row count and the latest update.
pub fn fingerprint(manifest: &Manifest) -> Option<String> {
    let table = manifest.tables.get("transactions")?;
    let checksum = table.checksum.as_deref()?;
    Some(format!("{checksum}|{}|{}", table.row_count, table.updated_at.as_deref().unwrap_or("")))
}

/// Pure decision, separated so the rule can be tested without a server.
pub fn needs_fetch(manifest: &Manifest, local: Option<&str>, force: bool) -> bool {
    if force || manifest.full_resync_required == Some(true) {
        return true;
    }
    match fingerprint(manifest) {
        Some(remote) => Some(remote.as_str()) != local,
        None => true,
    }
}

/// What a set of rows adds up to: every field of every row, in order. Two
/// snapshots with the same digest show the same table.
pub fn digest(rows: &[Transaction]) -> String {
    let mut hasher = Sha256::new();
    for row in rows {
        let body = serde_json::to_vec(row).expect("transaction serialises");
        hasher.update((body.len() as u64).to_le_bytes());
        hasher.update(&body);
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// One pass over the list. Offset paging over a table that changes during the
/// pass can hand the same row out twice; it is kept once.
async fn walk(api: &Api, progress: &mut impl FnMut(usize, i64)) -> Result<(Vec<Transaction>, i64), ApiError> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    let mut total;
    let mut page = 1usize;
    loop {
        let result = api.list_transactions(page, PAGE_SIZE).await?;
        let received = result.items.len();
        total = result.total;
        rows.extend(result.items.into_iter().filter(|row| seen.insert(row.id)));
        progress(rows.len(), total);
        if !result.has_more || received == 0 {
            break;
        }
        page += 1;
    }
    Ok((rows, total))
}

pub async fn fetch_all(api: &Api, mut progress: impl FnMut(usize, i64)) -> Result<Vec<Transaction>, ApiError> {
    let mut attempt = 1;
    loop {
        let (rows, total) = walk(api, &mut progress).await?;
        // A row inserted or deleted mid-walk shifts the pages, and then the
        // rows in hand do not add up to the total the last page reported.
        if rows.len() as i64 == total || attempt == WALK_ATTEMPTS {
            return Ok(rows);
        }
        attempt += 1;
    }
}

pub async fn sync_transactions(
    api: &Api,
    store: Arc<Store>,
    mode: Mode,
    progress: impl FnMut(usize, i64),
) -> Result<SyncResult, ApiError> {
    // Without a manifest the rows themselves still say whether anything
    // changed, so its failure costs a download and not the table.
    let manifest = match api.get::<Manifest>("/api/sync/manifest", &[]).await {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            tracing::warn!("sync manifest unavailable: {error}");
            None
        }
    };
    let (local_mark, local_digest) = {
        let store = store.clone();
        tokio::task::spawn_blocking(move || (store.checksum().ok().flatten(), store.digest().ok().flatten())).await.unwrap_or_default()
    };
    if mode == Mode::Quick
        && let Some(manifest) = &manifest
        && !needs_fetch(manifest, local_mark.as_deref(), false)
    {
        let rows = manifest.tables.get("transactions").map_or(0, |t| t.row_count);
        return Ok(SyncResult::Unchanged { rows });
    }

    let rows = fetch_all(api, progress).await?;
    let fresh = digest(&rows);
    let mark = manifest.as_ref().and_then(fingerprint);
    if local_digest.as_deref() == Some(fresh.as_str()) {
        if mark != local_mark {
            let _ = tokio::task::spawn_blocking(move || store.set_checksum(mark.as_deref())).await;
        }
        return Ok(SyncResult::Unchanged { rows: rows.len() as i64 });
    }
    let saved = rows.clone();
    let write = tokio::task::spawn_blocking(move || store.replace_transactions(&saved, mark.as_deref(), Some(&fresh))).await;
    if !matches!(write, Ok(Ok(()))) {
        // The cache is a convenience: the data in hand is still good to show.
        tracing::warn!("could not persist transactions cache");
    }
    Ok(SyncResult::Updated(rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use crate::secret::MemoryStore;

    fn manifest(checksum: Option<&str>) -> Manifest {
        let table = TableManifest { checksum: checksum.map(str::to_string), row_count: 2, updated_at: Some("2026-11-10T09:00:00".into()), ..Default::default() };
        Manifest { tables: HashMap::from([("transactions".to_string(), table)]), ..Default::default() }
    }

    #[test]
    fn fetch_decision() {
        let local = fingerprint(&manifest(Some("a")));
        assert!(!needs_fetch(&manifest(Some("a")), local.as_deref(), false));
        assert!(needs_fetch(&manifest(Some("b")), local.as_deref(), false));
        assert!(needs_fetch(&manifest(Some("a")), None, false));
        assert!(needs_fetch(&manifest(Some("a")), local.as_deref(), true));
        assert!(needs_fetch(&manifest(None), local.as_deref(), false));
        let mut m = manifest(Some("a"));
        m.full_resync_required = Some(true);
        assert!(needs_fetch(&m, local.as_deref(), false));
    }

    #[test]
    fn a_change_outside_the_sampled_rows_still_counts() {
        let local = fingerprint(&manifest(Some("a")));
        let mut edited = manifest(Some("a"));
        edited.tables.get_mut("transactions").unwrap().updated_at = Some("2026-11-10T09:05:00".into());
        assert!(needs_fetch(&edited, local.as_deref(), false));
        let mut deleted = manifest(Some("a"));
        deleted.tables.get_mut("transactions").unwrap().row_count = 1;
        assert!(needs_fetch(&deleted, local.as_deref(), false));
        // A cache saved by an earlier build kept the bare checksum.
        assert!(needs_fetch(&manifest(Some("a")), Some("a"), false));
    }

    fn tx(id: i64) -> Transaction {
        Transaction { id, operator_raw: Some(format!("op{id}")), transaction_date: Some(format!("2026-01-{:02}T10:00:00", id.clamp(1, 28))), ..Default::default() }
    }

    #[test]
    fn the_digest_sees_every_field_and_the_order() {
        let rows = vec![tx(1), tx(2)];
        assert_eq!(digest(&rows), digest(&rows.clone()));
        let mut described = rows.clone();
        described[1].description = Some("Склад".into());
        assert_ne!(digest(&rows), digest(&described));
        assert_ne!(digest(&rows), digest(&[tx(2), tx(1)]));
        assert_ne!(digest(&rows), digest(&rows[..1]));
        assert!(digest(&[]).starts_with("sha256:"));
    }

    /// A server small enough to read: what the list holds, what the manifest
    /// says about it, and how the test wants either to misbehave.
    #[derive(Default)]
    struct Server {
        rows: Mutex<Vec<Transaction>>,
        mark: Mutex<String>,
        page_len: usize,
        lists: AtomicUsize,
        manifest_down: AtomicBool,
        /// Page 2 starts one row early, as after an insert during the walk.
        shifted: AtomicBool,
    }

    impl Server {
        fn answer(&self, target: &str) -> (u16, String) {
            if target.starts_with("/api/sync/manifest") {
                if self.manifest_down.load(Ordering::SeqCst) {
                    return (503, r#"{"detail":"Sync temporarily unavailable"}"#.to_string());
                }
                let rows = self.rows.lock().unwrap().len();
                let mark = self.mark.lock().unwrap().clone();
                return (200, serde_json::json!({ "tables": { "transactions": { "row_count": rows, "updated_at": mark, "checksum": "sha256:sample" } } }).to_string());
            }
            if target.starts_with("/api/transactions/") {
                self.lists.fetch_add(1, Ordering::SeqCst);
                let page: usize = target.split(['?', '&']).find_map(|pair| pair.strip_prefix("page=")).and_then(|n| n.parse().ok()).unwrap_or(1);
                let rows = self.rows.lock().unwrap();
                let size = self.page_len.max(1);
                let mut start = (page - 1) * size;
                if page > 1 && self.shifted.swap(false, Ordering::SeqCst) {
                    start -= 1;
                }
                let items: Vec<&Transaction> = rows.iter().skip(start).take(size).collect();
                let has_more = start + items.len() < rows.len();
                return (200, serde_json::json!({ "ok": true, "total": rows.len(), "page": page, "page_size": size, "items": items, "has_more": has_more }).to_string());
            }
            (404, r#"{"detail":"Not found"}"#.to_string())
        }

        async fn listen(self: Arc<Self>) -> Api {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let server = self.clone();
                    tokio::spawn(async move {
                        let mut head = Vec::new();
                        let mut chunk = [0u8; 2048];
                        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                            match socket.read(&mut chunk).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => head.extend_from_slice(&chunk[..n]),
                            }
                        }
                        let text = String::from_utf8_lossy(&head).into_owned();
                        let target = text.split_whitespace().nth(1).unwrap_or("/");
                        let (status, body) = server.answer(target);
                        let reply = format!("HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                        let _ = socket.write_all(reply.as_bytes()).await;
                        let _ = socket.shutdown().await;
                    });
                }
            });
            Api::new(base, None, Arc::new(MemoryStore::default()))
        }
    }

    fn server(rows: Vec<Transaction>, page_len: usize) -> Arc<Server> {
        Arc::new(Server { rows: Mutex::new(rows), mark: Mutex::new("2026-11-10T09:00:00".into()), page_len, ..Default::default() })
    }

    async fn run(api: &Api, store: &Arc<Store>, mode: Mode) -> SyncResult {
        sync_transactions(api, store.clone(), mode, |_, _| {}).await.expect("sync")
    }

    #[tokio::test]
    async fn a_quiet_server_is_asked_but_not_downloaded() {
        let server = server((1..=5).map(tx).collect(), 2);
        let api = server.clone().listen().await;
        let store = Arc::new(Store::in_memory().unwrap());
        let SyncResult::Updated(rows) = run(&api, &store, Mode::Quick).await else { panic!("the first sync must download") };
        assert_eq!(rows.iter().map(|t| t.id).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        assert_eq!(store.load_transactions().unwrap().len(), 5);
        let walked = server.lists.load(Ordering::SeqCst);
        assert!(matches!(run(&api, &store, Mode::Quick).await, SyncResult::Unchanged { rows: 5 }));
        assert_eq!(server.lists.load(Ordering::SeqCst), walked, "a quick check downloaded the list");
        // Proof costs one walk and, with nothing changed, rebuilds nothing.
        assert!(matches!(run(&api, &store, Mode::Verify).await, SyncResult::Unchanged { rows: 5 }));
        assert!(server.lists.load(Ordering::SeqCst) > walked);
    }

    #[tokio::test]
    async fn what_the_manifest_cannot_see_is_found_by_the_rows() {
        let server = server((1..=3).map(tx).collect(), 10);
        let api = server.clone().listen().await;
        let store = Arc::new(Store::in_memory().unwrap());
        run(&api, &store, Mode::Quick).await;
        // An operator's description is kept apart from the transactions: it
        // changes what the list says about a row and nothing in the manifest.
        server.rows.lock().unwrap()[1].description = Some("Аренда склада".into());
        assert!(matches!(run(&api, &store, Mode::Quick).await, SyncResult::Unchanged { .. }));
        let SyncResult::Updated(rows) = run(&api, &store, Mode::Verify).await else { panic!("verification missed the change") };
        assert_eq!(rows[1].description.as_deref(), Some("Аренда склада"));
        assert_eq!(store.load_transactions().unwrap()[1].description.as_deref(), Some("Аренда склада"));
        assert!(matches!(run(&api, &store, Mode::Verify).await, SyncResult::Unchanged { .. }));
    }

    #[tokio::test]
    async fn a_moved_manifest_over_the_same_rows_downloads_once() {
        let server = server((1..=3).map(tx).collect(), 10);
        let api = server.clone().listen().await;
        let store = Arc::new(Store::in_memory().unwrap());
        run(&api, &store, Mode::Quick).await;
        *server.mark.lock().unwrap() = "2026-11-10T09:30:00".into();
        let before = server.lists.load(Ordering::SeqCst);
        assert!(matches!(run(&api, &store, Mode::Quick).await, SyncResult::Unchanged { .. }));
        assert_eq!(server.lists.load(Ordering::SeqCst), before + 1);
        assert!(matches!(run(&api, &store, Mode::Quick).await, SyncResult::Unchanged { .. }));
        assert_eq!(server.lists.load(Ordering::SeqCst), before + 1, "the new manifest was not remembered");
    }

    #[tokio::test]
    async fn the_table_loads_without_a_manifest() {
        let server = server((1..=3).map(tx).collect(), 10);
        server.manifest_down.store(true, Ordering::SeqCst);
        let api = server.clone().listen().await;
        let store = Arc::new(Store::in_memory().unwrap());
        assert!(matches!(run(&api, &store, Mode::Quick).await, SyncResult::Updated(rows) if rows.len() == 3));
        assert!(matches!(run(&api, &store, Mode::Quick).await, SyncResult::Unchanged { rows: 3 }));
        server.rows.lock().unwrap().push(tx(4));
        assert!(matches!(run(&api, &store, Mode::Quick).await, SyncResult::Updated(rows) if rows.len() == 4));
    }

    #[tokio::test]
    async fn a_row_handed_out_twice_is_kept_once() {
        let server = server((1..=5).map(tx).collect(), 2);
        server.shifted.store(true, Ordering::SeqCst);
        let api = server.clone().listen().await;
        let rows = fetch_all(&api, |_, _| {}).await.unwrap();
        assert_eq!(rows.iter().map(|t| t.id).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn a_local_edit_is_checked_against_the_server_again() {
        let server = server((1..=2).map(tx).collect(), 10);
        let api = server.clone().listen().await;
        let store = Arc::new(Store::in_memory().unwrap());
        run(&api, &store, Mode::Verify).await;
        // The edit reached the cache but, say, not the server.
        let mut edited = tx(1);
        edited.operator_raw = Some("локально".into());
        store.put_transaction(&edited).unwrap();
        let SyncResult::Updated(rows) = run(&api, &store, Mode::Verify).await else { panic!("the edited cache was trusted") };
        assert_eq!(rows[0].operator_raw.as_deref(), Some("op1"));
        assert_eq!(store.load_transactions().unwrap()[0].operator_raw.as_deref(), Some("op1"));
    }
}
