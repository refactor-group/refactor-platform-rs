//! Re-exports for transcript participant data access.

pub use entity::transcript_participant::{MatchSource, Model};
pub use entity_api::transcript_participant::{create_batch, find_by_transcription};

#[cfg(test)]
#[path = "transcript_participant_sqlite_tests.rs"]
mod sqlite_tests;
