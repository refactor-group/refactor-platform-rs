//! Rate limiting for the authenticated email lookup.
//!
//! Defence in depth rather than a fix for a hole this change opens: the lookup's
//! reach is limited to former members of organizations the caller administers, which
//! adds no enumeration surface. It is still worth throttling an authenticated
//! endpoint that answers questions about people, and it is the prerequisite if the
//! reach is ever widened.

use chrono::{Duration, Utc};
use entity_api::error::Error as EntityApiError;
use entity_api::user_lookup_attempt;
use sea_orm::{DatabaseConnection, TransactionTrait};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::{user_lookup_attempts, Id};

/// Lookups one requester may make per window before being refused.
///
/// Sized for a human working a member picker, not for a script. Keyed on the
/// requester rather than the email or the IP: the endpoint is authenticated, so the
/// requester is the thing being throttled.
pub const MAX_ATTEMPTS_PER_WINDOW: u64 = 30;

/// The window `MAX_ATTEMPTS_PER_WINDOW` is counted over.
pub const WINDOW_HOURS: i64 = 1;

/// Records this lookup, or refuses it when the requester is over their allowance.
///
/// Counts first and records only on the allowed path, so a refused attempt does not
/// extend the window it was refused by.
///
/// Runs inside a transaction holding an advisory lock on the requester. Counting and
/// inserting are two statements, so without the lock a burst of concurrent lookups
/// all read a count below the cap before any row lands and all proceed, which is the
/// case the cap exists to stop. Different requesters never wait on each other.
///
/// # Errors
///
/// `UserLookupRateLimited` when the requester has already made
/// `MAX_ATTEMPTS_PER_WINDOW` lookups within `WINDOW_HOURS`.
pub async fn record_attempt_or_reject(
    db: &DatabaseConnection,
    requester_id: Id,
) -> Result<(), Error> {
    let txn = db.begin().await.map_err(EntityApiError::from)?;

    user_lookup_attempt::lock_requester(&txn, requester_id).await?;

    let attempts = user_lookup_attempt::count_since(
        &txn,
        requester_id,
        (Utc::now() - Duration::hours(WINDOW_HOURS)).into(),
    )
    .await?;

    if attempts >= MAX_ATTEMPTS_PER_WINDOW {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::UserLookupRateLimited,
            )),
        });
    }

    user_lookup_attempt::record(&txn, requester_id).await?;
    txn.commit().await.map_err(EntityApiError::from)?;

    Ok(())
}

/// Deletes attempt rows older than `retention_hours`, returning how many went.
///
/// The table is written on every lookup and read only within a one-hour window, so
/// without this it grows for the life of the deployment. It is why the table is not
/// append-only: `user_role_changes` revokes DELETE because it is a permanent record,
/// this one is throttle state and must be prunable.
///
/// Retention is deliberately longer than the rate-limit window, so a sweep landing
/// mid-window cannot delete rows the next check still needs to count.
///
/// # Errors
///
/// `Validation` when `retention_hours` is shorter than the rate-limit window.
pub async fn sweep_old_attempts(
    db: &DatabaseConnection,
    retention_hours: i64,
) -> Result<u64, Error> {
    if retention_hours <= WINDOW_HOURS {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation(format!(
                "retention_hours must exceed the {WINDOW_HOURS}-hour rate-limit window \
                 (got {retention_hours}); shorter retention would purge rows the cap \
                 still needs to count"
            )),
        });
    }

    let cutoff = (Utc::now() - Duration::hours(retention_hours)).into();
    Ok(user_lookup_attempt::delete_older_than(db, cutoff).await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_lookup_tests.rs"]
mod tests;
