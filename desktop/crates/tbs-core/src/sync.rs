//! Keeps the local copy of transactions current.
//!
//! The server's sync rows are raw database shape, without the display fields
//! the table needs, so they cannot feed the grid directly. Instead the
//! manifest's [`fingerprint`] says whether anything changed, and a change refetches
//! through the regular list endpoint. At today's few thousand rows that is
//! under a second; past that, swap in cursor paging behind this same function.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::api::{Api, ApiError};
use crate::model::Transaction;
use crate::store::Store;

pub const PAGE_SIZE: usize = 1000;

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

/// What the local copy is compared by. The checksum alone covers only the
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

pub async fn fetch_all(api: &Api, mut progress: impl FnMut(usize, i64)) -> Result<Vec<Transaction>, ApiError> {
    let mut rows = Vec::new();
    let mut page = 1usize;
    loop {
        let result = api.list_transactions(page, PAGE_SIZE).await?;
        let received = result.items.len();
        rows.extend(result.items);
        progress(rows.len(), result.total);
        if !result.has_more || received == 0 {
            break;
        }
        page += 1;
    }
    Ok(rows)
}

pub async fn sync_transactions(
    api: &Api,
    store: Arc<Store>,
    force: bool,
    progress: impl FnMut(usize, i64),
) -> Result<SyncResult, ApiError> {
    let manifest: Manifest = api.get("/api/sync/manifest", &[]).await?;
    let local = {
        let store = store.clone();
        tokio::task::spawn_blocking(move || store.checksum().ok().flatten()).await.ok().flatten()
    };
    if !needs_fetch(&manifest, local.as_deref(), force) {
        let rows = manifest.tables.get("transactions").map_or(0, |t| t.row_count);
        return Ok(SyncResult::Unchanged { rows });
    }
    let rows = fetch_all(api, progress).await?;
    let checksum = fingerprint(&manifest);
    let saved = rows.clone();
    let write = tokio::task::spawn_blocking(move || store.replace_transactions(&saved, checksum.as_deref())).await;
    if !matches!(write, Ok(Ok(()))) {
        // The cache is a convenience: the data in hand is still good to show.
        tracing::warn!("could not persist transactions cache");
    }
    Ok(SyncResult::Updated(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
