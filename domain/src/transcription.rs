//! Business logic for meeting transcription lifecycle management.

pub use entity::transcription::{Model, TranscriptionStatus};
pub use entity_api::transcription::{
    find_by_coaching_session, find_by_external_id, find_by_id, try_claim_for_processing,
    update_status,
};

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use entity::meeting_recording::Model as RecordingModel;
use entity::transcript_participant::ActiveModel as ParticipantActiveModel;
use entity::transcript_segment::{ActiveModel as SegmentActiveModel, Model as Segment};
use entity::Id;
use entity_api::error::Error as EntityApiError;
use entity_api::{
    coaching_relationship, coaching_session as coaching_session_api,
    meeting_recording as recording_api, transcript_analysis as analysis_api,
    transcript_participant as participant_api, transcript_segment as segment_api,
    transcription as transcription_api,
};
use log::*;
use meeting_ai::traits::{recording_bot, transcription as transcription_trait};
use meeting_ai::types::transcription as transcription_types;
use sea_orm::{ActiveValue::Set, DatabaseConnection, DatabaseTransaction, TransactionTrait};
use serde::Serialize;
use service::config::Config;
use tokio::time::timeout;
use utoipa::ToSchema;

use crate::coaching_relationship::schedule_owner;
use crate::coaching_relationship_kind;
use crate::coaching_relationships;
use crate::coaching_sessions;
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::gateway::google_meet;
use crate::meeting_provider::Provider as MeetingProvider;
use crate::oauth_connection::{external_account_id, get_valid_access_token};
use crate::transcript_attribution::{anchor_and_other, attribute, Attribution, Evidence};
use crate::transcript_export::{self, Labeled, LabeledSegment, Rendered, Speaker};
use crate::transcript_suggestion_target::Target;
use crate::{user, users};

/// A transcription plus the speakers found in its transcript.
///
/// `speakers` lists every distinct speaker in first-speaking order with the role stored
/// for them at completion, labeled exactly as the transcript's segments are, so a client
/// can tell in advance whether filtering the plain-text download by `coach` or `coachee`
/// will succeed. Empty until segments exist.
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

/// Fetches the completed transcript, attributes its speakers, and persists them with its segments.
///
/// Called after the `transcript.done` webhook. Speaker attribution never fails the transcript: any
/// evidence that cannot be gathered leaves the affected speakers unattributed. The transcription is
/// marked `Completed` in the same transaction that stores its participants and segments.
pub async fn handle_completion(
    db: &DatabaseConnection,
    provider: Option<&dyn transcription_trait::Provider>,
    recording_bot: Option<&dyn recording_bot::Provider>,
    config: &Config,
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

    let attributions = attribute_speakers(
        db,
        recording_bot,
        config,
        &transcription,
        &result.participants,
    )
    .await;
    // Only transcripts completed while extraction is configured are ever analyzed.
    let analyses: &[Target] = if config.assembly_ai_api_key().is_some() {
        &[Target::Action]
    } else {
        &[]
    };
    persist_completion_with(db, &transcription, &result, &attributions, analyses).await?;

    info!(
        "Transcript completion handled for transcription {}: {} segments, {} speakers, {} attributed",
        transcription.id,
        result.segments.len(),
        attributions.len(),
        attributions.iter().filter(|a| a.user_id.is_some()).count()
    );

    Ok(())
}

/// Stores the transcript's participants with their attribution, its segments linked to them, and
/// its `Completed` status with the word count, in one transaction.
#[cfg(test)]
pub(crate) async fn persist_completion(
    db: &DatabaseConnection,
    transcription: &Model,
    result: &transcription_types::Transcription,
    attributions: &[Attribution],
) -> Result<(), Error> {
    persist_completion_with(db, transcription, result, attributions, &[]).await
}

