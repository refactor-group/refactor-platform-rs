use super::error::{EntityApiErrorKind, Error};
use entity::coaching_session_topics::{self, ActiveModel, Entity, Model};
use entity::topic_priority::Priority;
use entity::topic_status::Status;
use entity::Id;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{NotSet, Set, Unchanged},
    DatabaseConnection, QueryOrder, TryIntoModel,
};
use std::collections::HashSet;

/// Next append position: max existing display_order + 1, or 0 when none.
pub(crate) fn next_display_order(existing: &[Model]) -> i32 {
    existing
        .iter()
        .map(|t| t.display_order)
        .max()
        .map_or(0, |m| m + 1)
}

/// True when `provided` is a permutation of `current` (same length, same set,
/// no duplicates) — the precondition for a reorder.
pub(crate) fn reorder_request_is_valid(current_ids: &[Id], provided_ids: &[Id]) -> bool {
    if current_ids.len() != provided_ids.len() {
        return false;
    }
    let current: HashSet<Id> = current_ids.iter().copied().collect();
    let provided: HashSet<Id> = provided_ids.iter().copied().collect();
    provided.len() == provided_ids.len() && current == provided
}

pub async fn create(
    db: &DatabaseConnection,
    coaching_session_id: Id,
    body: String,
    user_id: Id,
    priority: Option<Priority>,
) -> Result<Model, Error> {
    let existing = Entity::find()
        .filter(coaching_session_topics::Column::CoachingSessionId.eq(coaching_session_id))
        .all(db)
        .await?;
    let now = chrono::Utc::now();
    // status defaults to 'open' and carried_from_topic_id to NULL via Default.
    let active = ActiveModel {
        coaching_session_id: Set(coaching_session_id),
        user_id: Set(user_id),
        body: Set(body),
        display_order: Set(next_display_order(&existing)),
        priority: priority.map_or(NotSet, |p| Set(Some(p))),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };
    Ok(active.save(db).await?.try_into_model()?)
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn update(db: &DatabaseConnection, id: Id, body: String) -> Result<Model, Error> {
    let topic = find_by_id(db, id).await?;
    let active = ActiveModel {
        id: Unchanged(topic.id),
        coaching_session_id: Unchanged(topic.coaching_session_id),
        user_id: Unchanged(topic.user_id),
        body: Set(body),
        display_order: Unchanged(topic.display_order),
        priority: Unchanged(topic.priority),
        status: Unchanged(topic.status),
        carried_from_topic_id: Unchanged(topic.carried_from_topic_id),
        created_at: Unchanged(topic.created_at),
        updated_at: Set(chrono::Utc::now().into()),
    };
    Ok(active.update(db).await?.try_into_model()?)
}

/// Coachee-set priority. `Some` sets it, `None` clears it; stamps updated_at.
pub async fn set_priority(
    db: &DatabaseConnection,
    id: Id,
    priority: Option<Priority>,
) -> Result<Model, Error> {
    let topic = find_by_id(db, id).await?;
    let mut active: ActiveModel = topic.into();
    active.priority = Set(priority);
    active.updated_at = Set(chrono::Utc::now().into());
    Ok(active.update(db).await?.try_into_model()?)
}

/// Sets the lifecycle status; stamps updated_at.
pub async fn set_status(db: &DatabaseConnection, id: Id, status: Status) -> Result<Model, Error> {
    let topic = find_by_id(db, id).await?;
    let mut active: ActiveModel = topic.into();
    active.status = Set(status);
    active.updated_at = Set(chrono::Utc::now().into());
    Ok(active.update(db).await?.try_into_model()?)
}

pub async fn delete(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

/// All topics for a session, pre-sorted in canonical wire order.
pub async fn find_by_coaching_session_id(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(coaching_session_topics::Column::CoachingSessionId.eq(coaching_session_id))
        .order_by_asc(coaching_session_topics::Column::DisplayOrder)
        .order_by_asc(coaching_session_topics::Column::CreatedAt)
        .all(db)
        .await?)
}

/// Reassign display_order from `ordered_ids` array position. Rejects unless
/// `ordered_ids` is a permutation of the session's current topic ids. Returns
/// the reordered, pre-sorted list. Non-transactional (see handoff).
pub async fn reorder(
    db: &DatabaseConnection,
    coaching_session_id: Id,
    ordered_ids: Vec<Id>,
) -> Result<Vec<Model>, Error> {
    let current = find_by_coaching_session_id(db, coaching_session_id).await?;
    let current_ids: Vec<Id> = current.iter().map(|t| t.id).collect();
    if !reorder_request_is_valid(&current_ids, &ordered_ids) {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::TopicReorderMismatch,
        });
    }
    let now = chrono::Utc::now();
    for (index, id) in ordered_ids.iter().enumerate() {
        let active = ActiveModel {
            id: Unchanged(*id),
            display_order: Set(index as i32),
            updated_at: Set(now.into()),
            ..Default::default()
        };
        active.update(db).await?;
    }
    find_by_coaching_session_id(db, coaching_session_id).await
}

/// Copies the source session's `Deferred` topics into the target session,
/// preserving body/priority/author, resetting status to Open, appending after any
/// existing target topics, and stamping carried_from_topic_id with the source id.
/// Returns the created copies in carry order. Status filtering is done in Rust;
/// filtering by a PG enum in SQL binds as text and Postgres rejects it (42804).
pub async fn carry_over(
    db: &impl ConnectionTrait,
    source_session_id: Id,
    target_session_id: Id,
) -> Result<Vec<Model>, Error> {
    let deferred: Vec<Model> = find_by_coaching_session_id(db, source_session_id)
        .await?
        .into_iter()
        .filter(|topic| topic.status == Status::Deferred)
        .collect();

    let base = next_display_order(&find_by_coaching_session_id(db, target_session_id).await?);

    let mut carried = Vec::with_capacity(deferred.len());
    for (offset, source) in deferred.into_iter().enumerate() {
        let now = chrono::Utc::now();
        let copy = ActiveModel {
            coaching_session_id: Set(target_session_id),
            user_id: Set(source.user_id),
            body: Set(source.body),
            display_order: Set(base + offset as i32),
            priority: Set(source.priority),
            status: Set(Status::Open),
            carried_from_topic_id: Set(Some(source.id)),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            ..Default::default()
        };
        carried.push(copy.save(db).await?.try_into_model()?);
    }
    Ok(carried)
}

#[cfg(test)]
// Gated on the mock feature because the test file uses seaORM's MockDatabase,
// which is only available under sea-orm/mock (mirrors note.rs).
#[cfg(feature = "mock")]
#[path = "coaching_session_topic_tests.rs"]
mod tests;
