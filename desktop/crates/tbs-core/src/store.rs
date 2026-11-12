//! Local copy of server data, so the app opens instantly and keeps reading
//! when the network drops. Nothing here is authoritative: it can be deleted
//! at any time and will be downloaded again.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::Transaction;

const SCHEMA_VERSION: i64 = 2;

/// Names this build's picture of a row. The cache keeps rows as this build
/// writes them, so a build that learns a field must not trust rows saved
/// without it: they would show that field empty until something else changed.
fn schema_stamp() -> String {
    let fields = match serde_json::to_value(Transaction::default()) {
        Ok(serde_json::Value::Object(map)) => map.keys().cloned().collect::<Vec<_>>().join(","),
        _ => String::new(),
    };
    format!("{SCHEMA_VERSION}:{fields}")
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS transactions (id INTEGER PRIMARY KEY, seq INTEGER NOT NULL, body TEXT NOT NULL);",
        )?;
        let store = Store { conn: Mutex::new(conn) };
        let stamp = schema_stamp();
        if store.get("schema_version")?.as_deref() != Some(stamp.as_str()) {
            store.clear()?;
            store.set("schema_version", &stamp)?;
        }
        Ok(store)
    }

    pub fn get(&self, key: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()
    }

    pub fn set(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [key, value])?;
        Ok(())
    }

    pub fn clear(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DELETE FROM kv; DELETE FROM transactions;")
    }

    pub fn load_transactions(&self) -> rusqlite::Result<Vec<Transaction>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT body FROM transactions ORDER BY seq")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for body in rows {
            // A row this build cannot read is dropped; the next sync restores it.
            if let Ok(tx) = serde_json::from_str::<Transaction>(&body?) {
                out.push(tx);
            }
        }
        Ok(out)
    }

    /// Atomic: a crash leaves the previous snapshot, never half of a new one.
    ///
    /// `checksum` is what the server's manifest said when the rows were taken;
    /// `digest` is what the rows themselves add up to (see [`crate::sync`]).
    pub fn replace_transactions(&self, rows: &[Transaction], checksum: Option<&str>, digest: Option<&str>) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM transactions", [])?;
        {
            let mut insert = tx.prepare("INSERT INTO transactions (id, seq, body) VALUES (?1, ?2, ?3)")?;
            for (seq, row) in rows.iter().enumerate() {
                let body = serde_json::to_string(row).expect("transaction serialises");
                insert.execute(params![row.id, seq as i64, body])?;
            }
        }
        for (key, value) in [("tx_checksum", checksum), ("tx_digest", digest)] {
            match value {
                Some(v) => tx.execute("INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [key, v])?,
                None => tx.execute("DELETE FROM kv WHERE key = ?1", [key])?,
            };
        }
        tx.commit()
    }

    pub fn checksum(&self) -> rusqlite::Result<Option<String>> {
        self.get("tx_checksum")
    }

    /// The manifest moved but the rows turned out the same: remember the new
    /// manifest so the next quick check does not download them again.
    pub fn set_checksum(&self, checksum: Option<&str>) -> rusqlite::Result<()> {
        match checksum {
            Some(value) => self.set("tx_checksum", value),
            None => self.conn.lock().unwrap().execute("DELETE FROM kv WHERE key = 'tx_checksum'", []).map(|_| ()),
        }
    }

    /// What the whole snapshot added up to when it was saved, or `None` once a
    /// single row has been written over it.
    pub fn digest(&self) -> rusqlite::Result<Option<String>> {
        self.get("tx_digest")
    }

    /// Keeps the cache honest after an edit. The snapshot is no longer the one
    /// the digest was taken from, so the next verification replaces it whole.
    pub fn put_transaction(&self, row: &Transaction) -> rusqlite::Result<()> {
        let body = serde_json::to_string(row).expect("transaction serialises");
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO transactions (id, seq, body)
             VALUES (?1, (SELECT COALESCE(MAX(seq), -1) + 1 FROM transactions), ?2)
             ON CONFLICT(id) DO UPDATE SET body = excluded.body",
            params![row.id, body],
        )?;
        conn.execute("DELETE FROM kv WHERE key = 'tx_digest'", [])?;
        Ok(())
    }

    pub fn remove_transactions(&self, ids: &[i64]) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut delete = tx.prepare("DELETE FROM transactions WHERE id = ?1")?;
            for id in ids {
                delete.execute([id])?;
            }
        }
        tx.execute("DELETE FROM kv WHERE key = 'tx_digest'", [])?;
        tx.commit()
    }

    /// Empties the cache for an account that lost the right to its rows. The
    /// pages they were written on are overwritten and given back, so the file
    /// holds nothing of them even while something still keeps it open.
    pub fn wipe(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "PRAGMA secure_delete = ON;
             DELETE FROM kv;
             DELETE FROM transactions;
             PRAGMA wal_checkpoint(TRUNCATE);
             VACUUM;",
        )
    }
}

