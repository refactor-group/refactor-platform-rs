//! Re-exports for transcript suggestion data access.

pub use entity::transcript_suggestion::{ActiveModel, Model, Status};
pub use entity_api::transcript_suggestion::{
    create_batch, find_by_id, find_pending_by_session, transition, update_proposal,
};

#[cfg(test)]
mod sqlite_tests;
