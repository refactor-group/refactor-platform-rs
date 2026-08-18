use super::error::Error;

use entity::user_lookup_attempts::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::{entity::prelude::*, ConnectionTrait, Set};

/// Append one attempt row for `requester_user_id`.
///
/// Keyed by the requester rather than by the search term, so the limit caps how
/// fast one account can probe, whatever it probes for. `attempted_at` is left to
/// the column default so the database clock stamps it.
pub async fn record(db: &impl ConnectionTrait, requester_user_id: Id) -> Result<Model, Error> {
    Ok(ActiveModel {
        requester_user_id: Set(requester_user_id),
        ..Default::default()
    }
    .insert(db)
    .await?)
}

/// Count this requester's attempts at or after `since`. Backs the windowed cap.
pub async fn count_since(
    db: &impl ConnectionTrait,
    requester_user_id: Id,
    since: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    let attempts = Entity::find()
        .filter(Column::RequesterUserId.eq(requester_user_id))
        .filter(Column::AttemptedAt.gte(since))
        .all(db)
        .await?;
    Ok(attempts.len() as u64)
}

/// Delete attempts older than `cutoff` across all requesters, returning the
/// number of rows deleted. Intended for periodic maintenance.
///
/// Safe to call concurrently with `record`: under MVCC a concurrent insert
/// stamped `NOW()` falls outside the `< cutoff` predicate.
pub async fn delete_older_than(
    db: &impl ConnectionTrait,
    cutoff: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    let deleted = Entity::delete_many()
        .filter(Column::AttemptedAt.lt(cutoff))
        .exec(db)
        .await?;
    Ok(deleted.rows_affected)
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_lookup_attempt_tests.rs"]
mod tests;
