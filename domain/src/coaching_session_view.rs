use crate::error::Error;
use crate::Id;
use entity_api::coaching_session_view::mark_viewed as api_mark_viewed;
use sea_orm::DatabaseConnection;

pub use entity_api::coaching_session_view::MarkViewed;

/// Upsert the caller's view marker to now() and return the prior value. No domain event fires.
pub async fn mark_viewed(
    db: &DatabaseConnection,
    coaching_session_id: Id,
    user_id: Id,
) -> Result<MarkViewed, Error> {
    Ok(api_mark_viewed(db, coaching_session_id, user_id).await?)
}
