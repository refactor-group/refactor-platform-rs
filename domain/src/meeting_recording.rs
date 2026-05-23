//! Business logic for meeting recording lifecycle management.

pub use entity::meeting_recording::{MeetingRecordingStatus, Model};
pub use entity_api::meeting_recording::{
    find_by_bot_id, find_latest_by_coaching_session, try_claim_completed, update_status,
    RecordingArtifacts,
};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use entity::Id;
use entity_api::meeting_recording as recording_api;
use log::*;
use meeting_ai::traits::recording_bot;
use meeting_ai::types::recording as recording_types;
use sea_orm::DatabaseConnection;
use std::collections::HashMap;

/// Creates a recording bot and persists the initial `meeting_recordings` row.
pub async fn start(
    db: &DatabaseConnection,
    provider: Option<&dyn recording_bot::Provider>,
    session_id: Id,
    meeting_url: &str,
) -> Result<Model, Error> {
    let provider = provider.ok_or_else(|| {
        warn!("Recording bot provider not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    let mut provider_options = HashMap::new();
    provider_options.insert("coaching_session_id".to_string(), session_id.to_string());

    let config = recording_types::Config {
        meeting_url: meeting_url.to_string(),
        bot_name: "Refactor Coach".to_string(),
        webhook_url: None,
        record_video: false,
        record_audio: true,
        enable_realtime_transcription: false,
        provider_options,
    };

    let bot_info = provider.create_bot(config).await.map_err(Error::from)?;

    info!(
        "Recording bot {} created for session {}",
        bot_info.id, session_id
    );

    let now = chrono::Utc::now();
    let model = Model {
        id: Id::new_v4(),
        coaching_session_id: session_id,
        bot_id: bot_info.id,
        status: MeetingRecordingStatus::Pending,
        video_url: None,
        audio_url: None,
        duration_seconds: None,
        started_at: None,
        ended_at: None,
        error_message: None,
        created_at: now.into(),
        updated_at: now.into(),
    };

    Ok(recording_api::create(db, model).await?)
}

/// Stops the active recording bot for a coaching session.
///
/// Looks up the latest recording for the session, calls `stop_bot` on the provider,
/// and updates the recording status to `Cancelled`.
pub async fn stop(
    db: &DatabaseConnection,
    provider: Option<&dyn recording_bot::Provider>,
    session_id: Id,
) -> Result<Model, Error> {
    let recording = recording_api::find_latest_by_coaching_session(db, session_id)
        .await?
        .ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::NotFound,
            )),
        })?;

    let provider = provider.ok_or_else(|| {
        warn!("Recording bot provider not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    // stop_bot is best-effort: if the bot has already left (meeting ended naturally,
    // provider timeout, etc.) the call returns an error which we treat as success.
    if let Err(e) = provider.stop_bot(&recording.bot_id).await {
        warn!(
            "stop_bot failed for bot {} — bot may have already left: {}",
            recording.bot_id, e
        );
    }

    info!(
        "Removed bot {} from call for session {}",
        recording.bot_id, session_id
    );

    Ok(recording_api::update_status(
        db,
        recording.id,
        MeetingRecordingStatus::Cancelled,
        RecordingArtifacts {
            ended_at: Some(chrono::Utc::now().into()),
            ..Default::default()
        },
    )
    .await?)
}
