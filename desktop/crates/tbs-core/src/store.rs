//! Local copy of server data, so the app opens instantly and keeps reading
//! when the network drops. Nothing here is authoritative: it can be deleted
//! at any time and will be downloaded again.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::Transaction;

const SCHEMA_VERSION: i64 = 1;

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
        let version = store.get("schema_version")?.and_then(|v| v.parse::<i64>().ok());
        if version != Some(SCHEMA_VERSION) {
            store.clear()?;
            store.set("schema_version", &SCHEMA_VERSION.to_string())?;
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
    pub fn replace_transactions(&self, rows: &[Transaction], checksum: Option<&str>) -> rusqlite::Result<()> {
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
        match checksum {
            Some(c) => tx.execute("INSERT INTO kv (key, value) VALUES ('tx_checksum', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [c])?,
            None => tx.execute("DELETE FROM kv WHERE key = 'tx_checksum'", [])?,
        };
        tx.commit()
    }

    pub fn checksum(&self) -> rusqlite::Result<Option<String>> {
        self.get("tx_checksum")
    }

    /// Keeps the cache honest after an edit. The next sync sees a new server
    /// checksum and refreshes the snapshot anyway.
    pub fn put_transaction(&self, row: &Transaction) -> rusqlite::Result<()> {
        let body = serde_json::to_string(row).expect("transaction serialises");
        self.conn.lock().unwrap().execute(
            "INSERT INTO transactions (id, seq, body)
             VALUES (?1, (SELECT COALESCE(MAX(seq), -1) + 1 FROM transactions), ?2)
             ON CONFLICT(id) DO UPDATE SET body = excluded.body",
            params![row.id, body],
        )?;
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
        tx.commit()
    }
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
        s.replace_transactions(&[tx(3), tx(1)], Some("sha:1")).unwrap();
        s.replace_transactions(&[tx(9), tx(2), tx(5)], Some("sha:2")).unwrap();
        let ids: Vec<i64> = s.load_transactions().unwrap().iter().map(|t| t.id).collect();
        assert_eq!(ids, [9, 2, 5]);
        assert_eq!(s.checksum().unwrap().as_deref(), Some("sha:2"));
    }

    #[test]
    fn single_rows_update_in_place_and_append() {
        let s = Store::in_memory().unwrap();
        s.replace_transactions(&[tx(3), tx(1)], Some("sha:1")).unwrap();
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
    fn survives_reopen_and_clears_checksum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cache.sqlite");
        {
            let s = Store::open(&path).unwrap();
            s.replace_transactions(&[tx(1)], Some("a")).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.load_transactions().unwrap(), vec![tx(1)]);
        s.replace_transactions(&[], None).unwrap();
        assert_eq!(s.checksum().unwrap(), None);
    }

    #[test]
    fn unreadable_row_is_skipped() {
        let s = Store::in_memory().unwrap();
        s.replace_transactions(&[tx(1)], None).unwrap();
        s.conn.lock().unwrap().execute("INSERT INTO transactions VALUES (2, 1, 'not json')", []).unwrap();
        assert_eq!(s.load_transactions().unwrap().len(), 1);
    }
}
