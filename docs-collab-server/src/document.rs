//! A live, multi-connection collaborative document.
//!
//! Owns the `yrs::sync::Awareness` (which owns the `Doc`), a persistence handle,
//! and a per-connection fan-out so a sender never receives its own echo.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::{broadcast, Notify};
use tokio::task::JoinHandle;
use tokio::time::{sleep_until, Instant};
use tracing::warn;
use yrs::sync::Awareness;
use yrs::updates::decoder::Decode;
use yrs::{ReadTxn, StateVector, Transact, Update};

use crate::protocol::{Body, Frame};
use crate::registry::DocumentRegistry;
use crate::storage::{Storage, StorageError};

/// Stable per-connection identifier used to skip echo when fanning out frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

const BROADCAST_CAPACITY: usize = 256;

/// Shared state poked by `handle` (and by `observe_update_v1`) and read by the
/// persist task to decide when a coalesced store should fire.
struct PersistState {
    dirty: AtomicBool,
    last_change: Mutex<Option<Instant>>,
    notify: Notify,
}

impl PersistState {
    fn new() -> Self {
        Self {
            dirty: AtomicBool::new(false),
            last_change: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    /// Mark "something changed at `now`" and wake the persist task.
    fn poke(&self) {
        self.dirty.store(true, Ordering::Release);
        *self.last_change.lock() = Some(Instant::now());
        self.notify.notify_one();
    }
}

/// Shared per-document state.
pub struct Document {
    name: String,
    awareness: Arc<Mutex<Awareness>>,
    conns: Mutex<HashMap<ConnectionId, broadcast::Sender<Vec<u8>>>>,
    next_id: AtomicU64,
    storage: Arc<dyn Storage>,
    persist: Arc<PersistState>,
    // Retained for its side effect: dropping unsubscribes the update observer.
    _update_sub: yrs::Subscription,
    persist_task: Mutex<Option<JoinHandle<()>>>,
    // Empty when this `Document` was opened outside the registry. When set,
    // `Drop` self-removes the registry's cell iff `conns` is empty, so that an
    // idle, unjoined doc is auto-evicted as its last `Arc` is released.
    registry: Weak<DocumentRegistry>,
}

impl Document {
    /// Hydrate from storage and start the write-behind task. Uses the default
    /// debounce window for persistence.
    pub async fn open(name: String, storage: Arc<dyn Storage>) -> Result<Arc<Self>, StorageError> {
        Self::open_with_debounce(name, storage, Duration::from_millis(500)).await
    }

    /// Same as `open`, but pins the persist-debounce window. Tests use this
    /// to drive deterministic coalescing under `tokio::time::pause`.
    pub async fn open_with_debounce(
        name: String,
        storage: Arc<dyn Storage>,
        persist_debounce: Duration,
    ) -> Result<Arc<Self>, StorageError> {
        Self::open_in_registry(name, storage, persist_debounce, Weak::new()).await
    }

    /// Registry-aware constructor. The `Weak<DocumentRegistry>` is used in
    /// `Drop` to self-remove the registry's cell when this doc has no joined
    /// connections, so an unused entry is collected as soon as the last
    /// external `Arc<Document>` is released.
    pub(crate) async fn open_in_registry(
        name: String,
        storage: Arc<dyn Storage>,
        persist_debounce: Duration,
        registry: Weak<DocumentRegistry>,
    ) -> Result<Arc<Self>, StorageError> {
        let doc = yrs::Doc::new();

        if let Some(bytes) = storage.fetch(&name).await? {
            let upd = Update::decode_v1(&bytes)
                .map_err(|e| StorageError::Backend(format!("hydrate decode: {e}")))?;
            doc.transact_mut()
                .apply_update(upd)
                .map_err(|e| StorageError::Backend(format!("hydrate apply: {e}")))?;
        }

        let persist = Arc::new(PersistState::new());
        let persist_for_observer = persist.clone();
        let update_sub = doc
            .observe_update_v1(move |_txn, _ev| persist_for_observer.poke())
            .map_err(|e| StorageError::Backend(format!("observe_update_v1: {e}")))?;

        let awareness = Arc::new(Mutex::new(Awareness::new(doc)));

        let persist_task = tokio::spawn(persist_loop(
            name.clone(),
            awareness.clone(),
            storage.clone(),
            persist.clone(),
            persist_debounce,
        ));

        Ok(Arc::new(Self {
            name,
            awareness,
            conns: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            storage,
            persist,
            _update_sub: update_sub,
            persist_task: Mutex::new(Some(persist_task)),
            registry,
        }))
    }

    /// Document name. Stable across the document's lifetime.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Register a new connection. The returned receiver yields wire-encoded
    /// `Frame`s from peer connections only; the sender's own frames are not
    /// delivered to its own receiver (echo-skip is per-connection by design,
    /// not a wire tag).
    pub fn join(self: &Arc<Self>) -> (ConnectionId, broadcast::Receiver<Vec<u8>>) {
        let id = ConnectionId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = broadcast::channel(BROADCAST_CAPACITY);
        self.conns.lock().insert(id, tx);
        (id, rx)
    }

    /// Drop the connection from the doc's roster. No-op if already gone.
    pub fn leave(&self, id: ConnectionId) {
        self.conns.lock().remove(&id);
    }

    /// Apply a body sent by `from`. Returns reply frames the server must send
    /// back to `from` directly (sync replies, ack). Peer fan-out happens on
    /// the per-connection broadcast channels returned by `join`.
    pub async fn handle(&self, from: ConnectionId, body: Body) -> Result<Vec<Body>, StorageError> {
        match body {
            Body::SyncStep1(client_sv) => Ok(vec![self.reply_step2(&client_sv)]),
            Body::SyncStep2(bytes) => {
                self.apply_update_bytes(&bytes);
                self.fan_out_to_peers(from, Body::Update(bytes));
                Ok(vec![])
            }
            Body::Update(bytes) => {
                self.apply_update_bytes(&bytes);
                self.fan_out_to_peers(from, Body::Update(bytes));
                Ok(vec![Body::SyncStatus(true)])
            }
            Body::Awareness(update) => {
                let _ = self.awareness.lock().apply_update(update.clone());
                self.fan_out_to_peers(from, Body::Awareness(update));
                Ok(vec![])
            }
            Body::AwarenessQuery => Ok(self.current_awareness_reply()),
            Body::AuthToken(_)
            | Body::Authenticated(_)
            | Body::PermissionDenied(_)
            | Body::Stateless(_)
            | Body::SyncStatus(_)
            | Body::Close => Ok(vec![]),
        }
    }

    /// Force-persist the current state. Production uses debounced write-behind
    /// plus a shutdown flush; tests call this directly to assert durability.
    pub async fn flush(&self) -> Result<(), StorageError> {
        let bytes = self.snapshot_state();
        self.persist.dirty.store(false, Ordering::Release);
        self.storage.store(&self.name, &bytes).await
    }

    fn reply_step2(&self, client_sv: &StateVector) -> Body {
        let aw = self.awareness.lock();
        let txn = aw.doc().transact();
        let bytes = txn.encode_state_as_update_v1(client_sv);
        Body::SyncStep2(bytes)
    }

    fn current_awareness_reply(&self) -> Vec<Body> {
        let aw = self.awareness.lock();
        aw.update().ok().map(Body::Awareness).into_iter().collect()
    }

    /// Apply update bytes if they decode; never propagate decode/apply failure
    /// to the caller (a malformed Update is not a storage error, and the
    /// debounced persist must still treat the frame as a change signal).
    fn apply_update_bytes(&self, bytes: &[u8]) {
        let _ = Update::decode_v1(bytes)
            .map_err(|e| warn!(name = %self.name, error = %e, "update decode failed"))
            .and_then(|upd| {
                self.awareness
                    .lock()
                    .doc()
                    .transact_mut()
                    .apply_update(upd)
                    .map_err(|e| warn!(name = %self.name, error = %e, "apply_update failed"))
            });
        // Treat every Update/SyncStep2 as a state-touching event so the
        // debounced persist task fires even when the bytes were rejected by
        // the CRDT layer. The observer also pokes on a successful apply; the
        // redundancy is harmless because the persist task coalesces.
        self.persist.poke();
    }

    fn fan_out_to_peers(&self, from: ConnectionId, body: Body) {
        let frame_bytes = Frame {
            name: self.name.clone(),
            body,
        }
        .encode();
        let conns = self.conns.lock();
        conns
            .iter()
            .filter(|(id, _)| **id != from)
            .for_each(|(_, tx)| {
                let _ = tx.send(frame_bytes.clone());
            });
    }

    fn snapshot_state(&self) -> Vec<u8> {
        let aw = self.awareness.lock();
        let txn = aw.doc().transact();
        txn.encode_state_as_update_v1(&StateVector::default())
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        self.persist_task
            .lock()
            .take()
            .into_iter()
            .for_each(|h| h.abort());

        // Only self-remove from the registry when there are no joined
        // connections. A still-joined doc whose last external `Arc` happens
        // to drop is treated as "in use" so a subsequent `evict_now` still
        // sees the cell and reports it as present.
        self.conns
            .lock()
            .is_empty()
            .then(|| self.registry.upgrade())
            .flatten()
            .into_iter()
            .for_each(|reg| reg.forget(&self.name));
    }
}

/// Background coalescing write-behind: waits for the first poke, then sleeps
/// until `last_change + window`. If a fresher poke arrived during the sleep,
/// re-arms; otherwise snapshots state under the awareness lock, releases it,
/// and stores. One burst inside `window` coalesces to one store.
async fn persist_loop(
    name: String,
    awareness: Arc<Mutex<Awareness>>,
    storage: Arc<dyn Storage>,
    persist: Arc<PersistState>,
    window: Duration,
) {
    loop {
        persist.notify.notified().await;

        // Wait until `window` has elapsed since the most recent poke. If a
        // fresher poke arrives while we sleep, the deadline moves forward and
        // we re-sleep. The let-binding scopes the guard to a single statement
        // so it drops before the await — a `while let` would extend the
        // temporary across the loop body and the (non-Send) MutexGuard would
        // straddle the await point.
        loop {
            let next_deadline = (*persist.last_change.lock())
                .map(|t| t + window)
                .filter(|d| Instant::now() < *d);
            let Some(deadline) = next_deadline else { break };
            sleep_until(deadline).await;
        }

        if !persist.dirty.swap(false, Ordering::AcqRel) {
            continue;
        }

        let bytes = {
            let aw = awareness.lock();
            let txn = aw.doc().transact();
            txn.encode_state_as_update_v1(&StateVector::default())
        };

        let _ = storage
            .store(&name, &bytes)
            .await
            .map_err(|e| warn!(name = %name, error = %e, "persist_loop store failed"));
    }
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
