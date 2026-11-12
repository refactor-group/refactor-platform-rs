//! The relationship access baseline: matrix rows from
//! `docs/test-plans/relationship_access_manual_testing.md`, sent through the production
//! route table on in-memory SQLite as each fixture caller.
//!
//! Rows assert only status and returned ids, never full bodies, and never depend on query
//! order, so moving a check between middleware and extractors cannot break them.

mod harness;

mod content_rows;
mod goal_and_topic_rows;
mod other_rows;
mod relationship_rows;
mod session_rows;
mod transcript_rows;
