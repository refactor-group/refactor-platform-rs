use super::error::Error;

use entity::user_lookup_attempts::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::{entity::prelude::*, ConnectionTrait, DbBackend, Set, Statement};

/// Append one attempt row for `requester_user_id`.
///
/// Keyed by the requester rather than by the search term, so the limit caps how
/// fast one account can probe, whatever it probes for. `attempted_at` is left to
/// the column default so the database clock stamps it.
/// Serializes rate-limit checks for one requester within the caller's transaction.
///
/// Counting and then inserting is two statements, so without this two concurrent
/// lookups both read a count below the cap before either row lands and both proceed.
/// A burst is exactly what the cap exists to stop. The lock is keyed on the requester,
/// so different requesters never wait on each other.
///
/// Mirrors [`crate::password_reset_attempt::lock_email_hash`].
pub async fn lock_requester(
    txn: &impl ConnectionTrait,
    requester_user_id: Id,
) -> Result<(), Error> {
    let statement = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
        [requester_user_id.to_string().into()],
    );
    txn.execute(statement).await?;
    Ok(())
}

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
