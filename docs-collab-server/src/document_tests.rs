//! Frozen white-box unit tests for `document.rs`. Wired in via `#[path]`
//! from `document.rs` so they can chmod a-w alongside `tests/`. Has access to
//! private items in this module via `use super::*;`.
//!
//! The bias-resistant gate is `tests/document_sync.rs`; these are for internal
//! checks the public API cannot reach (broadcast id-tagging, observe_update
//! subscription lifetime, debounce coalescing). Un-freeze with chmod +w
//! before editing.

#[allow(unused_imports)]
use super::*;

#[test]
#[ignore = "fill in: broadcast entries must carry the originating ConnectionId so consumers can skip their own echo"]
fn broadcast_entries_carry_originating_connection_id() {}

#[test]
#[ignore = "fill in: the yrs::Subscription returned by observe_update_v1 must be retained as a named field"]
fn update_observe_subscription_outlives_first_callback() {}

#[test]
#[ignore = "fill in: a burst of N updates within the debounce window must result in a single Storage::store call"]
fn debounced_writes_coalesce_a_burst() {}
