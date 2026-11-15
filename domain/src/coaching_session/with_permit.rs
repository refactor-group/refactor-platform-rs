//! Session writes that require a [`SchedulingPermit`]: the only way to make them from outside
//! this crate.

use entity_api::mutate;
use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::coaching_relationship::ensure_permits;
use crate::coaching_relationships::SchedulingPermit;
use crate::coaching_sessions::Model;
use crate::duration::Duration;
use crate::error::Error;
use crate::events::EventPublisher;

/// [`super::create`] by `permit`; refused for a session of another relationship.
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

/// [`super::update`] by `permit`; refused for a session of another relationship.
pub async fn update(
    db: &DatabaseConnection,
    permit: &SchedulingPermit,
    session: &Model,
    params: impl mutate::IntoUpdateMap + std::fmt::Debug,
) -> Result<Model, Error> {
    ensure_permits(permit, session.coaching_relationship_id)?;
    super::update(db, session.id, permit.user_id(), params).await
}

/// [`super::delete`] by `permit`; refused for a session of another relationship.
pub async fn delete(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    session: &Model,
) -> Result<(), Error> {
    ensure_permits(permit, session.coaching_relationship_id)?;
    super::delete(db, config, session.id).await
}

/// [`super::resend_invite`] by `permit`; refused for a session of another relationship.
pub async fn resend_invite(
    db: &DatabaseConnection,
    config: &Config,
    permit: &SchedulingPermit,
    session: &Model,
) -> Result<(), Error> {
    ensure_permits(permit, session.coaching_relationship_id)?;
    super::resend_invite(db, config, session.id).await
}
