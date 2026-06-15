// Pure passthrough to entity_api: no domain event, validation, or orchestration,
// so re-export rather than wrap (see coding-standards "Domain re-exports vs. custom wrappers").
pub use entity_api::coaching_session_view::{mark_viewed, MarkViewed};
