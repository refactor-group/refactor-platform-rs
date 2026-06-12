use super::error::{EntityApiErrorKind, Error};
use entity::coaching_session_topics::{self, ActiveModel, Entity, Model, TopicSnapshot};
use entity::topic_priority::Priority;
use entity::topic_status::Status;
use entity::Id;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{NotSet, Set, Unchanged},
    DatabaseConnection, QueryOrder, TransactionTrait, TryIntoModel,
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
        .filter(coaching_session_topics::Column::DeletedAt.is_null())
        .all(db)
        .await?;
    let now = chrono::Utc::now();
    // status defaults to 'open' and moved_from_session_id to NULL via Default.
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

pub async fn find_by_id(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id)
        .filter(coaching_session_topics::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })
}

/// Like find_by_id but also returns a soft-deleted row (undo must reach a deleted topic).
pub async fn find_including_deleted_by_id(
    db: &impl ConnectionTrait,
    id: Id,
) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn update(db: &DatabaseConnection, id: Id, body: String) -> Result<Model, Error> {
    let topic = find_by_id(db, id).await?;
    let mut active: ActiveModel = topic.into();
    active.body = Set(body);
    active.undo_snapshot = Set(None);
    active.updated_at = Set(chrono::Utc::now().into());
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
    active.undo_snapshot = Set(None); // settle the undo window
    active.updated_at = Set(chrono::Utc::now().into());
    Ok(active.update(db).await?.try_into_model()?)
}

/// Sets the lifecycle status; stamps updated_at. Called only for NON-Deferred statuses
/// (the Deferred path uses defer_move/defer_hold), so this is the undo-window settle point.
pub async fn set_status(db: &impl ConnectionTrait, id: Id, status: Status) -> Result<Model, Error> {
    let topic = find_by_id(db, id).await?;
    let mut active: ActiveModel = topic.into();
    active.status = Set(status);
    active.undo_snapshot = Set(None); // settle the undo window
    active.updated_at = Set(chrono::Utc::now().into());
    Ok(active.update(db).await?.try_into_model()?)
}

pub async fn delete(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let topic = find_by_id(db, id).await?; // live rows only
    let snapshot = snapshot_for_undo(&topic);
    let mut active: ActiveModel = topic.into();
    active.deleted_at = Set(Some(chrono::Utc::now().into()));
    active.undo_snapshot = Set(Some(snapshot));
    active.update(db).await?;
    Ok(())
}

/// All topics for a session, pre-sorted in canonical wire order.
pub async fn find_by_coaching_session_id(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(coaching_session_topics::Column::CoachingSessionId.eq(coaching_session_id))
        .filter(coaching_session_topics::Column::DeletedAt.is_null())
        .order_by_asc(coaching_session_topics::Column::DisplayOrder)
        .order_by_asc(coaching_session_topics::Column::CreatedAt)
        .all(db)
        .await?)
}

/// Reassign display_order from `ordered_ids` array position. Rejects unless
/// `ordered_ids` is a permutation of the session's current topic ids. Returns
/// the reordered, pre-sorted list. The per-row updates run in a single
/// transaction so a mid-loop failure rolls back rather than leaving
/// display_order partially reassigned.
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
    let txn = db.begin().await?;
    for (index, id) in ordered_ids.iter().enumerate() {
        let active = ActiveModel {
            id: Unchanged(*id),
            display_order: Set(index as i32),
            // Deliberate non-defer write settles the undo window.
            undo_snapshot: Set(None),
            updated_at: Set(now.into()),
            ..Default::default()
        };
        active.update(&txn).await?;
    }
    txn.commit().await?;
    find_by_coaching_session_id(db, coaching_session_id).await
}

/// Captures the row's pre-mutation state so undo can restore it faithfully.
fn snapshot_for_undo(topic: &Model) -> TopicSnapshot {
    TopicSnapshot {
        coaching_session_id: topic.coaching_session_id,
        body: topic.body.clone(),
        display_order: topic.display_order,
        priority: topic.priority.clone(),
        status: topic.status.clone(),
        moved_from_session_id: topic.moved_from_session_id,
        deleted_at: topic.deleted_at,
        updated_at: topic.updated_at,
    }
}

