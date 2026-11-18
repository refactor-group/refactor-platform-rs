//! Invites owed to participants, re-sent by a sweep.
//!
//! A row means one participant is owed a fresh copy of one invite: a session's own or a
//! series'. Each invite is owed to each participant at most once, however often it is
//! queued before the sweep sends it; queued while a send holds it, it is sent again. The sweep claims rows under a lease, sends, then
//! completes or releases them, so delivery is at least once.

use std::collections::BTreeSet;

use chrono::{NaiveDateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait, LockBehavior, LockType, OnConflict, Query};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityName, EntityTrait, Order,
    QueryFilter, QueryTrait,
};

use crate::coaching_session_pending_invites::{ActiveModel, Column, Entity};
use crate::coaching_session_pending_reschedule::{Released, LEASE, MAX_ATTEMPTS};
use crate::error::Error;
use crate::Id;

pub use crate::coaching_session_pending_invites::Model;

/// Which invite is owed: a session's own, or a series'.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Session(Id),
    Series(Id),
}

/// A row the sweep holds: send it, then `complete` or `release` it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claimed {
    pub id: Id,
    pub user_id: Id,
    pub target: Target,
    pub claim_id: Id,
}

/// Owes each of `targets` to each of `user_ids`. One already owed stays a single row, released
/// from any claim with a fresh retry budget, so a send already under way leaves it owed.
pub async fn queue(
    db: &impl ConnectionTrait,
    targets: &BTreeSet<Target>,
    user_ids: &[Id],
) -> Result<(), Error> {
    let now = Utc::now();
    let row = |target: Target, user_id: Id| {
        let (session, series) = match target {
            Target::Session(id) => (Some(id), None),
            Target::Series(id) => (None, Some(id)),
        };
        ActiveModel {
            id: Set(Id::new_v4()),
            user_id: Set(user_id),
            coaching_session_id: Set(session),
            coaching_session_series_id: Set(series),
            attempts: Set(0),
            claim_id: Set(None),
            claimed_at: Set(None),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
    };
    let owed = |of_kind: fn(&Target) -> bool| -> Vec<ActiveModel> {
        targets
            .iter()
            .filter(|target| of_kind(target))
            .flat_map(|&target| user_ids.iter().map(move |&user_id| row(target, user_id)))
            .collect()
    };

    for (rows, target_column) in [
        (
            owed(|target| matches!(target, Target::Session(_))),
            Column::CoachingSessionId,
        ),
        (
            owed(|target| matches!(target, Target::Series(_))),
            Column::CoachingSessionSeriesId,
        ),
    ] {
        if rows.is_empty() {
            continue;
        }
        Entity::insert_many(rows)
            .on_conflict(
                // An owed row a send holds is owed afresh: that send predates this change,
                // so its completion must not settle the row.
                OnConflict::columns([target_column, Column::UserId])
                    .value(Column::ClaimId, Expr::value(Option::<Id>::None))
                    .value(
                        Column::ClaimedAt,
                        Expr::value(Option::<NaiveDateTime>::None),
                    )
                    .value(Column::Attempts, 0)
                    .value(Column::UpdatedAt, now)
                    .to_owned(),
            )
            .exec_without_returning(db)
            .await?;
    }
    Ok(())
}

/// Claims up to `limit` rows, oldest first, skipping rows another claim holds.
pub async fn claim(
    db: &impl ConnectionTrait,
    now: NaiveDateTime,
    claim_id: Id,
    limit: u64,
) -> Result<Vec<Claimed>, Error> {
    let oldest = Query::select()
        .column(Column::Id)
        .from(Entity.table_ref())
        .cond_where(claimable(now))
        .order_by(Column::CreatedAt, Order::Asc)
        .limit(limit)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .to_owned();
    let claim = Entity::update_many()
        .col_expr(Column::ClaimId, Expr::value(claim_id))
        .col_expr(Column::ClaimedAt, Expr::value(now))
        .col_expr(Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(Column::Id.in_subquery(oldest))
        .filter(claimable(now))
        .into_query()
        .returning_all()
        .to_owned();

    let mut claimed = Entity::find()
        .from_raw_sql(db.get_database_backend().build(&claim))
        .all(db)
        .await?;
    claimed.sort_by_key(|row| row.created_at);

    Ok(claimed
        .into_iter()
        .filter_map(|row| {
            let target = row
                .coaching_session_id
                .map(Target::Session)
                .or(row.coaching_session_series_id.map(Target::Series))?;
            Some(Claimed {
                id: row.id,
                user_id: row.user_id,
                target,
                claim_id,
            })
        })
        .collect())
}

/// Drops a claimed row once its invite was sent or is no longer owed.
pub async fn complete(db: &impl ConnectionTrait, claimed: &Claimed) -> Result<(), Error> {
    Entity::delete_many().filter(held(claimed)).exec(db).await?;
    Ok(())
}

/// Hands back a claimed row the sweep did not attempt, charging it no attempt.
pub async fn hand_back(db: &impl ConnectionTrait, claimed: &Claimed) -> Result<(), Error> {
    Entity::update_many()
        .col_expr(Column::ClaimId, Expr::value(Option::<Id>::None))
        .col_expr(
            Column::ClaimedAt,
            Expr::value(Option::<NaiveDateTime>::None),
        )
        .col_expr(Column::UpdatedAt, Expr::value(Utc::now()))
        .filter(held(claimed))
        .exec(db)
        .await?;
    Ok(())
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
        .filter(Column::Id.eq(claimed.id))
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

/// Unclaimed, or its lease has run out.
fn claimable(now: NaiveDateTime) -> Condition {
    Condition::any()
        .add(Column::ClaimId.is_null())
        .add(Column::ClaimedAt.lte(now - LEASE))
}

/// The row `claimed` names, still under its claim.
fn held(claimed: &Claimed) -> Condition {
    Condition::all()
        .add(Column::Id.eq(claimed.id))
        .add(Column::ClaimId.eq(claimed.claim_id))
}
