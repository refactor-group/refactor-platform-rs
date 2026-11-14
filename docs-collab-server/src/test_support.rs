//! Shared test helpers. `#[cfg(test)]` only; not compiled into the crate.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::storage::{Storage, StorageError};

/// In-memory `Storage` that counts fetch/store/delete calls for invariant tests.
#[derive(Debug, Default)]
pub(crate) struct CountingStorage {
    inner: Mutex<HashMap<String, Vec<u8>>>,
    pub fetches: AtomicUsize,
    pub stores: AtomicUsize,
    pub deletes: AtomicUsize,
}

impl CountingStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }

    pub fn stores(&self) -> usize {
        self.stores.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Storage for CountingStorage {
    async fn fetch(&self, name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        Ok(self.inner.lock().unwrap().get(name).cloned())
    }

    async fn store(&self, name: &str, state: &[u8]) -> Result<(), StorageError> {
        self.stores.fetch_add(1, Ordering::SeqCst);
        self.inner
            .lock()
            .unwrap()
            .insert(name.to_string(), state.to_vec());
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), StorageError> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.inner.lock().unwrap().remove(name);
        Ok(())
    }
}
