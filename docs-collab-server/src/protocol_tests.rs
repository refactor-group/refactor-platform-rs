//! Frozen white-box unit tests for `protocol.rs`. Wired in via `#[path]`
//! from `protocol.rs` so they can chmod a-w alongside `tests/`. Has access to
//! private items in this module via `use super::*;`.
//!
//! Protocol white-box coverage now lives in `tests/protocol_conformance.rs`
//! (byte-exact round-trips against frozen fixtures). The two former white-box
//! stubs here were retired in Phase 10 once that suite proved sufficient.

#[allow(unused_imports)]
use super::*;

// RETIRED (Phase 10): SyncStatus carries only 0/1, where lib0 signed varInt and
// unsigned varUint are byte-identical, so the distinction is unobservable here.
// Covered byte-exactly by tests/protocol_conformance.rs (sync_status.bin fixture
// + round_trip_simple_bodies).

// RETIRED (Phase 10): the hand-rolled auth sub-tag path is proven byte-exact by
// tests/protocol_conformance.rs (auth_token/authenticated/permission_denied
// fixtures) and the negative case unknown_auth_subtag_is_rejected.
