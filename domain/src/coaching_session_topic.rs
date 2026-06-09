use crate::coaching_session;
use crate::coaching_session_topics::Model;
use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::topic_priority::Priority;
use crate::topic_status::Status;
use crate::Id;
use entity_api::coaching_session_topic as TopicApi;
use log::*;
use sea_orm::{DatabaseConnection, TransactionTrait};

// reads stay as direct re-exports
pub use entity_api::coaching_session_topic::{find_by_coaching_session_id, find_by_id};

/// Best-effort SSE notify. The DB write is the contract; a failure to resolve
/// participants must NOT fail the mutation — log and continue (mirrors bot_status.rs).
async fn publish_topics_changed(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    coaching_session_id: Id,
) {
    match coaching_session::find_participant_ids(db, coaching_session_id).await {
        Ok(notify_user_ids) => {
            event_publisher
                .publish(DomainEvent::TopicsChanged {
                    coaching_session_id,
                    notify_user_ids,
                })
                .await;
        }
        Err(e) => error!(
            "TopicsChanged: failed to resolve participants for session {coaching_session_id}: {e:?}"
        ),
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
    publish_topics_changed(db, event_publisher, coaching_session_id).await;
    Ok(topic)
}

pub async fn update(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    body: String,
) -> Result<Model, Error> {
    let topic = TopicApi::update(db, id, body).await?;
    publish_topics_changed(db, event_publisher, topic.coaching_session_id).await;
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
    publish_topics_changed(db, event_publisher, coaching_session_id).await;
    Ok(())
}

pub async fn reorder(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    coaching_session_id: Id,
    ordered_ids: Vec<Id>,
) -> Result<Vec<Model>, Error> {
    let topics = TopicApi::reorder(db, coaching_session_id, ordered_ids).await?;
    publish_topics_changed(db, event_publisher, coaching_session_id).await;
    Ok(topics)
}

pub async fn set_priority(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    priority: Option<Priority>,
) -> Result<Model, Error> {
    let topic = TopicApi::set_priority(db, id, priority).await?;
    publish_topics_changed(db, event_publisher, topic.coaching_session_id).await;
    Ok(topic)
}

pub async fn set_status(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    status: Status,
) -> Result<Model, Error> {
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    let topic = TopicApi::set_status(&txn, id, status).await?;

    // On defer, eagerly carry forward into the already-existing next session (if any).
    // The hydration-time task still covers the "next session not created yet" case;
    // carry_over dedupes on carried_from_topic_id, so the two paths never double-copy.
    let carried_target = if topic.status == Status::Deferred {
        let session = coaching_session::find_by_id(&txn, topic.coaching_session_id).await?;
        match coaching_session::find_next_session(
            &txn,
            session.coaching_relationship_id,
            session.date,
        )
        .await?
        {
            Some(next) => {
                let carried = TopicApi::carry_over(&txn, session.id, next.id).await?;
                (!carried.is_empty()).then_some(next.id)
            }
            None => None,
        }
    } else {
        None
    };

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    // Publish after commit so subscribers never refetch against a rolled-back copy.
    publish_topics_changed(db, event_publisher, topic.coaching_session_id).await;
    if let Some(target_id) = carried_target {
        publish_topics_changed(db, event_publisher, target_id).await;
    }
    Ok(topic)
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_session_topic_tests.rs"]
mod tests;
