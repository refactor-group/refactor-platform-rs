//! Process-wide registry of live documents keyed by name.

use std::sync::Arc;

use crate::document::Document;
use crate::storage::{Storage, StorageError};

/// Lookup-or-load entry point for `Document`s. One `Arc<Document>` per name,
/// shared across every connection currently joined to it.
pub struct DocumentRegistry;

impl DocumentRegistry {
    pub fn new(_storage: Arc<dyn Storage>) -> Arc<Self> {
        todo!("DocumentRegistry::new in Phase 5")
    }

    pub async fn get_or_load(&self, _name: &str) -> Result<Arc<Document>, StorageError> {
        todo!("DocumentRegistry::get_or_load in Phase 5")
    }
}
