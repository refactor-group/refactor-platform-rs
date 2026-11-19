//! Business logic for meeting recording lifecycle management.

pub use entity::meeting_recording::{MeetingRecordingStatus, Model};
pub use entity_api::meeting_recording::{
    find_by_bot_id, find_by_id, find_latest_by_coaching_session, try_claim_completed,
    update_status, RecordingArtifacts,
};

use crate::error::{reason, DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::organization::ensure_not_archived;
use chrono::TimeDelta;
use entity::{coaching_relationships, Id};
use entity_api::meeting_recording as recording_api;
use entity_api::organization;
use events::{DomainEvent, EventPublisher};
use log::*;
use meeting_ai::traits::recording_bot;
use meeting_ai::types::recording as recording_types;
use sea_orm::DatabaseConnection;
use std::collections::HashMap;
use std::sync::Arc;

/// How long a claim may wait for its bot id before it is taken as abandoned.
const ABANDONED_CLAIM_AFTER: TimeDelta = TimeDelta::minutes(2);

/// Claims the session for one recording, then sends the recording bot.
///
/// The claim is committed before the bot is sent, so of two concurrent starts exactly one
/// sends a bot; the other gets a 409 with reason `recording_already_active`. A failed send
/// marks the claim `Failed`, so the session can be retried, and tells the participants. A
/// session in an archived organization is refused with `OrganizationArchived` before anything
/// is written or sent. `relationship` is the session's coaching relationship.
///
/// Everything after the claim runs in its own task, so a dropped request cannot leave a sent
/// bot without its id stored.
pub async fn start(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    provider: Option<Arc<dyn recording_bot::Provider>>,
    session_id: Id,
    relationship: &coaching_relationships::Model,
    meeting_url: &str,
) -> Result<Model, Error> {
    let provider = provider.ok_or_else(|| {
        warn!("Recording bot provider not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    ensure_not_archived(&organization::find_by_id(db, relationship.organization_id).await?)?;

    let abandoned_before = chrono::Utc::now() - ABANDONED_CLAIM_AFTER;
    recording_api::fail_abandoned_claims(db, session_id, abandoned_before.into()).await?;

    let claim = recording_api::claim(db, session_id).await?.ok_or_else(|| {
        Error::conflict_because(
            reason::RECORDING_ALREADY_ACTIVE,
            "This session already has an active recording.",
            serde_json::json!({}),
        )
    })?;

    let (db, event_publisher, meeting_url) =
        (db.clone(), event_publisher.clone(), meeting_url.to_string());
    tokio::spawn(async move {
        dispatch(&db, &event_publisher, provider.as_ref(), claim, meeting_url).await
    })
    .await
    .map_err(|e| {
        warn!("Recording dispatch task for session {session_id} failed: {e}");
        Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "recording dispatch task failed".to_string(),
            )),
        }
    })?
}

/// Sends the bot for `claim` and stores its id, or recalls it when the claim ended meanwhile.
async fn dispatch(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    provider: &dyn recording_bot::Provider,
    claim: Model,
    meeting_url: String,
) -> Result<Model, Error> {
    let session_id = claim.coaching_session_id;
    let config = recording_types::Config {
        meeting_url,
        bot_name: "Refactor Coach".to_string(),
        webhook_url: None,
        record_video: false,
        record_audio: true,
        enable_realtime_transcription: false,
        provider_options: HashMap::from([(
            "coaching_session_id".to_string(),
            session_id.to_string(),
        )]),
    };

    let bot_info = match provider.create_bot(config).await {
        Ok(bot_info) => bot_info,
        Err(e) => {
            release_claim(db, claim.id, e.to_string()).await;
            publish_updated(db, event_publisher, session_id).await;
            return Err(Error::from(e));
        }
    };

    info!(
        "Recording bot {} created for session {}",
        bot_info.id, session_id
    );

    match recording_api::set_bot_id(db, claim.id, &bot_info.id).await {
        Ok(Some(recording)) => {
            publish_updated(db, event_publisher, session_id).await;
            Ok(recording)
        }
        Ok(None) => {
            // A stop or sweep ended the claim during dispatch; recall the bot, then keep its id.
            stop_bot(provider, &bot_info.id).await;
            Ok(recording_api::record_bot_id(db, claim.id, &bot_info.id)
                .await
                .unwrap_or_else(|e| {
                    warn!(
                        "Could not record recalled bot {} on recording {}: {e:?}",
                        bot_info.id, claim.id
                    );
                    Model {
                        bot_id: Some(bot_info.id.clone()),
                        ..claim
                    }
                }))
        }
        Err(e) => {
            stop_bot(provider, &bot_info.id).await;
            release_claim(db, claim.id, e.to_string()).await;
            publish_updated(db, event_publisher, session_id).await;
            Err(e.into())
        }
    }
}

/// Marks a claim whose bot could not be sent `Failed`, freeing the session for a retry. A
/// claim that already ended, say by a stop, keeps its status.
async fn release_claim(db: &DatabaseConnection, id: Id, error_message: String) {
    if let Err(e) = recording_api::fail_pending_claim(db, id, error_message).await {
        warn!("Could not release recording claim {id} after a failed dispatch: {e:?}");
    }
}

/// Asks the provider to remove a bot. Best-effort: a bot that already left answers an error.
async fn stop_bot(provider: &dyn recording_bot::Provider, bot_id: &str) {
    if let Err(e) = provider.stop_bot(bot_id).await {
        warn!("stop_bot failed for bot {bot_id}; it may have already left: {e}");
    }
}

/// Tells the session's participants its recording changed.
pub(crate) async fn publish_updated(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    coaching_session_id: Id,
) {
    match crate::coaching_session::find_participant_ids(db, coaching_session_id).await {
        Ok(user_ids) => {
            event_publisher
                .publish(DomainEvent::MeetingRecordingUpdated {
                    coaching_session_id,
                    notify_user_ids: user_ids,
                })
                .await;
        }
        Err(e) => warn!(
            "meeting recording: could not resolve participants for session {}: {:?}",
            coaching_session_id, e
        ),
    }
}

/// Stops the active recording bot for a coaching session.
///
/// Marks the session's latest recording `Cancelled`, then calls `stop_bot` on the provider
/// when the cancelled recording has a bot, and tells the session's participants. A recording
/// that already ended is returned unchanged, with no provider call and no event.
pub async fn stop(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
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

    if recording_api::TERMINAL_RECORDING_STATUSES.contains(&recording.status) {
        return Ok(recording);
    }

    let provider = provider.ok_or_else(|| {
        warn!("Recording bot provider not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    let Some(stopped) = recording_api::cancel(db, recording.id).await? else {
        // It ended after the read above.
        return Ok(recording_api::find_by_id(db, recording.id)
            .await?
            .unwrap_or(recording));
    };

    // A claim still being dispatched has no bot yet; `start` recalls it once the id arrives.
    if let Some(bot_id) = &stopped.bot_id {
        stop_bot(provider, bot_id).await;
        info!("Removed bot {bot_id} from call for session {session_id}");
    }

    publish_updated(db, event_publisher, session_id).await;
    Ok(stopped)
}

#[cfg(test)]
#[path = "meeting_recording_sqlite_tests.rs"]
mod sqlite_tests;