/// Deletes a cache file and what SQLite keeps beside it. `false` when one of
/// them is still there, as a file held open is on Windows.
pub fn remove_cache(path: &Path) -> bool {
    let mut gone = true;
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let file = std::path::PathBuf::from(name);
        if file.exists() && std::fs::remove_file(&file).is_err() {
            gone = false;
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(id: i64) -> Transaction {
        Transaction { id, operator_raw: Some(format!("op{id}")), ..Default::default() }
    }

    #[test]
    fn snapshot_replaces_and_keeps_order() {
        let s = Store::in_memory().unwrap();
        s.replace_transactions(&[tx(3), tx(1)], Some("sha:1"), Some("d1")).unwrap();
        s.replace_transactions(&[tx(9), tx(2), tx(5)], Some("sha:2"), Some("d2")).unwrap();
        let ids: Vec<i64> = s.load_transactions().unwrap().iter().map(|t| t.id).collect();
        assert_eq!(ids, [9, 2, 5]);
        assert_eq!(s.checksum().unwrap().as_deref(), Some("sha:2"));
        assert_eq!(s.digest().unwrap().as_deref(), Some("d2"));
    }

    #[test]
    fn single_rows_update_in_place_and_append() {
        let s = Store::in_memory().unwrap();
        s.replace_transactions(&[tx(3), tx(1)], Some("sha:1"), Some("d1")).unwrap();
        let mut edited = tx(3);
        edited.operator_raw = Some("edited".into());
        s.put_transaction(&edited).unwrap();
        s.put_transaction(&tx(7)).unwrap();
        s.remove_transactions(&[1]).unwrap();
        let rows = s.load_transactions().unwrap();
        assert_eq!(rows.iter().map(|t| t.id).collect::<Vec<_>>(), [3, 7]);
        assert_eq!(rows[0].operator_raw.as_deref(), Some("edited"));
    }

    #[test]
    fn a_local_write_forgets_what_the_snapshot_added_up_to() {
        let s = Store::in_memory().unwrap();
        s.replace_transactions(&[tx(1), tx(2)], Some("sha:1"), Some("d1")).unwrap();
        s.put_transaction(&tx(1)).unwrap();
        assert_eq!(s.digest().unwrap(), None);
        // The manifest mark stays: it is the server's word, not the rows'.
        assert_eq!(s.checksum().unwrap().as_deref(), Some("sha:1"));
        s.replace_transactions(&[tx(1), tx(2)], Some("sha:1"), Some("d1")).unwrap();
        s.remove_transactions(&[2]).unwrap();
        assert_eq!(s.digest().unwrap(), None);
    }

    #[test]
    fn survives_reopen_and_clears_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cache.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.replace_transactions(&[tx(1)], Some("a"), Some("d")).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.load_transactions().unwrap(), vec![tx(1)]);
        assert_eq!(s.digest().unwrap().as_deref(), Some("d"));
        s.replace_transactions(&[], None, None).unwrap();
        assert_eq!(s.checksum().unwrap(), None);
        assert_eq!(s.digest().unwrap(), None);
        s.set_checksum(Some("b")).unwrap();
        assert_eq!(s.checksum().unwrap().as_deref(), Some("b"));
        s.set_checksum(None).unwrap();
        assert_eq!(s.checksum().unwrap(), None);
    }

    #[test]
    fn a_cache_from_a_build_with_other_fields_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.replace_transactions(&[tx(1)], Some("a"), Some("d")).unwrap();
            // What the first builds wrote: a bare number, whatever their fields.
            s.set("schema_version", "1").unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert!(s.load_transactions().unwrap().is_empty());
        assert_eq!(s.checksum().unwrap(), None);
        assert!(schema_stamp().contains("receiver_card") && schema_stamp().contains("description") && schema_stamp().contains("excel_bank"));
        // A build that learned the stored type does not trust rows cached without it.
        assert!(schema_stamp().contains("stored_transaction_type"));
        // The same build finds its own cache again.
        s.replace_transactions(&[tx(2)], Some("b"), Some("e")).unwrap();
        drop(s);
        assert_eq!(Store::open(&path).unwrap().load_transactions().unwrap(), vec![tx(2)]);
    }

    #[test]
    fn unreadable_row_is_skipped() {
        let s = Store::in_memory().unwrap();
        s.replace_transactions(&[tx(1)], None, None).unwrap();
        s.conn.lock().unwrap().execute("INSERT INTO transactions VALUES (2, 1, 'not json')", []).unwrap();
        assert_eq!(s.load_transactions().unwrap().len(), 1);
    }

    #[test]
    fn a_wiped_cache_keeps_nothing_of_its_rows_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache-7-host.db");
        let store = Store::open(&path).unwrap();
        let mut row = tx(1);
        row.raw_message = Some("PAYMENT-MARKER-4417 for the one who lost access".repeat(40));
        store.replace_transactions(&[row], Some("sha"), Some("digest")).unwrap();
        let holds = |needle: &[u8]| {
            ["", "-wal"].iter().any(|suffix| {
                let file = dir.path().join(format!("cache-7-host.db{suffix}"));
                std::fs::read(file).is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
            })
        };
        assert!(holds(b"PAYMENT-MARKER-4417"), "the test needs the row on disk first");

        // Still open, as it is while a screen holds it: the rows go all the same.
        store.wipe().unwrap();
        assert!(store.load_transactions().unwrap().is_empty());
        assert_eq!(store.checksum().unwrap(), None);
        assert!(!holds(b"PAYMENT-MARKER-4417"), "the row is still readable in the file");

        // Closed, the files go too; another account's cache is not touched.
        let other = dir.path().join("cache-8-host.db");
        drop(Store::open(&other).unwrap());
        drop(store);
        assert!(remove_cache(&path));
        assert!(!path.exists() && !dir.path().join("cache-7-host.db-wal").exists());
        assert!(other.exists());
        assert!(remove_cache(&path), "nothing left to remove is not a failure");
    }
}
