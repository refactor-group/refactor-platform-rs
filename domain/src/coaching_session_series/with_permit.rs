//! Series writes that require a [`SchedulingPermit`]: the only way to make them from outside
//! this crate.

use chrono::NaiveDateTime;
use sea_orm::DatabaseConnection;
use service::config::Config;

use super::{FollowingReschedule, Model, Recurrence, Rescheduled};
use crate::coaching_relationship::ensure_permits;
use crate::coaching_relationships::SchedulingPermit;
use crate::coaching_sessions;
use crate::duration::Duration;
use crate::error::Error;

/// [`super::create_with_sessions`] in `permit`'s relationship.
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

/// [`super::reschedule`] by `permit`; refused for a series of another relationship.
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

/// [`super::reschedule_following`] by `permit`; refused for a series of another relationship.
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

/// [`super::delete_with_future_sessions`] by `permit`; refused for a series of another
/// relationship.
pub async fn delete_with_future_sessions(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    series: &Model,
) -> Result<(), Error> {
    ensure_permits(permit, series.coaching_relationship_id)?;
    super::delete_with_future_sessions(db, config, series.id).await
}

/// [`super::resend_invite`] by `permit`; refused for a series of another relationship.
pub async fn resend_invite(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    series: &Model,
) -> Result<(), Error> {
    ensure_permits(permit, series.coaching_relationship_id)?;
    super::resend_invite(db, config, series.id).await
}
