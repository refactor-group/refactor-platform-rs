use super::error::{EntityApiErrorKind, Error};
use entity::coaching_session_topics::{self, ActiveModel, Entity, Model};
use entity::Id;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
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
) -> Result<Model, Error> {
    let existing = Entity::find()
        .filter(coaching_session_topics::Column::CoachingSessionId.eq(coaching_session_id))
        .all(db)
        .await?;
    let now = chrono::Utc::now();
    let active = ActiveModel {
        coaching_session_id: Set(coaching_session_id),
        user_id: Set(user_id),
        body: Set(body),
        display_order: Set(next_display_order(&existing)),
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
        created_at: Unchanged(topic.created_at),
        updated_at: Set(chrono::Utc::now().into()),
    };
    Ok(active.update(db).await?.try_into_model()?)
}

pub async fn delete(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

/// All topics for a session, pre-sorted in canonical wire order.
pub async fn find_by_coaching_session_id(
    db: &DatabaseConnection,
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

#[cfg(test)]
// Gated on the mock feature because the test file uses seaORM's MockDatabase,
// which is only available under sea-orm/mock (mirrors note.rs).
#[cfg(feature = "mock")]
#[path = "coaching_session_topic_tests.rs"]
mod tests;
