use entity::transcript_analysis::{ActiveModel, Column, Entity, Model, Status};
use entity::transcript_suggestion_target::Target;
use entity::Id;
use log::debug;
use sea_orm::{
    entity::prelude::*,
    sea_query::{Expr, ExprTrait},
    ActiveValue::Set,
    ColumnTrait, Condition, ConnectionTrait, Order, QueryOrder, QuerySelect,
};

use super::error::{EntityApiErrorKind, Error};

/// The row `id` while it is still claimed by the worker that counted `attempts`.
fn held(id: Id, attempts: i16) -> Condition {
    Condition::all()
        .add(Column::Id.eq(id))
        .add(Column::Status.eq(Status::Processing))
        .add(Column::Attempts.eq(attempts))
}

/// Rows a worker may take: pending, or processing past `stale_before`, with attempts left.
fn claimable(stale_before: DateTimeWithTimeZone, max_attempts: i16) -> Condition {
    Condition::all().add(Column::Attempts.lt(max_attempts)).add(
        Condition::any()
            .add(Column::Status.eq(Status::Pending))
            .add(
                Condition::all()
                    .add(Column::Status.eq(Status::Processing))
                    .add(Column::ClaimedAt.lt(stale_before)),
            ),
    )
}

/// The analyses with these ids, in one query; ids with no analysis are skipped.
pub async fn find_by_ids<C: ConnectionTrait>(db: &C, ids: Vec<Id>) -> Result<Vec<Model>, Error> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    Ok(Entity::find().filter(Column::Id.is_in(ids)).all(db).await?)
}

/// The analysis with `id`; a `RecordNotFound` error when there is none.
pub async fn find_by_id<C: ConnectionTrait>(db: &C, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Records a pending analysis for a transcription and target.
pub async fn create_pending<C: ConnectionTrait>(
    db: &C,
    transcription_id: Id,
    target: Target,
) -> Result<Model, Error> {
    debug!("Creating pending {target} analysis for transcription {transcription_id}");

    let now = chrono::Utc::now();
    Ok(ActiveModel {
        id: Set(Id::new_v4()),
        transcription_id: Set(transcription_id),
        target: Set(target),
        status: Set(Status::Pending),
        attempts: Set(0),
        claimed_at: Set(None),
        error_message: Set(None),
        model: Set(None),
        input_tokens: Set(None),
        output_tokens: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await?)
}

/// The transcription's analysis for `target`, if one was recorded.
pub async fn find_by_transcription_and_target<C: ConnectionTrait>(
    db: &C,
    transcription_id: Id,
    target: Target,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::TranscriptionId.eq(transcription_id))
        .filter(Column::Target.eq(target))
        .one(db)
        .await?)
}

/// Claimable analyses, oldest first, at most `limit`.
pub async fn find_claimable<C: ConnectionTrait>(
    db: &C,
    stale_before: DateTimeWithTimeZone,
    max_attempts: i16,
    limit: u64,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(claimable(stale_before, max_attempts))
        .order_by(Column::CreatedAt, Order::Asc)
        .limit(limit)
        .all(db)
        .await?)
}

/// Takes one analysis in a single conditional UPDATE, so of racing callers at most one gets `Some`.
pub async fn claim<C: ConnectionTrait>(
    db: &C,
    id: Id,
    now: DateTimeWithTimeZone,
    stale_before: DateTimeWithTimeZone,
    max_attempts: i16,
) -> Result<Option<Model>, Error> {
    Ok(Entity::update_many()
        .set(ActiveModel {
            status: Set(Status::Processing),
            claimed_at: Set(Some(now)),
            updated_at: Set(now),
            ..Default::default()
        })
        .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
        .filter(Column::Id.eq(id))
        .filter(claimable(stale_before, max_attempts))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next())
}

/// Marks an analysis completed and stores the model and token usage.
pub async fn complete<C: ConnectionTrait>(
    db: &C,
    id: Id,
    model: String,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
) -> Result<Model, Error> {
    let existing = find_by_id(db, id).await?;

    Ok(ActiveModel {
        status: Set(Status::Completed),
        model: Set(Some(model)),
        input_tokens: Set(input_tokens),
        output_tokens: Set(output_tokens),
        error_message: Set(None),
        updated_at: Set(chrono::Utc::now().into()),
        ..existing.into()
    }
    .update(db)
    .await?)
}

/// Completes an analysis only while it is still the caller's claim.
///
/// A worker whose claim went stale and was retaken sees a different `attempts` and gets `None`.
pub async fn complete_claimed<C: ConnectionTrait>(
    db: &C,
    id: Id,
    attempts: i16,
    model: String,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    now: DateTimeWithTimeZone,
) -> Result<Option<Model>, Error> {
    Ok(Entity::update_many()
        .set(ActiveModel {
            status: Set(Status::Completed),
            model: Set(Some(model)),
            input_tokens: Set(input_tokens),
            output_tokens: Set(output_tokens),
            error_message: Set(None),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(held(id, attempts))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next())
}

/// Returns the caller's claimed analysis to pending after a retryable failure; attempts stay as
/// counted.
///
/// `None` when the claim is no longer the caller's, so a worker whose claim was retaken never
/// undoes the other worker's result.
pub async fn release_for_retry<C: ConnectionTrait>(
    db: &C,
    id: Id,
    attempts: i16,
    error: String,
    now: DateTimeWithTimeZone,
) -> Result<Option<Model>, Error> {
    Ok(Entity::update_many()
        .set(ActiveModel {
            status: Set(Status::Pending),
            error_message: Set(Some(error)),
            claimed_at: Set(None),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(held(id, attempts))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next())
}

/// Marks the caller's claimed analysis failed for good.
///
/// `None` when the claim is no longer the caller's.
pub async fn fail<C: ConnectionTrait>(
    db: &C,
    id: Id,
    attempts: i16,
    error: String,
    now: DateTimeWithTimeZone,
) -> Result<Option<Model>, Error> {
    Ok(Entity::update_many()
        .set(ActiveModel {
            status: Set(Status::Failed),
            error_message: Set(Some(error)),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(held(id, attempts))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next())
}

/// Fails, in one UPDATE, every analysis whose worker stopped on its last attempt: still
/// `processing`, with no attempts left, claimed before `stale_before`. Nothing would ever retake
/// these, so failing them lets a user start the scan again. Returns the rows it failed.
pub async fn fail_abandoned<C: ConnectionTrait>(
    db: &C,
    stale_before: DateTimeWithTimeZone,
    max_attempts: i16,
    error: String,
    now: DateTimeWithTimeZone,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::update_many()
        .set(ActiveModel {
            status: Set(Status::Failed),
            error_message: Set(Some(error)),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(Column::Status.eq(Status::Processing))
        .filter(Column::Attempts.gte(max_attempts))
        .filter(Column::ClaimedAt.lt(stale_before))
        .exec_with_returning(db)
        .await?)
}

/// Returns a failed analysis to pending with fresh attempts, in one conditional UPDATE.
///
/// `None` when the analysis is no longer failed.
pub async fn restart_failed<C: ConnectionTrait>(
    db: &C,
    id: Id,
    now: DateTimeWithTimeZone,
) -> Result<Option<Model>, Error> {
    Ok(Entity::update_many()
        .set(ActiveModel {
            status: Set(Status::Pending),
            attempts: Set(0),
            claimed_at: Set(None),
            error_message: Set(None),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(Column::Id.eq(id))
        .filter(Column::Status.eq(Status::Failed))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next())
}
