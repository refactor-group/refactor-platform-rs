//! Frozen white-box unit tests for `protocol.rs`. Wired in via `#[path]`
//! from `protocol.rs` so they can chmod a-w alongside `tests/`. Has access to
//! private items in this module via `use super::*;`.
//!
//! The bias-resistant gate is `tests/protocol_conformance.rs`; these are for
//! inner-loop checks that the public Frame API cannot reach (private helpers,
//! codec variant choice, byte-budget bounds). Filled in alongside the
//! implementation; un-freeze with chmod +w before editing.

#[allow(unused_imports)]
use super::*;

#[test]
#[ignore = "fill in: SyncStatus payload must be varInt (signed) not varUint"]
fn sync_status_uses_var_int_not_var_uint() {}

#[test]
#[ignore = "fill in: Auth sub-tag dispatch is hand-rolled, not via yrs::sync::Message::Auth"]
fn auth_sub_tag_path_is_separate_from_yrs_sync_message_auth() {}
