//! When each participant was last told a session's start.

use std::collections::BTreeMap;

use crate::coaching_session_notices::{ActiveModel, Column, Entity};
use chrono::Utc;
use entity::Id;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

use crate::error::Error;

pub use crate::coaching_session_notices::Model;

/// Records that each of `user_ids` was told the session's start at `at`, replacing any earlier stamp.
pub async fn stamp(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
    user_ids: &[Id],
    at: DateTimeWithTimeZone,
) -> Result<(), Error> {
    stamp_many(db, &[(coaching_session_id, at)], user_ids).await
}

/// [`stamp`] for many sessions in one statement, each `(session, at)` at its own time.
pub async fn stamp_many(
    db: &impl ConnectionTrait,
    notices: &[(Id, DateTimeWithTimeZone)],
    user_ids: &[Id],
) -> Result<(), Error> {
    // Postgres rejects an upsert that touches one row twice, as a self-coaching pair would.
    let unique: BTreeMap<(Id, Id), DateTimeWithTimeZone> = notices
        .iter()
        .flat_map(|&(session_id, at)| {
            user_ids
                .iter()
                .map(move |&user_id| ((session_id, user_id), at))
        })
        .collect();
    if unique.is_empty() {
        return Ok(());
    }

    let now: DateTimeWithTimeZone = Utc::now().into();
    let rows = unique
        .into_iter()
        .map(|((coaching_session_id, user_id), at)| ActiveModel {
            id: Set(Id::new_v4()),
            coaching_session_id: Set(coaching_session_id),
            user_id: Set(user_id),
            notice_given_at: Set(at),
            created_at: Set(now),
            updated_at: Set(now),
        });

    Entity::insert_many(rows)
        .on_conflict(
            OnConflict::columns([Column::CoachingSessionId, Column::UserId])
                .update_columns([Column::NoticeGivenAt, Column::UpdatedAt])
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;

    Ok(())
}

/// Every participant's notice row for the session.
pub async fn find_by_coaching_session(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .all(db)
        .await?)
}
