//! Process-wide registry of live documents keyed by name.
//!
//! Each entry holds a `Weak<Document>`; the strong ref count belongs to the
//! callers of `get_or_load`. When the last external `Arc<Document>` drops and
//! the doc has no joined connections, `Document::Drop` removes its own cell
//! via `forget`. `evict_now` is the explicit administrative path that also
//! flushes if the doc is still alive.

use std::sync::{Arc, Weak};
use std::time::Duration;

use dashmap::DashMap;
use parking_lot::Mutex;
use tokio::sync::OnceCell;
use tracing::warn;

use crate::document::Document;
use crate::storage::{Storage, StorageError};

const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(500);

/// Lookup-or-load entry point for `Document`s. One `Arc<Document>` per name,
/// shared across every connection currently joined to it.
pub struct DocumentRegistry {
    storage: Arc<dyn Storage>,
    persist_debounce: Duration,
    docs: DashMap<String, Arc<OnceCell<Weak<Document>>>>,
    /// Back-reference handed to each `Document` so it can self-remove on drop.
    me: Weak<DocumentRegistry>,
}

impl DocumentRegistry {
    pub fn new(storage: Arc<dyn Storage>) -> Arc<Self> {
        Self::new_with_debounce(storage, DEFAULT_DEBOUNCE)
    }

    /// Same as `new`, but pins the persist-debounce window applied to every
    /// `Document` minted through `get_or_load`. The runtime entrypoint uses this
    /// to honor `--persist-debounce-ms`; the default constructor keeps the
    /// frozen-test debounce so registry/document unit tests stay unaffected.
    pub fn new_with_debounce(storage: Arc<dyn Storage>, persist_debounce: Duration) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            storage,
            persist_debounce,
            docs: DashMap::new(),
            me: me.clone(),
        })
    }

    /// Return the shared `Arc<Document>` for `name`, loading it once on first
    /// contention. Concurrent callers converge on the same instance and
    /// trigger exactly one `Storage::fetch`. If a previously cached entry's
    /// `Weak` no longer upgrades, the stale cell is evicted and re-loaded.
    pub async fn get_or_load(&self, name: &str) -> Result<Arc<Document>, StorageError> {
        loop {
            let cell = self
                .docs
                .entry(name.to_string())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone();

            // The initializer creates the `Arc<Document>` and stores a `Weak`
            // in the `OnceCell`. We need to also hand the `Arc` back to the
            // initiating caller (the cell can't hold the strong ref or we'd
            // never auto-evict). A per-call slot does exactly that: only the
            // caller whose closure actually runs finds an `Arc` here.
            let arc_slot: Arc<Mutex<Option<Arc<Document>>>> = Arc::new(Mutex::new(None));
            let arc_slot_for_init = arc_slot.clone();
            let storage = self.storage.clone();
            let registry = self.me.clone();
            let debounce = self.persist_debounce;
            let name_for_init = name.to_string();

            let weak = cell
                .get_or_try_init(|| async move {
                    let doc =
                        Document::open_in_registry(name_for_init, storage, debounce, registry)
                            .await?;
                    let weak = Arc::downgrade(&doc);
                    *arc_slot_for_init.lock() = Some(doc);
                    Ok::<_, StorageError>(weak)
                })
                .await?;

            if let Some(arc) = arc_slot.lock().take() {
                return Ok(arc);
            }

            // The initializer ran for a peer caller; our slot is empty. The
            // peer is still mid-return holding the `Arc` strongly, so the
            // upgrade succeeds in the well-formed case.
            if let Some(arc) = weak.upgrade() {
                return Ok(arc);
            }

            // The cell outlived its `Document`. Prune by-identity and retry
            // so a concurrent fresh insert by another caller is preserved.
            self.docs
                .remove_if(name, |_, existing| Arc::ptr_eq(existing, &cell));
        }
    }

    /// Force-evict an idle document, flushing first when it is still alive.
    /// Returns true iff the registry held an entry for `name` at call time.
    /// After eviction, the registry no longer holds an entry for `name`; the
    /// next `get_or_load` mints a fresh `Document` and re-hydrates.
    pub async fn evict_now(&self, name: &str) -> Result<bool, StorageError> {
        let removed = self.docs.remove(name);
        let was_present = removed.is_some();
        let live_doc = removed.and_then(|(_, cell)| cell.get().and_then(Weak::upgrade));

        match live_doc {
            Some(doc) => doc.flush().await.map(|_| true),
            None => Ok(was_present),
        }
    }

    /// Flush every currently-live document. Used by graceful shutdown so any
    /// updates still inside a debounce window land in storage before exit.
    /// `Document::Drop` aborts the persist task without flushing, so an explicit
    /// pass here is the only thing that saves in-flight edits on stop.
    ///
    /// Best-effort: errors do not short-circuit the loop. Each doc gets a flush
    /// attempt; the first error encountered is returned, the rest are logged.
    pub async fn flush_all(&self) -> Result<(), StorageError> {
        // Snapshot strong refs out of the DashMap so we can release the shard
        // locks before any `.await`, and so the docs stay alive across flush
        // (preventing `Drop`-driven abort of their persist tasks mid-flight).
        let live: Vec<Arc<Document>> = self
            .docs
            .iter()
            .filter_map(|entry| entry.value().get().and_then(Weak::upgrade))
            .collect();

        let mut first_err: Option<StorageError> = None;
        for doc in live {
            if let Err(e) = doc.flush().await {
                warn!(name = %doc.name(), error = %e, "shutdown flush failed");
                first_err.get_or_insert(e);
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Collect the cell for `name` only if its `Document` is actually gone.
    /// Called from `Document::Drop`; this path never flushes (the doc is
    /// mid-drop and a flush would re-borrow it). The liveness guard is what
    /// makes it identity-safe: a concurrent reload may have already replaced
    /// the dead cell with a fresh live `Document` reusing this name, and an
    /// unconditional remove-by-name would orphan that live entry (split-brain).
    pub(crate) fn forget(&self, name: &str) {
        self.docs
            .remove_if(name, |_, cell| cell.get().and_then(Weak::upgrade).is_none());
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
