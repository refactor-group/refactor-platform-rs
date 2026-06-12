use crate::coaching_session;
use crate::coaching_session_topics::Model;
use crate::error::{DomainErrorKind, Error};
use crate::events::{DomainEvent, EventPublisher};
use crate::topic_priority::Priority;
use crate::topic_status::Status;
use crate::Id;
use entity_api::coaching_session_topic as TopicApi;
use log::*;
use sea_orm::{DatabaseConnection, TransactionTrait};

// reads stay as direct re-exports
pub use entity_api::coaching_session_topic::{
    find_by_coaching_session_id, find_by_id, find_including_deleted_by_id,
};

/// Best-effort SSE notify for one or more sessions of the SAME coaching relationship
/// (e.g. origin + destination of a move). They share a participant set, so it is resolved
/// once from the first id and reused for every event — one round-trip, no second silent-miss.
/// The DB write is the contract; a failed lookup must NOT fail the mutation — log and continue
/// (mirrors bot_status.rs).
async fn publish_topics_changed(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    session_ids: &[Id],
) {
    let Some(&primary) = session_ids.first() else {
        return;
    };
    let notify_user_ids = match coaching_session::find_participant_ids(db, primary).await {
        Ok(ids) => ids,
        Err(e) => {
            error!("TopicsChanged: failed to resolve participants for session {primary}: {e:?}");
            return;
        }
    };
    for &coaching_session_id in session_ids {
        event_publisher
            .publish(DomainEvent::TopicsChanged {
                coaching_session_id,
                notify_user_ids: notify_user_ids.clone(),
            })
            .await;
    }
}

pub async fn create(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    coaching_session_id: Id,
    body: String,
    user_id: Id,
    priority: Option<Priority>,
) -> Result<Model, Error> {
    let topic = TopicApi::create(db, coaching_session_id, body, user_id, priority).await?;
    publish_topics_changed(db, event_publisher, &[coaching_session_id]).await;
    Ok(topic)
}

pub async fn update(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    body: String,
) -> Result<Model, Error> {
    let topic = TopicApi::update(db, id, body).await?;
    publish_topics_changed(db, event_publisher, &[topic.coaching_session_id]).await;
    Ok(topic)
}

pub async fn delete(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
) -> Result<(), Error> {
    // Capture the session id BEFORE deletion (the row is gone after).
    let coaching_session_id = TopicApi::find_by_id(db, id).await?.coaching_session_id;
    TopicApi::delete(db, id).await?;
    publish_topics_changed(db, event_publisher, &[coaching_session_id]).await;
    Ok(())
}

pub async fn reorder(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    coaching_session_id: Id,
    ordered_ids: Vec<Id>,
) -> Result<Vec<Model>, Error> {
    let topics = TopicApi::reorder(db, coaching_session_id, ordered_ids).await?;
    publish_topics_changed(db, event_publisher, &[coaching_session_id]).await;
    Ok(topics)
}

pub async fn set_priority(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    priority: Option<Priority>,
) -> Result<Model, Error> {
    let topic = TopicApi::set_priority(db, id, priority).await?;
    publish_topics_changed(db, event_publisher, &[topic.coaching_session_id]).await;
    Ok(topic)
}

pub async fn set_status(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    status: Status,
) -> Result<Model, Error> {
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    // Deferred + an existing next session => MOVE (re-parent), not a persisted Deferred.
    // Deferred + no next session => HOLD (persist Deferred; the hydration hook moves it later).
    let (result, notify_origin) = if status == Status::Deferred {
        let current = TopicApi::find_by_id(&txn, id).await?;
        let session = coaching_session::find_by_id(&txn, current.coaching_session_id).await?;
        match coaching_session::find_next_session(
            &txn,
            session.coaching_relationship_id,
            session.date,
        )
        .await?
        {
            Some(next) => (
                TopicApi::defer_move(&txn, id, next.id).await?,
                Some(session.id),
            ),
            // Already held Deferred with no next session: re-deferring would overwrite the
            // pre-defer snapshot with a Deferred self-snapshot, stranding undo. No-op instead.
            None if current.status == Status::Deferred => (current, None),
            None => (TopicApi::defer_hold(&txn, id).await?, None),
        }
    } else {
        (TopicApi::set_status(&txn, id, status).await?, None)
    };

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    // result.coaching_session_id is the destination (move) or the in-place session (hold/other);
    // the origin (move only) shares the same relationship, so both notify from one participant lookup.
    let mut sessions = vec![result.coaching_session_id];
    sessions.extend(notify_origin);
    publish_topics_changed(db, event_publisher, &sessions).await;
    Ok(result)
}

/// Reverses the most recent undoable change to a topic (a defer or a delete) by restoring
/// its pre-mutation snapshot. 422 when there is nothing to undo. Publishes topics_changed
/// for every affected session (two on a move-back, one otherwise).
pub async fn undo(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
) -> Result<Model, Error> {
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    let before = TopicApi::find_including_deleted_by_id(&txn, id).await?;
    let old_session = before.coaching_session_id;
    let Some(restored) = TopicApi::restore_from_snapshot(&txn, id).await? else {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Validation("Topic has nothing to undo.".to_string()),
        });
    };

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    // A move-back affects both sessions (same relationship -> one participant lookup serves both).
    let mut sessions = vec![restored.coaching_session_id];
    if old_session != restored.coaching_session_id {
        sessions.push(old_session);
    }
    publish_topics_changed(db, event_publisher, &sessions).await;
    Ok(restored)
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_session_topic_tests.rs"]
mod tests;
