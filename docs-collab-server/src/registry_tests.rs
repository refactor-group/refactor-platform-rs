//! Frozen white-box unit tests for `registry.rs`. Wired in via `#[path]`
//! from `registry.rs` so they can chmod a-w alongside `tests/`. Has access to
//! private items in this module via `use super::*;`.
//!
//! The bias-resistant gate is `tests/document_sync.rs`; this is for inner-loop
//! checks the public API cannot reach (entry refcount, weak-map cleanup,
//! race-with-load). Un-freeze with chmod +w before editing.

use std::sync::Arc;

#[allow(unused_imports)]
use super::*;
use crate::test_support::CountingStorage;

#[tokio::test]
async fn evict_drops_inner_entry_after_last_arc_release() {
    let storage = Arc::new(CountingStorage::new());
    let reg = DocumentRegistry::new(storage.clone());

    let a = reg.get_or_load("d").await.expect("first load");
    assert_eq!(storage.fetches(), 1, "first load must hit storage once");

    assert!(
        reg.evict_now("d").await.expect("evict"),
        "evict_now on a live entry must return true"
    );

    // The registry must no longer hold its internal handle for "d".
    // We still hold `a`, so the inner Document is alive in our scope, but the
    // next get_or_load must mint a brand new Arc and re-fetch from storage.
    let b = reg.get_or_load("d").await.expect("reload after evict");
    assert!(
        !Arc::ptr_eq(&a, &b),
        "evict must drop the inner entry; reload must mint a fresh Arc<Document>"
    );
    assert_eq!(
        storage.fetches(),
        2,
        "reload after evict must hit storage again"
    );

    // Second evict_now of an already-evicted name reports absent.
    drop(b);
    assert!(
        !reg.evict_now("d").await.expect("evict absent"),
        "evict_now on an absent name must return false"
    );
}

#[tokio::test]
async fn flush_all_flushes_every_live_document() {
    let storage = Arc::new(CountingStorage::new());
    let reg = DocumentRegistry::new(storage.clone());

    // Hold both Arcs so the entries stay live across flush_all; no writes, so
    // the debounced persist loop never fires and `stores` is attributable
    // solely to flush_all.
    let _a = reg.get_or_load("a").await.expect("load a");
    let _b = reg.get_or_load("b").await.expect("load b");

    reg.flush_all().await.expect("flush_all");

    assert_eq!(
        storage.stores(),
        2,
        "flush_all must flush every live doc exactly once"
    );
    assert!(
        storage.fetch("a").await.expect("fetch a").is_some(),
        "doc a must be persisted by flush_all"
    );
    assert!(
        storage.fetch("b").await.expect("fetch b").is_some(),
        "doc b must be persisted by flush_all"
    );
}

#[tokio::test]
async fn concurrent_get_or_load_does_not_double_load() {
    let storage = Arc::new(CountingStorage::new());
    let reg = DocumentRegistry::new(storage.clone());

    let (a, b) = tokio::join!(reg.get_or_load("d"), reg.get_or_load("d"));
    let a = a.expect("first concurrent load");
    let b = b.expect("second concurrent load");

    assert!(
        Arc::ptr_eq(&a, &b),
        "concurrent loaders must converge on one Arc<Document>"
    );
    assert_eq!(
        storage.fetches(),
        1,
        "first-load coalescing: exactly one Storage::fetch under contention"
    );
}
