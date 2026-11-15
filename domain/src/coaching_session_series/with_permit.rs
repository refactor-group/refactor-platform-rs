//! Creating, rescheduling, cancelling and re-inviting recurring series. Each function takes the
//! caller's [`SchedulingPermit`], so code outside this crate can't change a series without first
//! proving the caller may. Each refuses (403) a permit for a different relationship than the
//! series', then acts as the permit's user.

use chrono::NaiveDateTime;
use sea_orm::DatabaseConnection;
use service::config::Config;

use super::{FollowingReschedule, Model, Recurrence, Rescheduled};
use crate::coaching_relationship::ensure_permits;
use crate::coaching_relationships::SchedulingPermit;
use crate::coaching_sessions;
use crate::duration::Duration;
use crate::error::Error;

/// Creates a recurring series and all its sessions in `permit`'s relationship, and emails both
/// participants the series invite.
pub async fn create_with_sessions(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    start_at: NaiveDateTime,
    recurrence: Recurrence,
    requested_duration: Option<Duration>,
) -> Result<(Model, Vec<coaching_sessions::Model>), Error> {
    super::create_with_sessions(
        db,
        config,
        permit.relationship().id,
        permit.user_id(),
        start_at,
        recurrence,
        requested_duration,
    )
    .await
}

/// Changes when the whole `series` recurs: replaces its future sessions with ones on the new
/// schedule and emails both participants the updated invite.
pub async fn reschedule(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    series: &Model,
    new_start_at: NaiveDateTime,
    new_recurrence: Recurrence,
    new_requested_duration: Option<Duration>,
) -> Result<Rescheduled, Error> {
    ensure_permits(permit, series.coaching_relationship_id)?;
    super::reschedule(
        db,
        config,
        series.id,
        permit.user_id(),
        new_start_at,
        new_recurrence,
        new_requested_duration,
    )
    .await
}

/// Moves one session of `series` and every session after it ("this and following"), splitting
/// the series when earlier sessions stay where they are, and emails both participants.
pub async fn reschedule_following(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    series: &Model,
    request: FollowingReschedule,
) -> Result<Rescheduled, Error> {
    ensure_permits(permit, series.coaching_relationship_id)?;
    super::reschedule_following(db, config, series.id, permit.user_id(), request).await
}

/// Cancels `series`: deletes it and its future sessions, keeps past ones, and emails both
/// participants a cancellation.
pub async fn delete_with_future_sessions(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    series: &Model,
) -> Result<(), Error> {
    ensure_permits(permit, series.coaching_relationship_id)?;
    super::delete_with_future_sessions(db, config, series.id).await
}

/// Re-sends `series`' calendar invite to both participants.
pub async fn resend_invite(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    series: &Model,
) -> Result<(), Error> {
    ensure_permits(permit, series.coaching_relationship_id)?;
    super::resend_invite(db, config, series.id).await
}
