//! Re-exports for transcript analysis data access.

pub use entity::transcript_analysis::{Model, Status};
pub use entity_api::transcript_analysis::{
    claim, complete, create_pending, fail, find_claimable, release_for_retry,
};

#[cfg(test)]
#[path = "transcript_analysis_sqlite_tests.rs"]
mod sqlite_tests;
