//! Session writes that require a [`SessionManager`]: the only way to make them from outside
//! this crate.

use entity_api::mutate;
use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::coaching_relationship::ensure_manages;
use crate::coaching_relationships::SessionManager;
use crate::coaching_sessions::Model;
use crate::duration::Duration;
use crate::error::Error;
use crate::events::EventPublisher;

/// [`super::create`] by `manager`; refused for a session of another relationship.
pub async fn create(
    db: &DatabaseConnection,
    config: &Config,
    event_publisher: &EventPublisher,
    manager: &SessionManager,
    coaching_session_model: Model,
    requested_duration: Option<Duration>,
) -> Result<Model, Error> {
    ensure_manages(manager, coaching_session_model.coaching_relationship_id)?;
    super::create(
        db,
        config,
        event_publisher,
        coaching_session_model,
        manager.user_id(),
        requested_duration,
    )
    .await
}

/// [`super::update`] by `manager`; refused for a session of another relationship.
pub async fn update(
    db: &DatabaseConnection,
    manager: &SessionManager,
    session: &Model,
    params: impl mutate::IntoUpdateMap + std::fmt::Debug,
) -> Result<Model, Error> {
    ensure_manages(manager, session.coaching_relationship_id)?;
    super::update(db, session.id, manager.user_id(), params).await
}

/// [`super::delete`] by `manager`; refused for a session of another relationship.
pub async fn delete(
    db: &DatabaseConnection,
    config: &Config,
    manager: &SessionManager,
    session: &Model,
) -> Result<(), Error> {
    ensure_manages(manager, session.coaching_relationship_id)?;
    super::delete(db, config, session.id).await
}

/// [`super::resend_invite`] by `manager`; refused for a session of another relationship.
pub async fn resend_invite(
    db: &DatabaseConnection,
    config: &Config,
    manager: &SessionManager,
    session: &Model,
) -> Result<(), Error> {
    ensure_manages(manager, session.coaching_relationship_id)?;
    super::resend_invite(db, config, session.id).await
}
