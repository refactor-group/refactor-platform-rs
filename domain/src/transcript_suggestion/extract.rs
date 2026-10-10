//! One extraction run: transcript in, verified suggestions stored.

use chrono::Utc;
use log::*;
use meeting_ai::traits::analysis;
use meeting_ai::types::analysis::Request;
use sea_orm::{ActiveEnum, ActiveValue::Set, DatabaseConnection, TransactionTrait};

use super::{
    create_batch, render, verify_output, ActiveModel, Existing, People, Refs, Session, Status,
    Suggestible, Verified,
};
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::events::{DomainEvent, EventPublisher};
use crate::transcript_analysis::{self, complete_claimed};
use crate::transcription::{local_session_date, read_segments};
use crate::{action, coaching_relationship, coaching_session, cost, transcription, Id};

/// Upper bound on generated tokens for one answer.
const MAX_OUTPUT_TOKENS: u32 = 8192;
/// Model recorded when no model was called.
const NO_MODEL: &str = "none";

/// What one extraction run did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The run completed its analysis and stored this many suggestions.
    Stored(usize),
    /// Another worker retook the claim, so nothing was stored.
    Superseded,
}

/// Why a run failed, kept apart so the sweep can choose between retrying and giving up.
#[derive(Debug)]
pub enum RunError {
    /// The model call failed.
    Provider(meeting_ai::Error),
    /// The model answered but the answer did not verify.
    Verification(Error),
    /// Loading or storing failed.
    Domain(Error),
}

impl From<Error> for RunError {
    fn from(err: Error) -> Self {
        Self::Domain(err)
    }
}

impl From<entity_api::error::Error> for RunError {
    fn from(err: entity_api::error::Error) -> Self {
        Self::Domain(err.into())
    }
}

/// Runs one claimed analysis end to end for target `S`.
///
/// `analysis` is the row as returned by `claim`. Completion is conditional on that claim, so a
/// worker whose claim was retaken stores nothing and returns `Superseded`.
pub async fn run<S: Suggestible>(
    db: &DatabaseConnection,
    provider: &dyn analysis::Provider,
    publisher: &EventPublisher,
    analysis: &transcript_analysis::Model,
) -> Result<Outcome, RunError> {
    let transcription = transcription::find_by_id(db, analysis.transcription_id)
        .await?
        .ok_or_else(not_found)?;
    let session = coaching_session::find_by_id(db, transcription.coaching_session_id).await?;
    let (coach, coachee) =
        coaching_relationship::find_coach_and_coachee(db, session.coaching_relationship_id).await?;

    let segments = read_segments(db, &session, transcription.id).await?;
    if segments.is_empty() {
        debug!("Analysis {} has no segments to read", analysis.id);
        return store::<S>(db, analysis, session.id, NO_MODEL.to_string(), None, vec![]).await;
    }

    let existing = action::find_by_coaching_session(db, session.id)
        .await?
        .into_iter()
        .map(|a| Existing {
            id: a.id,
            text: a.body.unwrap_or_default(),
        })
        .collect();
    let next_session =
        coaching_session::find_next_session(db, session.coaching_relationship_id, session.date)
            .await?
            .map(|next| local_session_date(&next, &coach));

    let input = Session {
        coaching_session_id: session.id,
        people: People {
            coach_id: coach.id,
            coachee_id: coachee.id,
        },
        coach_name: coach.preferred_name().into_owned(),
        coachee_name: coachee.preferred_name().into_owned(),
        coach_timezone: coach.timezone.parse().unwrap_or(chrono_tz::UTC),
        session_date: local_session_date(&session, &coach),
        next_session,
        segments: &segments,
        existing,
    };
    let refs = Refs::new(&input);

    let completion = provider
        .complete(Request {
            instructions: S::instructions().to_string(),
            content: render(&input),
            schema_name: S::schema_name().to_string(),
            output_schema: S::output_schema(),
            max_output_tokens: MAX_OUTPUT_TOKENS,
        })
        .await
        .map_err(RunError::Provider)?;
    let verified =
        verify_output::<S>(&completion.output, &input, &refs).map_err(RunError::Verification)?;
    let tokens = (
        completion.usage.input_tokens,
        completion.usage.output_tokens,
    );

    let outcome = store::<S>(
        db,
        analysis,
        session.id,
        completion.model,
        Some(tokens),
        verified,
    )
    .await?;

    if let Outcome::Stored(stored) = outcome {
        if let Err(e) =
            cost::record_llm_tokens(db, analysis.id, session.id, tokens.0, tokens.1).await
        {
            warn!(
                "Recording the cost of analysis {} failed: {e:?}",
                analysis.id
            );
        }
        if stored > 0 {
            notify::<S>(db, publisher, session.id).await;
        }
    }

    Ok(outcome)
}

/// Completes the claim and stores the verified rows in one transaction, or neither.
async fn store<S: Suggestible>(
    db: &DatabaseConnection,
    analysis: &transcript_analysis::Model,
    coaching_session_id: Id,
    model: String,
    tokens: Option<(u32, u32)>,
    verified: Vec<Verified<S::Proposal>>,
) -> Result<Outcome, RunError> {
    let now = Utc::now().fixed_offset();
    let rows = verified
        .into_iter()
        .map(|v| {
            serde_json::to_value(&v.proposal).map(|proposal| ActiveModel {
                id: Set(Id::new_v4()),
                coaching_session_id: Set(coaching_session_id),
                transcript_analysis_id: Set(analysis.id),
                target: Set(S::TARGET),
                proposal: Set(proposal),
                segment_id: Set(v.segment_id),
                quote: Set(v.quote),
                duplicate_of_id: Set(v.duplicate_of_id),
                status: Set(Status::Pending),
                resolved_by: Set(None),
                resolved_at: Set(None),
                created_entity_id: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| RunError::Domain(internal(format!("Serializing a proposal failed: {e}"))))?;
    let count = rows.len();
    let token_count = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    let completed = complete_claimed(
        &txn,
        analysis.id,
        analysis.attempts,
        model,
        tokens.map(|(input, _)| token_count(input)),
        tokens.map(|(_, output)| token_count(output)),
        now,
    )
    .await?;
    if completed.is_none() {
        txn.rollback()
            .await
            .map_err(entity_api::error::Error::from)?;
        info!("Analysis {} was retaken by another worker", analysis.id);
        return Ok(Outcome::Superseded);
    }
    create_batch(&txn, rows).await?;
    txn.commit().await.map_err(entity_api::error::Error::from)?;

    info!("Analysis {} stored {count} suggestion(s)", analysis.id);
    Ok(Outcome::Stored(count))
}

/// Tells the session's participants new suggestions exist; a failed lookup only skips the event.
pub(super) async fn notify<S: Suggestible>(
    db: &DatabaseConnection,
    publisher: &EventPublisher,
    coaching_session_id: Id,
) {
    match coaching_session::find_participant_ids(db, coaching_session_id).await {
        Ok(notify_user_ids) => {
            publisher
                .publish(DomainEvent::TranscriptSuggestionsChanged {
                    coaching_session_id,
                    target: S::TARGET.to_value(),
                    notify_user_ids,
                })
                .await
        }
        Err(e) => warn!(
            "Suggestions for session {coaching_session_id}: loading participants failed: {e:?}"
        ),
    }
}

fn not_found() -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound)),
    }
}

fn internal(message: String) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(message)),
    }
}

#[cfg(test)]
#[path = "extract_sqlite_tests.rs"]
mod sqlite_tests;