/// Defer-forward: re-parent the topic into `target_session_id`, snapshotting its pre-defer
/// state so undo can restore it faithfully. Status -> Open, moved_from -> origin, appended.
pub async fn defer_move(
    db: &impl ConnectionTrait,
    id: Id,
    target_session_id: Id,
) -> Result<Model, Error> {
    let existing = find_by_coaching_session_id(db, target_session_id).await?;
    let topic = find_by_id(db, id).await?;
    let snapshot = snapshot_for_undo(&topic);
    let origin = topic.coaching_session_id;
    let mut active: ActiveModel = topic.into();
    active.coaching_session_id = Set(target_session_id);
    active.status = Set(Status::Open);
    active.moved_from_session_id = Set(Some(origin));
    active.display_order = Set(next_display_order(&existing));
    active.undo_snapshot = Set(Some(snapshot));
    active.updated_at = Set(chrono::Utc::now().into());
    Ok(active.update(db).await?.try_into_model()?)
}

/// Defer-hold (no next session): mark Deferred in place, snapshotting pre-defer state.
pub async fn defer_hold(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    let topic = find_by_id(db, id).await?;
    let snapshot = snapshot_for_undo(&topic);
    let mut active: ActiveModel = topic.into();
    active.status = Set(Status::Deferred);
    active.undo_snapshot = Set(Some(snapshot));
    active.updated_at = Set(chrono::Utc::now().into());
    Ok(active.update(db).await?.try_into_model()?)
}

/// Reverses any undoable op (defer or delete) by writing the captured prior row back and
/// clearing the buffer. Returns None when there is nothing to undo.
pub async fn restore_from_snapshot(
    db: &impl ConnectionTrait,
    id: Id,
) -> Result<Option<Model>, Error> {
    let topic = find_including_deleted_by_id(db, id).await?;
    let Some(snapshot) = topic.undo_snapshot.clone() else {
        return Ok(None);
    };
    let mut active: ActiveModel = topic.into();
    active.coaching_session_id = Set(snapshot.coaching_session_id);
    active.body = Set(snapshot.body);
    active.display_order = Set(snapshot.display_order);
    active.priority = Set(snapshot.priority);
    active.status = Set(snapshot.status);
    active.moved_from_session_id = Set(snapshot.moved_from_session_id);
    active.deleted_at = Set(snapshot.deleted_at); // delete-undo: NULL un-deletes; defer-undo: stays NULL
    active.updated_at = Set(snapshot.updated_at);
    active.undo_snapshot = Set(None);
    Ok(Some(active.update(db).await?.try_into_model()?))
}

/// Moves the source session's `Deferred` topics into the target session (status -> Open,
/// moved_from -> source, appended in order). One canonical row each — no copy, no dedupe
/// needed (a moved topic no longer matches the source filter on a re-run). Status filtered
/// in Rust (a PG enum in WHERE binds as text -> 42804).
pub async fn move_deferred_to_session(
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
    let mut moved = Vec::with_capacity(deferred.len());
    for (offset, topic) in deferred.into_iter().enumerate() {
        let mut active: ActiveModel = topic.into();
        active.coaching_session_id = Set(target_session_id);
        active.status = Set(Status::Open);
        active.moved_from_session_id = Set(Some(source_session_id));
        active.display_order = Set(base + offset as i32);
        // Hydration batch moves are non-undoable: clear any snapshot left by a prior
        // defer_hold so undo returns 422 instead of time-traveling to the pre-defer state.
        active.undo_snapshot = Set(None);
        active.updated_at = Set(chrono::Utc::now().into());
        moved.push(active.update(db).await?.try_into_model()?);
    }
    Ok(moved)
}

#[cfg(test)]
// Gated on the mock feature because the test file uses seaORM's MockDatabase,
// which is only available under sea-orm/mock (mirrors note.rs).
#[cfg(feature = "mock")]
#[path = "coaching_session_topic_tests.rs"]
mod tests;