/// Like `persist_completion`, also recording a pending analysis per target in the same
/// transaction, unless the session's relationship suggests no actions.
///
/// Writing the pending rows with the segments means a completed transcript is never left without
/// the analyses it was due, and an analysis never exists for a transcript that did not complete.
pub(crate) async fn persist_completion_with(
    db: &DatabaseConnection,
    transcription: &Model,
    result: &transcription_types::Transcription,
    attributions: &[Attribution],
    analyses: &[Target],
) -> Result<(), Error> {
    let analyses = owed_analyses(db, transcription, analyses).await?;
    let txn = db.begin().await.map_err(EntityApiError::from)?;

    if !(result.participants.is_empty() && result.segments.is_empty()) {
        store_speakers_and_segments(&txn, transcription, result, attributions).await?;
    }

    let word_count: usize = result
        .segments
        .iter()
        .map(|s| s.text.split_whitespace().count())
        .sum();
    transcription_api::update_status(
        &txn,
        transcription.id,
        TranscriptionStatus::Completed,
        Some(i32::try_from(word_count).unwrap_or(i32::MAX)),
        None,
        None,
    )
    .await?;

    for target in analyses {
        analysis_api::create_pending(&txn, transcription.id, *target).await?;
    }

    txn.commit().await.map_err(EntityApiError::from)?;

    Ok(())
}

/// The `analyses` a transcription is owed: none when its session's relationship suggests no
/// actions.
async fn owed_analyses<'a>(
    db: &DatabaseConnection,
    transcription: &Model,
    analyses: &'a [Target],
) -> Result<&'a [Target], Error> {
    if analyses.is_empty() {
        return Ok(analyses);
    }
    let (_, relationship) = coaching_session_api::find_by_id_with_coaching_relationship(
        db,
        transcription.coaching_session_id,
    )
    .await?;
    Ok(match relationship.kind.rules().suggests_actions() {
        true => analyses,
        false => &[],
    })
}

async fn store_speakers_and_segments(
    txn: &DatabaseTransaction,
    transcription: &Model,
    result: &transcription_types::Transcription,
    attributions: &[Attribution],
) -> Result<(), Error> {
    let now = Utc::now();
    let participant_rows = participant_rows(transcription.id, result, attributions);

    let participants = participant_api::create_batch(txn, participant_rows).await?;
    let row_of: HashMap<&str, Id> = participants
        .iter()
        .map(|p| (p.provider_participant_id.as_str(), p.id))
        .collect();
    let segment_rows: Vec<SegmentActiveModel> = result
        .segments
        .iter()
        .map(|seg| SegmentActiveModel {
            id: Set(Id::new_v4()),
            participant_id: Set(seg
                .participant_id
                .as_deref()
                .and_then(|provider_id| row_of.get(provider_id).copied())),
            transcription_id: Set(transcription.id),
            speaker_label: Set(seg.speaker.clone()),
            text: Set(seg.text.clone()),
            start_ms: Set(i32::try_from(seg.start_ms).unwrap_or(i32::MAX)),
            end_ms: Set(i32::try_from(seg.end_ms).unwrap_or(i32::MAX)),
            confidence: Set(None),
            sentiment: Set(None),
            created_at: Set(now.into()),
        })
        .collect();
    segment_api::create_batch(txn, segment_rows).await?;

    Ok(())
}

/// One participant row per transcript speaker, carrying its attribution by provider id.
fn participant_rows(
    transcription_id: Id,
    result: &transcription_types::Transcription,
    attributions: &[Attribution],
) -> Vec<ParticipantActiveModel> {
    let now = Utc::now();
    let attribution_of: HashMap<&str, &Attribution> = attributions
        .iter()
        .map(|a| (a.provider_id.as_str(), a))
        .collect();
    result
        .participants
        .iter()
        .map(|p| {
            let attribution = attribution_of.get(p.provider_id.as_str());
            ParticipantActiveModel {
                id: Set(Id::new_v4()),
                transcription_id: Set(transcription_id),
                provider_participant_id: Set(p.provider_id.clone()),
                display_name: Set(p.display_name.clone()),
                is_host: Set(p.is_host),
                platform: Set(p.platform.clone()),
                platform_account_id: Set(p.account_id.clone()),
                extra_data: Set(p.extra_data.clone()),
                user_id: Set(attribution.and_then(|a| a.user_id)),
                match_source: Set(attribution.and_then(|a| a.source)),
                created_at: Set(now.into()),
            }
        })
        .collect()
}

