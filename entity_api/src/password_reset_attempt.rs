use super::error::Error;

use chrono::Utc;
use entity::password_reset_attempts::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::{entity::prelude::*, ConnectionTrait, QueryOrder, Set};

/// Append an attempt row for a user. This table is the source of truth for
/// rate-limiting; it is intentionally append-only (no UPDATE / DELETE on
/// the request path) so the row count over a time window is meaningful.
pub async fn record(db: &impl ConnectionTrait, user_id: Id) -> Result<Model, Error> {
    let active_model = ActiveModel {
        user_id: Set(user_id),
        attempted_at: Set(Utc::now().into()),
        ..Default::default()
    };
    Ok(active_model.insert(db).await?)
}

/// Latest attempt for a user, if any. Used for the min-interval rate-limit
/// check ("no new request within N seconds of the previous one").
pub async fn find_most_recent(
    db: &impl ConnectionTrait,
    user_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::UserId.eq(user_id))
        .order_by_desc(Column::AttemptedAt)
        .one(db)
        .await?)
}

/// Count attempts for a user since `since`. Used for the daily-cap rate
/// limit ("no more than N requests in the last 24h").
///
/// Implemented as `.all().len()` rather than SQL `COUNT(*)` for two reasons:
/// (1) the upper bound is the rate-limit cap itself (5 per 24h), so the
/// returned row count is tiny in practice; (2) it makes the function
/// straightforward to mock against `MockDatabase` in unit tests. The
/// composite `(user_id, attempted_at DESC)` index keeps the query cheap.
pub async fn count_since(
    db: &impl ConnectionTrait,
    user_id: Id,
    since: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    let rows = Entity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::AttemptedAt.gte(since))
        .all(db)
        .await?;
    Ok(rows.len() as u64)
}

/// Delete attempts older than `cutoff` across all users. Returns the number
/// of rows deleted.
///
/// Intended for periodic maintenance — see `domain::password_reset::sweep_old_attempts`
/// for the recommended wrapper, retention policy, and call pattern.
///
/// **Safe to call concurrently** with `record()`: under MVCC, a concurrent
/// INSERT with `attempted_at = NOW()` falls outside the `< cutoff` predicate
/// and is unaffected.
///
/// For ad-hoc inspection ("how many rows would I delete?") run the equivalent
/// `SELECT count(*) FROM refactor_platform.password_reset_attempts
///  WHERE attempted_at < <cutoff>` via psql — there is no dry-run mode here.
pub async fn delete_older_than(
    db: &impl ConnectionTrait,
    cutoff: DateTimeWithTimeZone,
) -> Result<u64, Error> {
    let result = Entity::delete_many()
        .filter(Column::AttemptedAt.lt(cutoff))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}
