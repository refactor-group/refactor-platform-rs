//! Persistence of Yjs document state.
//!
//! `Storage` is the abstract interface; `MemoryStorage` backs tests,
//! `PostgresStorage` backs runtime via `sqlx`.

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("backend error: {0}")]
    Backend(String),
    #[error("not found")]
    NotFound,
}

/// Blob persistence for a single named document.
#[async_trait]
pub trait Storage: Send + Sync + 'static {
    /// Load the persisted state for `name`, or `None` if absent.
    async fn fetch(&self, name: &str) -> Result<Option<Vec<u8>>, StorageError>;

    /// Upsert the document's state bytes.
    async fn store(&self, name: &str, state: &[u8]) -> Result<(), StorageError>;

    /// Remove the row for `name`; not-found is not an error.
    async fn delete(&self, name: &str) -> Result<(), StorageError>;
}

/// In-memory storage backend for tests.
#[derive(Debug, Default)]
pub struct MemoryStorage;

impl MemoryStorage {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn fetch(&self, _name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        todo!("MemoryStorage::fetch in Phase 4")
    }

    async fn store(&self, _name: &str, _state: &[u8]) -> Result<(), StorageError> {
        todo!("MemoryStorage::store in Phase 4")
    }

    async fn delete(&self, _name: &str) -> Result<(), StorageError> {
        todo!("MemoryStorage::delete in Phase 4")
    }
}

/// PostgreSQL storage backend writing to `<schema>.collab_documents`.
pub struct PostgresStorage;

impl PostgresStorage {
    /// Connect, ensure the schema and table exist, return a ready instance.
    pub async fn connect(_database_url: &str, _schema: &str) -> Result<Self, StorageError> {
        todo!("PostgresStorage::connect in Phase 4")
    }
}

#[async_trait]
impl Storage for PostgresStorage {
    async fn fetch(&self, _name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        todo!("PostgresStorage::fetch in Phase 4")
    }

    async fn store(&self, _name: &str, _state: &[u8]) -> Result<(), StorageError> {
        todo!("PostgresStorage::store in Phase 4")
    }

    async fn delete(&self, _name: &str) -> Result<(), StorageError> {
        todo!("PostgresStorage::delete in Phase 4")
    }
}