/// Upper bound on evidence gathering so a hung provider never stalls a completion.
const EVIDENCE_TIMEOUT: Duration = Duration::from_secs(60);

/// Evidence gathered for attribution, owned so `Evidence` can borrow from it.
#[derive(Default)]
struct Gathered {
    recorded_session_space: bool,
    anchor_account_id: Option<String>,
    attendees: Option<Vec<google_meet::Participant>>,
}

impl Gathered {
    fn evidence(&self) -> Evidence<'_> {
        Evidence {
            recorded_session_space: self.recorded_session_space,
            anchor_account_id: self.anchor_account_id.as_deref(),
            attendees: self.attendees.as_deref(),
        }
    }
}

/// Attributes the speakers to the session's two participants; nobody when they are unknown or
/// the relationship's kind names no anchor.
async fn attribute_speakers(
    db: &DatabaseConnection,
    recording_bot: Option<&dyn recording_bot::Provider>,
    config: &Config,
    transcription: &Model,
    speakers: &[transcription_types::Participant],
) -> Vec<Attribution> {
    let Ok((session, relationship)) = session_relationship(db, transcription)
        .await
        .inspect_err(|e| {
            warn!(
                "Attribution for transcription {}: loading the session's relationship failed: {e:?}",
                transcription.id
            )
        })
    else {
        return unattributed(speakers);
    };
    let Some((anchor_id, other_id)) =
        anchor_and_other(&relationship, session.meeting_url_created_by_user_id)
    else {
        debug!(
            "Attribution for transcription {}: no anchor for this relationship's kind",
            transcription.id
        );
        return unattributed(speakers);
    };

    let gathered = timeout(
        EVIDENCE_TIMEOUT,
        gather_evidence(
            db,
            recording_bot,
            config,
            transcription,
            &session,
            anchor_id,
        ),
    )
    .await
    .unwrap_or_else(|_| {
        warn!(
            "Attribution for transcription {}: gathering evidence timed out",
            transcription.id
        );
        Gathered::default()
    });
    attribute(
        speakers,
        anchor_id,
        other_id,
        relationship.kind,
        &gathered.evidence(),
    )
}

/// The transcription's session with its relationship.
async fn session_relationship(
    db: &DatabaseConnection,
    transcription: &Model,
) -> Result<(coaching_sessions::Model, coaching_relationships::Model), Error> {
    let session = coaching_session_api::find_by_id(db, transcription.coaching_session_id).await?;
    let relationship =
        coaching_relationship::find_by_id(db, session.coaching_relationship_id).await?;
    Ok((session, relationship))
}

fn unattributed(speakers: &[transcription_types::Participant]) -> Vec<Attribution> {
    speakers
        .iter()
        .map(|s| Attribution {
            provider_id: s.provider_id.clone(),
            user_id: None,
            source: None,
        })
        .collect()
}

