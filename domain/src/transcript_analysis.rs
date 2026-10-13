//! Re-exports for transcript analysis data access.

pub use entity::transcript_analysis::{Model, Status};
pub use entity_api::transcript_analysis::{
    claim, complete, complete_claimed, create_pending, fail, fail_abandoned, find_by_id,
    find_by_transcription_and_target, find_claimable, release_for_retry, restart_failed,
};

#[cfg(test)]
#[path = "transcript_analysis_sqlite_tests.rs"]
mod sqlite_tests;
