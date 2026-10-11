use entity::transcript_suggestion::{ActiveModel, Column, Entity, Model, Status};
use entity::transcript_suggestion_target::Target;
use entity::Id;
use log::debug;
use sea_orm::{
    entity::prelude::*, ActiveValue::Set, ColumnTrait, ConnectionTrait, Order, QueryOrder,
};

use super::error::{EntityApiErrorKind, Error};

/// Inserts suggestions in one statement, returned in input order; an empty batch is a no-op.
pub async fn create_batch<C: ConnectionTrait>(
    db: &C,
    rows: Vec<ActiveModel>,
) -> Result<Vec<Model>, Error> {
    debug!("Inserting {} transcript suggestions", rows.len());

    if rows.is_empty() {
        return Ok(vec![]);
    }

    let ids: Vec<Id> = rows
        .iter()
        .filter_map(|r| r.id.try_as_ref().copied())
        .collect();
    let mut inserted = Entity::insert_many(rows).exec_with_returning(db).await?;
    // RETURNING order is not guaranteed, so restore input order by id.
    inserted.sort_by_key(|m| ids.iter().position(|id| *id == m.id));
    Ok(inserted)
}

pub async fn find_by_id<C: ConnectionTrait>(db: &C, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Pending suggestions of a session and target, oldest first.
pub async fn find_pending_by_session<C: ConnectionTrait>(
    db: &C,
    coaching_session_id: Id,
    target: Target,
) -> Result<Vec<Model>, Error> {
    find_by_session(db, coaching_session_id, target, Status::Pending).await
}

/// Suggestions of a session, target and status, oldest first.
pub async fn find_by_session<C: ConnectionTrait>(
    db: &C,
    coaching_session_id: Id,
    target: Target,
    status: Status,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .filter(Column::Target.eq(target))
        .filter(Column::Status.eq(status))
        .order_by(Column::CreatedAt, Order::Asc)
        .order_by(Column::Id, Order::Asc)
        .all(db)
        .await?)
}

/// Edits a pending suggestion's proposal; `None` when it is not pending.
pub async fn update_proposal<C: ConnectionTrait>(
    db: &C,
    id: Id,
    proposal: Json,
    now: DateTimeWithTimeZone,
) -> Result<Option<Model>, Error> {
    Ok(Entity::update_many()
        .set(ActiveModel {
            proposal: Set(proposal),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(Column::Id.eq(id))
        .filter(Column::Status.eq(Status::Pending))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next())
}

/// Moves a suggestion `from` one status `to` another in one conditional UPDATE.
///
/// Returns `None` when the row is not in `from`, which guards double accepts.
/// Restoring to pending clears who resolved it and the entity it created.
pub async fn transition<C: ConnectionTrait>(
    db: &C,
    id: Id,
    from: Status,
    to: Status,
    actor: Id,
    created_entity_id: Option<Id>,
    now: DateTimeWithTimeZone,
) -> Result<Option<Model>, Error> {
    let resolution = match to {
        Status::Pending => ActiveModel {
            resolved_by: Set(None),
            resolved_at: Set(None),
            created_entity_id: Set(None),
            ..Default::default()
        },
        Status::Accepted | Status::Dismissed => ActiveModel {
            resolved_by: Set(Some(actor)),
            resolved_at: Set(Some(now)),
            created_entity_id: Set(created_entity_id),
            ..Default::default()
        },
    };

    Ok(Entity::update_many()
        .set(ActiveModel {
            status: Set(to),
            updated_at: Set(now),
            ..resolution
        })
        .filter(Column::Id.eq(id))
        .filter(Column::Status.eq(from))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next())
}
