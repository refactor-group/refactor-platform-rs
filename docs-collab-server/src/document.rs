//! A live, multi-connection collaborative document.
//!
//! Owns the `yrs::sync::Awareness` (which owns the `Doc`), a persistence handle,
//! and a `broadcast` channel that fans out applied updates to peer connections.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::protocol::Body;
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

    /// Document name. Stable across the document's lifetime.
    pub fn name(&self) -> &str {
        todo!("Document::name in Phase 5")
    }

    /// Register a new connection. Returns its id and a stream of broadcast
    /// frames already encoded to wire bytes. The receiver yields frames from
    /// peer connections only; the server filters this connection's own echoes
    /// by `ConnectionId`.
    pub fn join(self: &Arc<Self>) -> (ConnectionId, broadcast::Receiver<Vec<u8>>) {
        todo!("Document::join in Phase 5")
    }

    /// Drop the connection from the doc's roster. No-op if already gone.
    pub fn leave(&self, _id: ConnectionId) {
        todo!("Document::leave in Phase 5")
    }

    /// Apply a body sent by `from`. Returns any reply frames the server must
    /// send back to `from` directly (sync replies, acks). Peer fan-out happens
    /// via the broadcast channel returned by `join`, not through this vec.
    pub async fn handle(
        &self,
        _from: ConnectionId,
        _body: Body,
    ) -> Result<Vec<Body>, StorageError> {
        todo!("Document::handle in Phase 5")
    }

    /// Force-persist current state to storage. Production uses debounced
    /// write-behind plus a shutdown flush; tests call this directly.
    pub async fn flush(&self) -> Result<(), StorageError> {
        todo!("Document::flush in Phase 5")
    }
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
