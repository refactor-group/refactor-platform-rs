//! Rate limiting for the authenticated email lookup.
//!
//! Defence in depth rather than a fix for a hole this change opens: the lookup's
//! reach is limited to former members of organizations the caller administers, which
//! adds no enumeration surface. It is still worth throttling an authenticated
//! endpoint that answers questions about people, and it is the prerequisite if the
//! reach is ever widened.

use sea_orm::DatabaseConnection;

use crate::error::Error;
use crate::Id;

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
/// # Errors
///
/// `UserLookupRateLimited` when the requester has already made
/// `MAX_ATTEMPTS_PER_WINDOW` lookups within `WINDOW_HOURS`.
// Names kept for the later implementation; todo!() leaves them unread.
#[allow(unused_variables)]
pub async fn record_attempt_or_reject(
    db: &DatabaseConnection,
    requester_id: Id,
) -> Result<(), Error> {
    todo!()
}
