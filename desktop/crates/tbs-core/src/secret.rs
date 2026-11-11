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
        off_runtime(|| keyring::Entry::new(SERVICE, key).ok()?.get_password().ok()).flatten()
    }

    fn set(&self, key: &str, value: &str) {
        match off_runtime(|| keyring::Entry::new(SERVICE, key).and_then(|e| e.set_password(value))) {
            Some(Ok(())) => {}
            Some(Err(e)) => tracing::warn!("credential store write failed: {e}"),
            None => tracing::warn!("credential store write failed"),
        }
    }

    fn delete(&self, key: &str) {
        off_runtime(|| {
            if let Ok(entry) = keyring::Entry::new(SERVICE, key) {
                let _ = entry.delete_credential();
            }
        });
    }
}

/// Windows answers from the Credential Manager on the spot.
#[cfg(windows)]
fn off_runtime<T>(ask: impl FnOnce() -> T) -> Option<T> {
    Some(ask())
}

/// Elsewhere the store is the Secret Service, and its client blocks on a
/// runtime of its own: on a thread that is driving ours that is a panic, in
/// the middle of a sign-in. So it is asked from a thread that drives nothing.
#[cfg(not(windows))]
fn off_runtime<T: Send>(ask: impl FnOnce() -> T + Send) -> Option<T> {
    std::thread::scope(|threads| threads.spawn(ask).join().ok())
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

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    /// What the Secret Service client does, blocking on a runtime of its own,
    /// panics on a thread that is driving one. Asked through here it does not.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_store_can_block_while_a_runtime_is_being_driven() {
        let blocking = || tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async { 7 });
        assert_eq!(off_runtime(blocking), Some(7));
    }

    #[test]
    fn a_store_that_panics_has_no_answer_and_takes_nothing_down() {
        let answer: Option<u8> = off_runtime(|| panic!("the store is gone"));
        assert_eq!(answer, None);
    }
}
