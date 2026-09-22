//! Business logic for meeting transcription lifecycle management.

pub use entity::transcription::{Model, TranscriptionStatus};
pub use entity_api::transcription::{
    find_by_coaching_session, find_by_external_id, find_by_id, try_claim_for_processing,
    update_status,
};

use std::collections::HashMap;

use chrono::{NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use entity::meeting_recording::Model as RecordingModel;
use entity::transcript_segment::ActiveModel as SegmentActiveModel;
use entity::Id;
use entity_api::{
    coaching_relationship, transcript_segment as segment_api, transcription as transcription_api,
    user,
};
use log::*;
use meeting_ai::traits::transcription as transcription_trait;
use meeting_ai::types::transcription as transcription_types;
use sea_orm::{ActiveValue::Set, DatabaseConnection};
use serde::Serialize;
use utoipa::ToSchema;

use crate::coaching_sessions;
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::transcript_export::{self, Rendered, Speaker, SpeakerRole};
use crate::users;

/// A transcription plus the speakers found in its transcript.
///
/// `speakers` lists every distinct speaker label in first-appearance order with the
/// relationship participant it resolved to, so a client can tell in advance whether
/// filtering the plain-text download by `coach` or `coachee` will succeed. Empty until
/// segments exist.
#[derive(Clone, Debug, PartialEq, Serialize, ToSchema)]
#[schema(as = domain::transcription::WithSpeakers)]
pub struct WithSpeakers {
    #[serde(flatten)]
    pub transcription: Model,
    pub speakers: Vec<Speaker>,
}

/// Triggers async transcription for the given recording and persists the `transcriptions` row.
///
/// Called after `recording.done` webhook. `recall_recording_id` is the recording UUID
/// used for all subsequent transcript API calls.
pub async fn start(
    db: &DatabaseConnection,
    provider: Option<&dyn transcription_trait::Provider>,
    recording: &RecordingModel,
    recall_recording_id: &str,
) -> Result<Model, Error> {
    let provider = provider.ok_or_else(|| {
        warn!("Transcription provider not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    let mut provider_options = HashMap::new();
    provider_options.insert(
        "recall_recording_id".to_string(),
        recall_recording_id.to_string(),
    );

    let config = transcription_types::Config {
        media_url: String::new(),
        webhook_url: None,
        enable_speaker_labels: true,
        enable_sentiment_analysis: false,
        enable_auto_chapters: false,
        enable_entity_detection: false,
        language_code: None,
        provider_options,
    };

    let transcription = provider
        .create_transcription(config)
        .await
        .map_err(Error::from)?;

    info!(
        "Created async transcript {} for session {}",
        transcription.id, recording.coaching_session_id
    );

    let now = chrono::Utc::now();
    let model = Model {
        id: Id::new_v4(),
        coaching_session_id: recording.coaching_session_id,
        meeting_recording_id: recording.id,
        external_id: transcription.id,
        recall_recording_id: Some(recall_recording_id.to_string()),
        status: TranscriptionStatus::Queued,
        language_code: None,
        speaker_count: None,
        word_count: None,
        duration_seconds: None,
        confidence: None,
        error_message: None,
        created_at: now.into(),
        updated_at: now.into(),
    };

    Ok(transcription_api::create(db, model).await?)
}

/// Fetches the completed transcript from the provider and persists segments.
///
/// Called after `transcript.done` webhook:
/// 1. Retrieves coalesced transcript segments from the provider
/// 2. Updates the `transcriptions` row with word count and Completed status
/// 3. Inserts all utterance segments as `transcript_segments`
pub async fn handle_completion(
    db: &DatabaseConnection,
    provider: Option<&dyn transcription_trait::Provider>,
    external_id: &str,
) -> Result<(), Error> {
    info!(
        "Handling transcript completion for external_id={}",
        external_id
    );

    let transcription = transcription_api::find_by_external_id(db, external_id)
        .await?
        .ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::NotFound,
            )),
        })?;

    let provider = provider.ok_or_else(|| {
        warn!("Transcription provider not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    let result = provider
        .get_transcription(external_id)
        .await
        .map_err(Error::from)?;

    let word_count: usize = result
        .segments
        .iter()
        .map(|s| s.text.split_whitespace().count())
        .sum();

    let segment_count = result.segments.len();

    transcription_api::update_status(
        db,
        transcription.id,
        TranscriptionStatus::Completed,
        Some(i32::try_from(word_count).unwrap_or(i32::MAX)),
        None,
        None,
    )
    .await?;

    if result.segments.is_empty() {
        warn!(
            "No segments in transcript external_id={} — no segments inserted",
            external_id
        );
    } else {
        let now = chrono::Utc::now();
        let segment_models: Vec<SegmentActiveModel> = result
            .segments
            .into_iter()
            .map(|seg| SegmentActiveModel {
                id: Set(Id::new_v4()),
                transcription_id: Set(transcription.id),
                speaker_label: Set(seg.speaker),
                text: Set(seg.text),
                start_ms: Set(i32::try_from(seg.start_ms).unwrap_or(i32::MAX)),
                end_ms: Set(i32::try_from(seg.end_ms).unwrap_or(i32::MAX)),
                confidence: Set(None),
                sentiment: Set(None),
                created_at: Set(now.into()),
            })
            .collect();

        segment_api::create_batch(db, segment_models).await?;
    }

    info!(
        "Transcript completion handled for session_id={}: {} segments inserted",
        transcription.coaching_session_id, segment_count
    );

    Ok(())
}

/// Finds a transcription, requiring it to belong to the given coaching session.
///
/// A transcription under another session reads as missing, so the id cannot be used to
/// confirm transcriptions outside the session the caller is authorized for.
pub async fn find_for_session(
    db: &DatabaseConnection,
    transcription_id: Id,
    coaching_session_id: Id,
) -> Result<Model, Error> {
    transcription_api::find_by_id(db, transcription_id)
        .await?
        .filter(|transcription| transcription.coaching_session_id == coaching_session_id)
        .ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::TranscriptionNotFound,
            )),
        })
}

