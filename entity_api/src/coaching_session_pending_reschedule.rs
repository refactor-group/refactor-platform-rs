//! Unannounced schedule changes awaiting an email.
//!
//! A row means a session's schedule changed since participants were last told; no row
//! means they know the current one. A sweep claims due rows under a lease, sends, then
//! completes or releases them, so delivery is at least once. Every schedule and due-time
//! comes from the caller; only `created_at` and `updated_at` read the clock.

use chrono::{NaiveDateTime, TimeDelta, Utc};
use entity::coaching_session_pending_reschedules::{ActiveModel, Column, Entity};
use entity::{coaching_sessions, Id};
use sea_orm::sea_query::{Expr, ExprTrait, LockBehavior, LockType, OnConflict, Query};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityName, EntityTrait, Order,
    QueryFilter, QueryTrait,
};

use super::error::Error;

pub use entity::coaching_session_pending_reschedules::Model;

/// Retries allowed before a failing send is given up.
pub const MAX_ATTEMPTS: i16 = 3;

/// How long a sweep's claim holds a row.
pub const LEASE: TimeDelta = TimeDelta::minutes(2);

/// A session's start and duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub date: NaiveDateTime,
    pub duration_minutes: i16,
}

/// How long a change must be quiet, the longest it may wait, and how long a claim lasts.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub settle: TimeDelta,
    pub settle_max: TimeDelta,
    pub lease: TimeDelta,
}

/// A row the sweep holds: send it, then `complete` or `release` it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claimed {
    pub coaching_session_id: Id,
    pub claim_id: Id,
    pub revision: i32,
    pub announced: Schedule,
}

/// What recording a change left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    Pending,
    Undone,
}

/// What a delivered send left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completed {
    Settled,
    StillPending,
    Superseded,
}

/// What a failed send left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Released {
    Retrying,
    GaveUp,
    Superseded,
}

