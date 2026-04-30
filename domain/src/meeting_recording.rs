//! Business logic for meeting recording lifecycle management.

pub use entity::meeting_recording::{MeetingRecordingStatus, Model};
pub use entity_api::meeting_recording::{
    find_by_bot_id, find_latest_by_coaching_session, update_status,
};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::gateway::recall_ai;
use entity::Id;
use entity_api::meeting_recording as recording_api;
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;

fn recall_ai_provider(config: &Config) -> Result<recall_ai::Provider, Error> {
    let api_key = config.recall_ai_api_key().ok_or_else(|| {
        warn!("RECALL_AI_API_KEY not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;
    let webhook_base = config.webhook_base_url().ok_or_else(|| {
        warn!("WEBHOOK_BASE_URL not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;
    let webhook_url = format!("{}/webhooks/recall_ai", webhook_base);
    recall_ai::Provider::new(&api_key, config.recall_ai_region(), &webhook_url)
}

/// Creates a Recall.ai recording bot and persists the initial `meeting_recordings` row.
pub async fn start(
    db: &DatabaseConnection,
    config: &Config,
    session_id: Id,
    meeting_url: &str,
) -> Result<Model, Error> {
    let provider = recall_ai_provider(config)?;

    let bot = provider
        .create_bot(&session_id.to_string(), meeting_url, "Refactor Coach")
        .await?;

    info!(
        "Recall.ai bot created for session {}: {}",
        session_id, bot.id
    );

    let now = chrono::Utc::now();
    let model = Model {
        id: Id::new_v4(),
        coaching_session_id: session_id,
        bot_id: bot.id,
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
/// Looks up the latest recording for the session, calls `DELETE /bot/{id}` on Recall.ai,
/// and updates the recording status to `failed`.
pub async fn stop(
    db: &DatabaseConnection,
    config: &Config,
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

    let provider = recall_ai_provider(config)?;
    provider.delete_bot(&recording.bot_id).await?;

    info!(
        "Deleted Recall.ai bot {} for session {}",
        recording.bot_id, session_id
    );

    Ok(recording_api::update_status(
        db,
        recording.id,
        MeetingRecordingStatus::Processing,
        None,
        None,
        None,
        None,
        Some(chrono::Utc::now().into()),
        None,
    )
    .await?)
}
