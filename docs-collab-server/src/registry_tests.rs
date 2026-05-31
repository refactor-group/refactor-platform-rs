//! Frozen white-box unit tests for `registry.rs`. Wired in via `#[path]`
//! from `registry.rs` so they can chmod a-w alongside `tests/`. Has access to
//! private items in this module via `use super::*;`.
//!
//! The bias-resistant gate is `tests/document_sync.rs`; this is for inner-loop
//! checks the public API cannot reach (entry refcount, weak-map cleanup,
//! race-with-load). Un-freeze with chmod +w before editing.

#[allow(unused_imports)]
use super::*;

#[test]
#[ignore = "fill in: after the last Arc<Document> drops and evict runs, the inner entry must be gone"]
fn evict_drops_inner_entry_after_last_arc_release() {}

#[test]
#[ignore = "fill in: two get_or_load(N) calls in flight must trigger exactly one Storage::fetch"]
fn concurrent_get_or_load_does_not_double_load() {}