/// Gathers what is known about the anchor; every failure degrades the evidence, none is fatal.
/// Attendees are matched against the anchor's account, but read with the token of whoever made
/// the meeting link, or the anchor's when no maker was recorded.
async fn gather_evidence(
    db: &DatabaseConnection,
    recording_bot: Option<&dyn recording_bot::Provider>,
    config: &Config,
    transcription: &Model,
    session: &coaching_sessions::Model,
    anchor_id: Id,
) -> Gathered {
    let link_maker_id = session.meeting_url_created_by_user_id.unwrap_or(anchor_id);
    let Some(code) = session_meet_code(session) else {
        debug!(
            "Attribution for transcription {}: not a Google Meet session",
            transcription.id
        );
        return Gathered::default();
    };
    let Some(recording) = recording_of_space(db, recording_bot, transcription, &code).await else {
        return Gathered::default();
    };
    let anchor_account_id = external_account_id(db, config, anchor_id, MeetingProvider::Google)
        .await
        .inspect_err(|_| {
            warn!(
                "Attribution for transcription {}: looking up the anchor's Google account failed",
                transcription.id
            )
        })
        .ok()
        .flatten();

    let attendees = match (&anchor_account_id, recording.started_at, recording.ended_at) {
        (Some(_), Some(started_at), Some(ended_at)) => {
            conference_attendees(
                db,
                config,
                transcription.id,
                link_maker_id,
                &code,
                (started_at.with_timezone(&Utc), ended_at.with_timezone(&Utc)),
            )
            .await
        }
        _ => None,
    };

    Gathered {
        recorded_session_space: true,
        anchor_account_id,
        attendees,
    }
}

/// The session's Meet code, when it is a Google Meet session.
fn session_meet_code(session: &coaching_sessions::Model) -> Option<String> {
    session
        .provider
        .filter(|provider| *provider == MeetingProvider::Google)
        .and(session.meeting_url.as_deref())
        .and_then(google_meet::meeting_code_from_url)
}

/// The transcription's recording, when its bot recorded the meeting with the given code.
async fn recording_of_space(
    db: &DatabaseConnection,
    recording_bot: Option<&dyn recording_bot::Provider>,
    transcription: &Model,
    code: &str,
) -> Option<RecordingModel> {
    let Some(recording_bot) = recording_bot else {
        warn!(
            "Attribution for transcription {}: recording bot provider not configured",
            transcription.id
        );
        return None;
    };

    let recording = recording_api::find_by_id(db, transcription.meeting_recording_id)
        .await
        .inspect_err(|e| {
            warn!(
                "Attribution for transcription {}: loading the recording failed: {e:?}",
                transcription.id
            )
        })
        .ok()?
        .or_else(|| {
            warn!(
                "Attribution for transcription {}: the recording does not exist",
                transcription.id
            );
            None
        })?;

    let Some(bot_id) = recording.bot_id.as_deref() else {
        warn!(
            "Attribution for transcription {}: the recording has no bot",
            transcription.id
        );
        return None;
    };

    let info = recording_bot
        .get_bot_status(bot_id)
        .await
        .inspect_err(|e| {
            warn!(
                "Attribution for transcription {}: reading the recording bot failed: {e}",
                transcription.id
            )
        })
        .ok()?;

    if info.meeting_id.as_deref() == Some(code) {
        Some(recording)
    } else {
        warn!(
            "Attribution for transcription {}: the bot recorded a different meeting than the session's",
            transcription.id
        );
        None
    }
}

