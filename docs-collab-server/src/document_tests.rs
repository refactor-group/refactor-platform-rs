//! Frozen white-box unit tests for `document.rs`. Wired in via `#[path]`
//! from `document.rs` so they can chmod a-w alongside `tests/`. Has access to
//! private items in this module via `use super::*;`.
//!
//! The bias-resistant gate is `tests/document_sync.rs`; these are for internal
//! checks the public API cannot reach (broadcast id-tagging, observe_update
//! subscription lifetime, debounce coalescing). Un-freeze with chmod +w
//! before editing.

use std::sync::Arc;
use std::time::Duration;

#[allow(unused_imports)]
use super::*;
use crate::protocol::Body;
use crate::test_support::CountingStorage;
use yrs::{Text, Transact, WriteTxn};

// RETIRED (Phase 10): broadcast entries are NOT id-tagged. Echo-skip is structural
// `Document::join` gives each connection its own broadcast::Sender and fan-out
// skips the originating ConnectionId. No-self-echo is covered by
// tests/document_sync.rs::{two_clients_converge_via_document,
// awareness_updates_fan_out_to_peers}.

#[tokio::test(start_paused = true)]
async fn update_observe_subscription_outlives_first_callback() {
    let storage = Arc::new(CountingStorage::new());
    let window = Duration::from_millis(500);
    let doc = Document::open_with_debounce("d".into(), storage.clone(), window)
        .await
        .expect("open");

    // Two separate write+settle cycles. Mutate the inner Doc DIRECTLY (not via
    // handle()), so persistence can only fire through the retained observer.
    for _ in 0..2 {
        {
            let aw = doc.awareness.lock();
            let mut txn = aw.doc().transact_mut();
            let text = txn.get_or_insert_text("t");
            text.push(&mut txn, "x");
        } // drop guard + txn before advancing time
        tokio::time::advance(window + Duration::from_millis(10)).await;
        tokio::task::yield_now().await;
    }

    assert_eq!(
        storage.stores(),
        2,
        "observer must persist each update cycle; a store on the 2nd cycle proves \
         the observe_update_v1 subscription outlived the first callback"
    );
}

#[tokio::test(start_paused = true)]
async fn debounced_writes_coalesce_a_burst() {
    let storage = Arc::new(CountingStorage::new());
    let window = Duration::from_millis(500);
    let doc = Document::open_with_debounce("d".into(), storage.clone(), window)
        .await
        .expect("open");

    let (id, _rx) = doc.join();

    // Fire three updates well inside the debounce window. The exact bytes do
    // not matter for this invariant. We assert ordering of timer effects, not
    // CRDT semantics.
    let step = Duration::from_millis(50);
    for _ in 0..3 {
        doc.handle(id, Body::Update(vec![1, 2, 3]))
            .await
            .expect("handle");
        tokio::time::advance(step).await;
    }

    // The burst is well under `window`, so still zero stores.
    assert_eq!(
        storage.stores(),
        0,
        "writes inside the debounce window must not flush"
    );

    // Advance past the window from the last update; debounce must fire exactly once.
    tokio::time::advance(window + Duration::from_millis(10)).await;
    // Yield so the spawned debounce task gets a chance to run after the timer.
    tokio::task::yield_now().await;

    assert_eq!(
        storage.stores(),
        1,
        "a burst within the debounce window must coalesce to a single Storage::store"
    );
}
