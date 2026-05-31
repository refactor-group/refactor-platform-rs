//! A live, multi-connection collaborative document.
//!
//! Owns the `yrs::sync::Awareness` (which owns the `Doc`), a persistence handle,
//! and a `broadcast` channel that fans out applied updates to peer connections.

use std::sync::Arc;

use crate::storage::{Storage, StorageError};

/// Identifier tagging frames published to the per-document broadcast channel,
/// so a connection skips its own echoes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

/// Shared per-document state.
pub struct Document;

impl Document {
    /// Hydrate from storage and create the broadcast channel.
    pub async fn open(
        _name: String,
        _storage: Arc<dyn Storage>,
    ) -> Result<Arc<Self>, StorageError> {
        todo!("Document::open in Phase 5")
    }
}
