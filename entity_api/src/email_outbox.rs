//! Emails owed to recipients, written in the transaction they announce and sent by a sweep.
//!
//! A sweep claims due rows under a lease, delivers them, then marks each sent, releases it
//! for a later attempt, or fails it. Every transition acts only while the caller still
//! holds the claim, so a sweep whose lease lapsed changes nothing. Every time comes from
//! the caller.

use std::collections::BTreeSet;

use chrono::{NaiveDateTime, TimeDelta};
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::sea_query::{Alias, Expr, ExprTrait, LockBehavior, LockType, Query};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityName, EntityTrait, Order,
    QueryFilter, QueryTrait, UpdateMany,
};

use crate::advisory_lock::{advisory_xact_lock, fold};
use crate::error::Error;
use crate::Id;
use entity::email_outbox::{ActiveModel, Column, Entity};

pub use entity::email_outbox::{Model, State};

/// The columns that order pending rows, most significant first; `id` breaks every tie.
const ORDER: [Column; 3] = [Column::CreatedAt, Column::Position, Column::Id];

/// The first key of every conversation lock, apart from the other two-key locks.
const CONVERSATION_LOCK_NAMESPACE: i32 = 3;

/// One email to write: what it is, the conversation it belongs to, and its request body.
#[derive(Clone, Debug, PartialEq)]
pub struct New {
    pub kind: String,
    pub subject_key: String,
    pub recipient_user_id: Id,
    pub request: serde_json::Value,
}

/// A row the sweep holds: deliver it, then `mark_sent`, `release` or `fail` it.
#[derive(Clone, Debug, PartialEq)]
pub struct Claimed {
    pub id: Id,
    pub kind: String,
    /// Attempts including this one.
    pub attempts: i16,
    /// When the row was written, naive UTC.
    pub created_at: NaiveDateTime,
    pub request: serde_json::Value,
    pub claim_id: Id,
}

/// Writes `rows` as pending, due at `now`, ordered by their position in `rows`.
pub async fn insert(
    db: &impl ConnectionTrait,
    rows: Vec<New>,
    now: NaiveDateTime,
) -> Result<(), Error> {
    if rows.is_empty() {
        return Ok(());
    }
    let stamp = utc(now);
    let models = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| ActiveModel {
            id: Set(Id::new_v4()),
            kind: Set(row.kind),
            subject_key: Set(row.subject_key),
            recipient_user_id: Set(row.recipient_user_id),
            // Batches hold a few recipients, far below the column's range.
            position: Set(i16::try_from(index).unwrap_or(i16::MAX)),
            request: Set(Some(row.request)),
            state: Set(State::Pending),
            attempts: Set(0),
            not_before: Set(now),
            claim_id: Set(None),
            lease_until: Set(None),
            last_error: Set(None),
            created_at: Set(stamp),
            updated_at: Set(stamp),
            finished_at: Set(None),
        });
    Entity::insert_many(models)
        .exec_without_returning(db)
        .await?;
    Ok(())
}

/// Holds each conversation of `rows` locked until the transaction ends, so writers to one
/// conversation commit one after the other. Locks are taken in key order, so two writers
/// never wait on each other.
pub async fn lock_conversations(db: &impl ConnectionTrait, rows: &[New]) -> Result<(), Error> {
    let keys: BTreeSet<i32> = rows
        .iter()
        .map(|row| conversation_lock_key(&row.subject_key, row.recipient_user_id))
        .collect();
    for key in keys {
        advisory_xact_lock(db, CONVERSATION_LOCK_NAMESPACE, key).await?;
    }
    Ok(())
}

/// Claims up to `limit` due rows, oldest first, skipping rows another claim holds and rows
/// waiting behind an older pending row in their conversation. Counts the attempt.
pub async fn claim(
    db: &impl ConnectionTrait,
    now: NaiveDateTime,
    lease: TimeDelta,
    limit: u64,
    claim_id: Id,
) -> Result<Vec<Claimed>, Error> {
    let eligible = Condition::all().add(due(now)).add(first_in_line());
    let oldest = Query::select()
        .column(Column::Id)
        .from(Entity.table_ref())
        .cond_where(eligible.clone())
        .order_by_columns(ORDER.map(|column| (column, Order::Asc)))
        .limit(limit)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .to_owned();
    let claim = Entity::update_many()
        .col_expr(Column::ClaimId, Expr::value(claim_id))
        .col_expr(Column::LeaseUntil, Expr::value(now + lease))
        .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
        .col_expr(Column::UpdatedAt, Expr::value(utc(now)))
        .filter(Column::Id.in_subquery(oldest))
        .filter(eligible)
        .into_query()
        .returning_all()
        .to_owned();

    let claimed = Entity::find()
        .from_raw_sql(db.get_database_backend().build(&claim))
        .all(db)
        .await?;

    Ok(claimed
        .into_iter()
        .map(|row| Claimed {
            id: row.id,
            kind: row.kind,
            attempts: row.attempts,
            created_at: row.created_at.naive_utc(),
            // A pending row always has a request; the schema enforces it.
            request: row.request.unwrap_or_default(),
            claim_id,
        })
        .collect())
}

