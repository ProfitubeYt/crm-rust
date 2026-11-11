//! Long-lived secrets. Only the refresh token is persisted, and only in the
//! Windows Credential Manager. Access tokens stay in memory.

use std::collections::HashMap;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str);
    fn delete(&self, key: &str);
}

const SERVICE: &str = "TBSparcer";

#[derive(Default)]
pub struct KeyringStore;

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Option<String> {
        keyring::Entry::new(SERVICE, key).ok()?.get_password().ok()
    }

    fn set(&self, key: &str, value: &str) {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.set_password(value)) {
            Ok(()) => {}
            Err(e) => tracing::warn!("credential store write failed: {e}"),
        }
    }

    fn delete(&self, key: &str) {
        if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
            let _ = entry.delete_credential();
        }
    }
}

/// For tests and for machines where the credential store refuses to open.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }

    fn set(&self, key: &str, value: &str) {
        self.0.lock().unwrap().insert(key.to_string(), value.to_string());
    }

    fn delete(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}
