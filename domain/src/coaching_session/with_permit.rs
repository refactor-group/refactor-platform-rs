//! Creating, editing, cancelling and re-inviting single sessions. Each function takes the
//! caller's [`SchedulingPermit`], so code outside this crate can't change a session's schedule
//! without first proving the caller may. Each refuses (403) a permit for a different
//! relationship than the session's, then acts as the permit's user.

use entity_api::mutate;
use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::coaching_relationship::ensure_permits;
use crate::coaching_relationships::SchedulingPermit;
use crate::coaching_sessions::Model;
use crate::duration::Duration;
use crate::error::Error;
use crate::events::EventPublisher;

/// Schedules a new session in `permit`'s relationship and emails both participants an invite.
/// The permit's user is recorded as who scheduled it.
pub async fn create(
    db: &DatabaseConnection,
    config: &Config,
    event_publisher: &EventPublisher,
    permit: &SchedulingPermit,
    coaching_session_model: Model,
    requested_duration: Option<Duration>,
) -> Result<Model, Error> {
    ensure_permits(permit, coaching_session_model.coaching_relationship_id)?;
    super::create(
        db,
        config,
        event_publisher,
        coaching_session_model,
        permit.user_id(),
        requested_duration,
    )
    .await
}

/// Edits `session`. Moving it or changing its length records the permit's user as who
/// rescheduled it, and the participants are emailed once the change settles.
pub async fn update(
    db: &DatabaseConnection,
    permit: &SchedulingPermit,
    session: &Model,
    params: impl mutate::IntoUpdateMap + std::fmt::Debug,
) -> Result<Model, Error> {
    ensure_permits(permit, session.coaching_relationship_id)?;
    super::update(db, session.id, permit.user_id(), params).await
}

/// Cancels `session`: deletes it and, unless it's already past, emails both participants a
/// cancellation.
pub async fn delete(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    session: &Model,
) -> Result<(), Error> {
    ensure_permits(permit, session.coaching_relationship_id)?;
    super::delete(db, config, session.id, permit.user_id()).await
}

/// Re-sends `session`'s calendar invite to both participants.
pub async fn resend_invite(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    session: &Model,
) -> Result<(), Error> {
    ensure_permits(permit, session.coaching_relationship_id)?;
    super::resend_invite(db, config, session.id).await
}