/// Records a delivered row as sent and drops its body. False when the claim was lost.
pub async fn mark_sent(
    db: &impl ConnectionTrait,
    claimed: &Claimed,
    now: NaiveDateTime,
) -> Result<bool, Error> {
    finish(db, claimed, now, State::Sent, None).await
}

/// Records a row as failed for good and drops its body. False when the claim was lost.
pub async fn fail(
    db: &impl ConnectionTrait,
    claimed: &Claimed,
    now: NaiveDateTime,
    error: &str,
) -> Result<bool, Error> {
    finish(db, claimed, now, State::Failed, Some(error)).await
}

/// Hands a row back for another attempt at `not_before`. False when the claim was lost.
pub async fn release(
    db: &impl ConnectionTrait,
    claimed: &Claimed,
    now: NaiveDateTime,
    not_before: NaiveDateTime,
    error: &str,
) -> Result<bool, Error> {
    let released = unclaim(Entity::update_many())
        .col_expr(Column::NotBefore, Expr::value(not_before))
        .col_expr(Column::LastError, Expr::value(error))
        .col_expr(Column::UpdatedAt, Expr::value(utc(now)))
        .filter(held(claimed))
        .exec(db)
        .await?;
    Ok(released.rows_affected > 0)
}

/// Deletes rows that finished before `finished_before`.
pub async fn purge(
    db: &impl ConnectionTrait,
    finished_before: NaiveDateTime,
) -> Result<u64, Error> {
    Entity::delete_many()
        .filter(Column::FinishedAt.lt(utc(finished_before)))
        .exec(db)
        .await
        .map(|deleted| deleted.rows_affected)
        .map_err(Into::into)
}

/// Moves a held row to a finished `state`, dropping its body and claim.
async fn finish(
    db: &impl ConnectionTrait,
    claimed: &Claimed,
    now: NaiveDateTime,
    state: State,
    error: Option<&str>,
) -> Result<bool, Error> {
    let finished = unclaim(Entity::update_many())
        .col_expr(Column::State, Expr::value(state))
        .col_expr(
            Column::Request,
            Expr::value(Option::<serde_json::Value>::None),
        )
        .col_expr(Column::FinishedAt, Expr::value(utc(now)))
        .col_expr(Column::UpdatedAt, Expr::value(utc(now)));
    let finished = match error {
        Some(error) => finished.col_expr(Column::LastError, Expr::value(error)),
        None => finished,
    };
    let finished = finished.filter(held(claimed)).exec(db).await?;
    Ok(finished.rows_affected > 0)
}

/// Clears the claim and its lease.
fn unclaim(update: UpdateMany<Entity>) -> UpdateMany<Entity> {
    update
        .col_expr(Column::ClaimId, Expr::value(Option::<Id>::None))
        .col_expr(
            Column::LeaseUntil,
            Expr::value(Option::<NaiveDateTime>::None),
        )
}

/// Pending, past its `not_before`, and unclaimed or its lease has lapsed.
fn due(now: NaiveDateTime) -> Condition {
    Condition::all()
        .add(Column::State.eq(State::Pending))
        .add(Column::NotBefore.lte(now))
        .add(
            Condition::any()
                .add(Column::LeaseUntil.is_null())
                .add(Column::LeaseUntil.lte(now)),
        )
}

/// No pending row ahead of this one in `ORDER` exists for the same subject and recipient.
fn first_in_line() -> Condition {
    let older = Alias::new("older");
    let theirs = |column: Column| Expr::col((older.clone(), column));
    let ours = |column: Column| Expr::col((Entity, column));
    let ahead = Query::select()
        .expr(Expr::val(1))
        .from_as(Entity.table_ref(), older.clone())
        .and_where(theirs(Column::State).eq(State::Pending))
        .and_where(theirs(Column::SubjectKey).equals((Entity, Column::SubjectKey)))
        .and_where(theirs(Column::RecipientUserId).equals((Entity, Column::RecipientUserId)))
        .and_where(Expr::tuple(ORDER.map(theirs)).lt(Expr::tuple(ORDER.map(ours))))
        .to_owned();
    Condition::all().add(Expr::exists(ahead).not())
}

/// The second key of the lock for a subject and recipient.
fn conversation_lock_key(subject_key: &str, recipient_user_id: Id) -> i32 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let (upper, lower) = recipient_user_id.as_u64_pair();
    let hash = subject_key.bytes().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    });
    fold(hash ^ upper ^ lower)
}

/// The row `claimed` names, still under its claim.
fn held(claimed: &Claimed) -> Condition {
    Condition::all()
        .add(Column::Id.eq(claimed.id))
        .add(Column::ClaimId.eq(claimed.claim_id))
}

/// A naive UTC time as a stored timestamp with zone.
fn utc(at: NaiveDateTime) -> DateTimeWithTimeZone {
    at.and_utc().fixed_offset()
}

#[cfg(all(test, feature = "mock"))]
#[path = "email_outbox_tests.rs"]
mod tests;