/// Records a change from `before` to `after`, dropping the row when `after` undoes it.
pub async fn record_change(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
    before: Schedule,
    after: Schedule,
    now: NaiveDateTime,
    lease: TimeDelta,
) -> Result<Recorded, Error> {
    let stamp = Utc::now();
    let row = ActiveModel {
        coaching_session_id: Set(coaching_session_id),
        announced_date: Set(before.date),
        announced_duration_minutes: Set(before.duration_minutes),
        unannounced_since: Set(now),
        changed_at: Set(now),
        revision: Set(0),
        attempts: Set(0),
        claim_id: Set(None),
        claimed_at: Set(None),
        created_at: Set(stamp.into()),
        updated_at: Set(stamp.into()),
    };
    Entity::insert(row)
        .on_conflict(
            OnConflict::column(Column::CoachingSessionId)
                .value(Column::ChangedAt, now)
                // Qualified, since Postgres reads a bare column as ambiguous with `excluded`.
                .value(
                    Column::Revision,
                    Expr::col((Entity, Column::Revision)).add(1),
                )
                // A new change earns a full retry budget.
                .value(Column::Attempts, 0)
                .value(Column::UpdatedAt, stamp)
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;

    let undone = Entity::delete_many()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .filter(Column::AnnouncedDate.eq(after.date))
        .filter(Column::AnnouncedDurationMinutes.eq(after.duration_minutes))
        .filter(claimable(now, lease))
        .exec(db)
        .await?;

    Ok(if undone.rows_affected > 0 {
        Recorded::Undone
    } else {
        Recorded::Pending
    })
}

/// Claims up to `limit` due rows, oldest first, skipping rows another claim holds.
pub async fn claim_due(
    db: &impl ConnectionTrait,
    now: NaiveDateTime,
    timing: Timing,
    claim_id: Id,
    limit: u64,
) -> Result<Vec<Claimed>, Error> {
    let eligible = Condition::all()
        .add(due(now, timing))
        .add(claimable(now, timing.lease));
    let oldest = Query::select()
        .column(Column::CoachingSessionId)
        .from(Entity.table_ref())
        .cond_where(eligible.clone())
        .order_by(Column::UnannouncedSince, Order::Asc)
        .limit(limit)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .to_owned();
    let claim = Entity::update_many()
        .col_expr(Column::ClaimId, Expr::value(claim_id))
        .col_expr(Column::ClaimedAt, Expr::value(now))
        .col_expr(Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(Column::CoachingSessionId.in_subquery(oldest))
        .filter(eligible)
        .into_query()
        .returning_all()
        .to_owned();

    let mut claimed = Entity::find()
        .from_raw_sql(db.get_database_backend().build(&claim))
        .all(db)
        .await?;
    claimed.sort_by_key(|row| row.unannounced_since);

    Ok(claimed
        .into_iter()
        .map(|row| Claimed {
            coaching_session_id: row.coaching_session_id,
            claim_id,
            revision: row.revision,
            announced: Schedule {
                date: row.announced_date,
                duration_minutes: row.announced_duration_minutes,
            },
        })
        .collect())
}

/// Settles a claimed row after `sent` was delivered, or keeps a newer change pending.
pub async fn complete(
    db: &impl ConnectionTrait,
    claimed: &Claimed,
    sent: Schedule,
) -> Result<Completed, Error> {
    let settled = Entity::delete_many()
        .filter(held(claimed))
        .filter(Column::Revision.eq(claimed.revision))
        .exec(db)
        .await?;
    if settled.rows_affected > 0 {
        return Ok(Completed::Settled);
    }

    let rebased = Entity::update_many()
        .col_expr(Column::AnnouncedDate, Expr::value(sent.date))
        .col_expr(
            Column::AnnouncedDurationMinutes,
            Expr::value(sent.duration_minutes),
        )
        .col_expr(Column::ClaimId, Expr::value(Option::<Id>::None))
        .col_expr(
            Column::ClaimedAt,
            Expr::value(Option::<NaiveDateTime>::None),
        )
        .col_expr(Column::Attempts, Expr::value(0i16))
        .col_expr(Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(held(claimed))
        .exec(db)
        .await?;

    Ok(if rebased.rows_affected > 0 {
        Completed::StillPending
    } else {
        Completed::Superseded
    })
}

/// Hands a claimed row back after a failed send, giving up after `MAX_ATTEMPTS`.
pub async fn release(db: &impl ConnectionTrait, claimed: &Claimed) -> Result<Released, Error> {
    let released = Entity::update_many()
        .col_expr(Column::ClaimId, Expr::value(Option::<Id>::None))
        .col_expr(
            Column::ClaimedAt,
            Expr::value(Option::<NaiveDateTime>::None),
        )
        .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
        .col_expr(Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(held(claimed))
        .exec(db)
        .await?;
    if released.rows_affected == 0 {
        return Ok(Released::Superseded);
    }

    let given_up = Entity::delete_many()
        .filter(Column::CoachingSessionId.eq(claimed.coaching_session_id))
        .filter(Column::ClaimId.is_null())
        .filter(Column::Attempts.gte(MAX_ATTEMPTS))
        .exec(db)
        .await?;

    Ok(if given_up.rows_affected > 0 {
        Released::GaveUp
    } else {
        Released::Retrying
    })
}

/// Drops the session's pending change, if any.
pub async fn clear(db: &impl ConnectionTrait, coaching_session_id: Id) -> Result<(), Error> {
    Entity::delete_by_id(coaching_session_id).exec(db).await?;
    Ok(())
}

/// The session's pending change, if any.
pub async fn find(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
) -> Result<Option<Model>, Error> {
    Entity::find_by_id(coaching_session_id)
        .one(db)
        .await
        .map_err(Into::into)
}

/// Unclaimed, or its lease has run out.
fn claimable(now: NaiveDateTime, lease: TimeDelta) -> Condition {
    Condition::any()
        .add(Column::ClaimId.is_null())
        .add(Column::ClaimedAt.lte(now - lease))
}

/// Quiet long enough, waiting too long, or the session starts soon.
fn due(now: NaiveDateTime, timing: Timing) -> Condition {
    let starting_soon = Query::select()
        .column(coaching_sessions::Column::Id)
        .from(coaching_sessions::Entity.table_ref())
        .and_where(coaching_sessions::Column::Date.lte(now + timing.settle_max))
        .to_owned();
    Condition::any()
        .add(Column::ChangedAt.lte(now - timing.settle))
        .add(Column::UnannouncedSince.lte(now - timing.settle_max))
        .add(Column::CoachingSessionId.in_subquery(starting_soon))
}

/// The row `claimed` names, still under its claim.
fn held(claimed: &Claimed) -> Condition {
    Condition::all()
        .add(Column::CoachingSessionId.eq(claimed.coaching_session_id))
        .add(Column::ClaimId.eq(claimed.claim_id))
}

#[cfg(all(test, feature = "mock"))]
#[path = "coaching_session_pending_reschedule_tests.rs"]
mod tests;