/// The coach and coachee of the session's coaching relationship, in that order.
async fn load_participants(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
) -> Result<(users::Model, users::Model), Error> {
    let relationship =
        coaching_relationship::find_by_id(db, session.coaching_relationship_id).await?;
    let coach = user::find_by_id_without_roles(db, relationship.coach_id).await?;
    let coachee = user::find_by_id_without_roles(db, relationship.coachee_id).await?;

    Ok((coach, coachee))
}

/// Renders the session's transcript as plain text, optionally limited to given roles.
///
/// Refuses a transcription that has not completed, since its segments are still partial.
/// An empty `filter` keeps every speaker.
pub async fn export_plain_text(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    transcription_id: Id,
    filter: &[SpeakerRole],
) -> Result<Rendered, Error> {
    let transcription = find_for_session(db, transcription_id, session.id).await?;

    if transcription.status != TranscriptionStatus::Completed {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::TranscriptionNotCompleted,
            )),
        });
    }

    let segments =
        segment_api::find_by_transcription_and_session(db, transcription_id, session.id).await?;
    let (coach, coachee) = load_participants(db, session).await?;
    let speakers = transcript_export::resolve_speakers(&coach, &coachee, &segments);

    transcript_export::render_plain_text(
        local_session_date(session, &coach),
        &speakers,
        &segments,
        filter,
    )
}

/// The session's calendar date where the coach is; the schedule is anchored on them.
///
/// `session.date` is a UTC instant, so an evening session in the Americas would
/// otherwise be dated tomorrow.
fn local_session_date(session: &coaching_sessions::Model, coach: &users::Model) -> NaiveDate {
    let tz: Tz = coach.timezone.parse().unwrap_or(chrono_tz::UTC);
    Utc.from_utc_datetime(&session.date)
        .with_timezone(&tz)
        .date_naive()
}

/// Reads the session's transcription along with its resolved speakers.
///
/// Readable at any status: the speaker list simply stays empty until segments land.
pub async fn read_with_speakers(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    transcription_id: Id,
) -> Result<WithSpeakers, Error> {
    let transcription = find_for_session(db, transcription_id, session.id).await?;
    let segments =
        segment_api::find_by_transcription_and_session(db, transcription_id, session.id).await?;
    let (coach, coachee) = load_participants(db, session).await?;

    Ok(WithSpeakers {
        transcription,
        speakers: transcript_export::resolve_speakers(&coach, &coachee, &segments),
    })
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "transcription_tests.rs"]
mod tests;
