//! Series writes that require a [`SessionManager`]: the only way to make them from outside
//! this crate.

use chrono::NaiveDateTime;
use sea_orm::DatabaseConnection;
use service::config::Config;

use super::{FollowingReschedule, Model, Recurrence, Rescheduled};
use crate::coaching_relationship::ensure_manages;
use crate::coaching_relationships::SessionManager;
use crate::coaching_sessions;
use crate::duration::Duration;
use crate::error::Error;

/// [`super::create_with_sessions`] in `manager`'s relationship.
pub async fn create_with_sessions(
    db: &DatabaseConnection,
    config: &Config,
    manager: &SessionManager,
    start_at: NaiveDateTime,
    recurrence: Recurrence,
    requested_duration: Option<Duration>,
) -> Result<(Model, Vec<coaching_sessions::Model>), Error> {
    super::create_with_sessions(
        db,
        config,
        manager.relationship().id,
        manager.user_id(),
        start_at,
        recurrence,
        requested_duration,
    )
    .await
}

/// [`super::reschedule`] by `manager`; refused for a series of another relationship.
pub async fn reschedule(
    db: &DatabaseConnection,
    config: &Config,
    manager: &SessionManager,
    series: &Model,
    new_start_at: NaiveDateTime,
    new_recurrence: Recurrence,
    new_requested_duration: Option<Duration>,
) -> Result<Rescheduled, Error> {
    ensure_manages(manager, series.coaching_relationship_id)?;
    super::reschedule(
        db,
        config,
        series.id,
        manager.user_id(),
        new_start_at,
        new_recurrence,
        new_requested_duration,
    )
    .await
}

/// [`super::reschedule_following`] by `manager`; refused for a series of another relationship.
pub async fn reschedule_following(
    db: &DatabaseConnection,
    config: &Config,
    manager: &SessionManager,
    series: &Model,
    request: FollowingReschedule,
) -> Result<Rescheduled, Error> {
    ensure_manages(manager, series.coaching_relationship_id)?;
    super::reschedule_following(db, config, series.id, manager.user_id(), request).await
}

/// [`super::delete_with_future_sessions`] by `manager`; refused for a series of another
/// relationship.
pub async fn delete_with_future_sessions(
    db: &DatabaseConnection,
    config: &Config,
    manager: &SessionManager,
    series: &Model,
) -> Result<(), Error> {
    ensure_manages(manager, series.coaching_relationship_id)?;
    super::delete_with_future_sessions(db, config, series.id).await
}

/// [`super::resend_invite`] by `manager`; refused for a series of another relationship.
pub async fn resend_invite(
    db: &DatabaseConnection,
    config: &Config,
    manager: &SessionManager,
    series: &Model,
) -> Result<(), Error> {
    ensure_manages(manager, series.coaching_relationship_id)?;
    super::resend_invite(db, config, series.id).await
}
