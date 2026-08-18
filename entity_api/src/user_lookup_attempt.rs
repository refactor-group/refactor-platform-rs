use super::error::Error;

use entity::user_lookup_attempts::Model;
use entity::Id;
use sea_orm::{entity::prelude::*, ConnectionTrait};

/// Append one attempt row for `requester_user_id`.
///
/// Keyed by the requester rather than by the search term, so the limit caps how
/// fast one account can probe, whatever it probes for. `attempted_at` is left to
/// the column default so the database clock stamps it.
// Names kept for the later implementation; `todo!()` leaves them unread.
#[allow(unused_variables)]
pub async fn record(db: &impl ConnectionTrait, requester_user_id: Id) -> Result<Model, Error> {
    todo!()
}

/// Count this requester's attempts at or after `since`. Backs the windowed cap.
// Names kept for the later implementation; `todo!()` leaves them unread.
#[allow(unused_variables)]
pub async fn count_since(
    db: &impl ConnectionTrait,
    requester_user_id: Id,
    since: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    todo!()
}

/// Delete attempts older than `cutoff` across all requesters, returning the
/// number of rows deleted. Intended for periodic maintenance.
///
/// Safe to call concurrently with `record`: under MVCC a concurrent insert
/// stamped `NOW()` falls outside the `< cutoff` predicate.
// Names kept for the later implementation; `todo!()` leaves them unread.
#[allow(unused_variables)]
pub async fn delete_older_than(
    db: &impl ConnectionTrait,
    cutoff: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    todo!()
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_lookup_attempt_tests.rs"]
mod tests;