/// Attendees of the recorded conference, read with `account_holder_id`'s Google token.
async fn conference_attendees(
    db: &DatabaseConnection,
    config: &Config,
    transcription_id: Id,
    account_holder_id: Id,
    code: &str,
    (started_at, ended_at): (DateTime<Utc>, DateTime<Utc>),
) -> Option<Vec<google_meet::Participant>> {
    let warn_step = |step: &str| {
        warn!("Attribution for transcription {transcription_id}: {step} failed");
    };

    let token = get_valid_access_token(db, config, account_holder_id, MeetingProvider::Google)
        .await
        .inspect_err(|_| warn_step("getting the account holder's Google token"))
        .ok()?;
    let client = google_meet::Client::new(&token, config.google_meet_api_url())
        .inspect_err(|_| warn_step("building the Google Meet client"))
        .ok()?;

    match client
        .conference_participants(code, started_at, ended_at)
        .await
    {
        Ok(Some(attendees)) => Some(attendees),
        Ok(None) => {
            warn_step("finding the one conference overlapping the recording");
            None
        }
        Err(_) => {
            warn_step("listing the conference attendees");
            None
        }
    }
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

/// Renders the session's transcript as plain text, optionally limited to given roles.
///
/// Refuses a transcription that has not completed, since its segments are still partial.
/// An empty `filter` keeps every speaker.
pub async fn export_plain_text(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    transcription_id: Id,
    filter: &[coaching_relationship_kind::Role],
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
    let (labeled, relationship, owner) = label(db, session, transcription_id, &segments).await?;

    transcript_export::render_plain_text(
        relationship.kind,
        local_session_date(session, &owner),
        &labeled,
        filter,
    )
}

/// The session's calendar date where `person` is.
///
/// `session.date` is a UTC instant, so an evening session in the Americas would
/// otherwise be dated tomorrow.
pub(crate) fn local_session_date(
    session: &coaching_sessions::Model,
    person: &users::Model,
) -> NaiveDate {
    Utc.from_utc_datetime(&session.date)
        .with_timezone(&user::timezone(person))
        .date_naive()
}

/// Reads the session's transcription along with its labeled speakers.
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
    let (labeled, ..) = label(db, session, transcription_id, &segments).await?;

    Ok(WithSpeakers {
        transcription,
        speakers: labeled.speakers,
    })
}

/// The session's most recent transcription with its labeled speakers, if it has one.
///
/// `speakers` is empty until segments exist.
pub async fn read_latest_with_speakers(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
) -> Result<Option<WithSpeakers>, Error> {
    let Some(transcription) = find_by_coaching_session(db, session.id).await? else {
        return Ok(None);
    };
    let segments =
        segment_api::find_by_transcription_and_session(db, transcription.id, session.id).await?;
    let speakers = if segments.is_empty() {
        vec![]
    } else {
        label(db, session, transcription.id, &segments)
            .await?
            .0
            .speakers
    };

    Ok(Some(WithSpeakers {
        transcription,
        speakers,
    }))
}

/// The transcription's segments as readers see them, in speaking order.
///
/// Empty when the transcription does not exist, belongs to another session, or has no segments,
/// matching the previous behavior of the segments endpoint.
pub async fn read_segments(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    transcription_id: Id,
) -> Result<Vec<LabeledSegment>, Error> {
    let segments =
        segment_api::find_by_transcription_and_session(db, transcription_id, session.id).await?;
    if segments.is_empty() {
        return Ok(vec![]);
    }

    let (labeled, ..) = label(db, session, transcription_id, &segments).await?;
    Ok(labeled.segments)
}

/// Labels the segments from the transcription's stored participants; also returns the
/// relationship and the session's schedule owner.
async fn label(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
    transcription_id: Id,
    segments: &[Segment],
) -> Result<(Labeled, coaching_relationships::Model, users::Model), Error> {
    let participants = participant_api::find_by_transcription(db, transcription_id).await?;
    let (relationship, coach, coachee) =
        coaching_relationship::find_with_coach_and_coachee(db, session.coaching_relationship_id)
            .await?;
    let labeled = transcript_export::label_transcript(
        &participants,
        segments,
        &relationship,
        &coach,
        &coachee,
    );
    let owner = schedule_owner(
        &relationship,
        &coach,
        &coachee,
        session.scheduled_by_user_id,
    )
    .clone();

    Ok((labeled, relationship, owner))
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "transcription_tests.rs"]
mod tests;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "transcription_completion_tests.rs"]
mod completion_tests;

#[cfg(test)]
#[path = "transcription_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(test)]
#[path = "transcription_completion_sqlite_tests.rs"]
mod completion_sqlite_tests;

#[cfg(test)]
#[path = "transcription_analysis_hook_sqlite_tests.rs"]
mod analysis_hook_sqlite_tests;
