//! Controller for handling webhooks from external services.
//!
//! Handles webhooks from Recall.ai for meeting recording status updates.

use crate::{AppState, Error};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;

use domain::action as ActionApi;
use domain::agreement as AgreementApi;
use domain::ai_suggested_item as AiSuggestedItemApi;
use domain::ai_suggestion::AiSuggestionType;
use domain::coaching_relationship as CoachingRelationshipApi;
use domain::coaching_session as CoachingSessionApi;
use domain::gateway::assembly_ai::{
    create_standard_transcript_request, extract_action_items, AssemblyAiClient, TranscriptStatus,
};
use domain::meeting_recording as MeetingRecordingApi;
use domain::meeting_recording_status::MeetingRecordingStatus;
use domain::meeting_recordings::Model as MeetingRecordingModel;
use domain::transcript_segment::{self, SegmentInput};
use domain::transcription as TranscriptionApi;
use domain::transcription_status::TranscriptionStatus;
use domain::transcriptions::Model as TranscriptionModel;
use domain::user as UserApi;
use domain::user_integration as UserIntegrationApi;
use log::*;
use serde::{Deserialize, Serialize};

/// Recall.ai webhook event payload
#[derive(Debug, Deserialize)]
pub struct RecallWebhookPayload {
    /// The type of event
    pub event: String,
    /// The bot ID this event is for
    pub data: RecallWebhookData,
}

/// Data section of Recall.ai webhook
#[derive(Debug, Deserialize)]
pub struct RecallWebhookData {
    /// Bot ID
    pub bot_id: String,
    /// Status code (for status change events)
    pub status: Option<RecallBotStatus>,
    /// Video URL (available when recording is complete)
    pub video_url: Option<String>,
    /// Recording duration in seconds
    pub duration: Option<i32>,
    /// Error details if the bot failed
    pub error: Option<RecallError>,
}

/// Recall.ai bot status
#[derive(Debug, Deserialize)]
pub struct RecallBotStatus {
    pub code: String,
}

/// Recall.ai error details
#[derive(Debug, Deserialize)]
pub struct RecallError {
    pub code: Option<String>,
    pub message: Option<String>,
}

/// Response for webhook acknowledgment
#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    pub status: String,
}

/// Maps Recall.ai status codes to our internal status
fn map_recall_status(code: &str) -> MeetingRecordingStatus {
    match code {
        "ready" | "joining_call" => MeetingRecordingStatus::Joining,
        "in_call_not_recording" | "in_waiting_room" => MeetingRecordingStatus::Joining,
        "in_call_recording" => MeetingRecordingStatus::Recording,
        "call_ended" | "done" => MeetingRecordingStatus::Processing,
        "analysis_done" => MeetingRecordingStatus::Completed,
        "fatal" | "error" => MeetingRecordingStatus::Failed,
        _ => MeetingRecordingStatus::Pending,
    }
}

/// POST /webhooks/recall
///
/// Handles webhook callbacks from Recall.ai for bot status updates.
/// This endpoint does not require authentication but validates via webhook secret.
pub async fn recall_webhook(
    State(app_state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RecallWebhookPayload>,
) -> Result<impl IntoResponse, Error> {
    debug!("Received Recall.ai webhook: {:?}", payload.event);

    let config = &app_state.config;
    let db = app_state.db_conn_ref();

    // Validate webhook secret if configured
    if let Some(expected_secret) = config.webhook_secret() {
        let provided_secret = headers
            .get("x-webhook-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if provided_secret != expected_secret {
            warn!("Invalid webhook secret received");
            return Ok((
                StatusCode::UNAUTHORIZED,
                Json(WebhookResponse {
                    status: "unauthorized".to_string(),
                }),
            ));
        }
    }

    let bot_id = &payload.data.bot_id;

    // Find the recording by bot ID
    let recording: Option<MeetingRecordingModel> =
        MeetingRecordingApi::find_by_recall_bot_id(db, bot_id).await?;

    let recording = match recording {
        Some(r) => r,
        None => {
            warn!("Received webhook for unknown bot ID: {}", bot_id);
            return Ok((
                StatusCode::OK,
                Json(WebhookResponse {
                    status: "ignored".to_string(),
                }),
            ));
        }
    };

    // Handle different event types
    match payload.event.as_str() {
        "bot.status_change" => {
            if let Some(status) = &payload.data.status {
                let new_status = map_recall_status(&status.code);
                info!(
                    "Bot {} status changed to {} (internal: {:?})",
                    bot_id, status.code, new_status
                );

                // Check for errors
                let error_message = if new_status == MeetingRecordingStatus::Failed {
                    payload.data.error.as_ref().map(|e| {
                        format!(
                            "{}: {}",
                            e.code.as_deref().unwrap_or("unknown"),
                            e.message.as_deref().unwrap_or("Unknown error")
                        )
                    })
                } else {
                    None
                };

                let _: MeetingRecordingModel =
                    MeetingRecordingApi::update_status(db, recording.id, new_status, error_message)
                        .await?;
            }
        }
        "bot.done" | "recording.done" => {
            info!("Bot {} recording completed", bot_id);

            // Update with video URL and duration if available
            let mut updated_recording = recording.clone();
            updated_recording.status = MeetingRecordingStatus::Processing;
            updated_recording.recording_url = payload.data.video_url.clone();
            updated_recording.duration_seconds = payload.data.duration;
            updated_recording.ended_at = Some(chrono::Utc::now().into());

            let updated: MeetingRecordingModel =
                MeetingRecordingApi::update(db, recording.id, updated_recording).await?;

            // If we have a video URL, trigger AssemblyAI transcription
            if let Some(video_url) = payload.data.video_url.clone() {
                // Look up the coach to get their AssemblyAI API key
                match trigger_assemblyai_transcription(
                    db,
                    config,
                    updated.id,
                    updated.coaching_session_id,
                    &video_url,
                )
                .await
                {
                    Ok(_) => {
                        info!(
                            "AssemblyAI transcription triggered for recording {}",
                            updated.id
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Failed to trigger AssemblyAI transcription for recording {}: {:?}",
                            updated.id, e
                        );
                        // Don't fail the webhook - recording is still saved
                    }
                }
            }
        }
        "bot.error" | "recording.error" => {
            warn!("Bot {} encountered an error", bot_id);

            let error_message = payload.data.error.as_ref().map(|e| {
                format!(
                    "{}: {}",
                    e.code.as_deref().unwrap_or("unknown"),
                    e.message.as_deref().unwrap_or("Unknown error")
                )
            });

            let _: MeetingRecordingModel = MeetingRecordingApi::update_status(
                db,
                recording.id,
                MeetingRecordingStatus::Failed,
                error_message,
            )
            .await?;
        }
        _ => {
            debug!("Ignoring unhandled Recall.ai event: {}", payload.event);
        }
    }

    Ok((
        StatusCode::OK,
        Json(WebhookResponse {
            status: "ok".to_string(),
        }),
    ))
}

/// AssemblyAI webhook payload
///
/// Note: AssemblyAI webhooks are notifications only - they don't include the
/// actual transcript data. We must fetch the full transcript via the API when
/// we receive a "completed" notification.
#[derive(Debug, Deserialize)]
pub struct AssemblyAiWebhookPayload {
    /// The transcript ID - AssemblyAI sends this as "id" in webhook payload
    #[serde(alias = "id")]
    pub transcript_id: String,
    /// Status: queued, processing, completed, error
    pub status: TranscriptStatus,
    /// Error message if failed
    #[serde(default)]
    pub error: Option<String>,
}

/// POST /webhooks/assemblyai
///
/// Handles webhook callbacks from AssemblyAI when transcription is complete.
/// This endpoint validates via webhook secret header.
pub async fn assemblyai_webhook(
    State(app_state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AssemblyAiWebhookPayload>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "Received AssemblyAI webhook for transcript: {}",
        payload.transcript_id
    );

    let config = &app_state.config;
    let db = app_state.db_conn_ref();

    // Validate webhook secret if configured
    if let Some(expected_secret) = config.webhook_secret() {
        let provided_secret = headers
            .get("x-webhook-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if provided_secret != expected_secret {
            warn!("Invalid AssemblyAI webhook secret received");
            return Ok((
                StatusCode::UNAUTHORIZED,
                Json(WebhookResponse {
                    status: "unauthorized".to_string(),
                }),
            ));
        }
    }

    // Find the transcription by AssemblyAI transcript ID
    let transcription: Option<TranscriptionModel> =
        TranscriptionApi::find_by_assemblyai_id(db, &payload.transcript_id).await?;

    let transcription = match transcription {
        Some(t) => t,
        None => {
            warn!(
                "Received webhook for unknown AssemblyAI transcript: {}",
                payload.transcript_id
            );
            return Ok((
                StatusCode::OK,
                Json(WebhookResponse {
                    status: "ignored".to_string(),
                }),
            ));
        }
    };

    match payload.status {
        TranscriptStatus::Completed => {
            info!(
                "AssemblyAI transcription completed: {}, fetching full transcript...",
                payload.transcript_id
            );

            // AssemblyAI webhooks are notifications only - they don't include the transcript data.
            // We need to fetch the full transcript via the API.
            let full_transcript = match fetch_assemblyai_transcript(
                db,
                config,
                &transcription,
                &payload.transcript_id,
            )
            .await
            {
                Ok(t) => t,
                Err(e) => {
                    warn!("Failed to fetch full transcript from AssemblyAI: {:?}", e);
                    // Mark as failed since we can't get the data
                    let _: TranscriptionModel = TranscriptionApi::update_status(
                        db,
                        transcription.id,
                        TranscriptionStatus::Failed,
                        Some(format!("Failed to fetch transcript: {}", e)),
                    )
                    .await?;
                    return Ok((
                        StatusCode::OK,
                        Json(WebhookResponse {
                            status: "fetch_failed".to_string(),
                        }),
                    ));
                }
            };

            debug!(
                "Fetched full transcript - has_text: {}, text_len: {}, has_chapters: {}, has_utterances: {}",
                full_transcript.text.is_some(),
                full_transcript.text.as_ref().map(|t| t.len()).unwrap_or(0),
                full_transcript.chapters.is_some(),
                full_transcript.utterances.is_some()
            );

            // Build summary from chapters if available
            let summary = full_transcript.chapters.as_ref().map(|chapters| {
                chapters
                    .iter()
                    .map(|c| format!("**{}**\n{}", c.headline, c.summary))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            });

            // Calculate word count from full text
            let word_count = full_transcript
                .text
                .as_ref()
                .map(|t| t.split_whitespace().count() as i32);

            debug!(
                "AssemblyAI processing - summary_len: {}, word_count: {:?}",
                summary.as_ref().map(|s| s.len()).unwrap_or(0),
                word_count
            );

            // Update transcription with completed data
            let mut updated = transcription.clone();
            updated.status = TranscriptionStatus::Completed;
            updated.full_text = full_transcript.text.clone();
            updated.summary = summary;
            updated.confidence_score = full_transcript.confidence;
            updated.word_count = word_count;

            let updated_transcription: TranscriptionModel =
                TranscriptionApi::update(db, transcription.id, updated).await?;

            info!(
                "Updated transcription {} - has_full_text: {}, has_summary: {}",
                updated_transcription.id,
                updated_transcription.full_text.is_some(),
                updated_transcription.summary.is_some()
            );

            // Store transcript segments (utterances) if available
            if let Some(ref utterances) = full_transcript.utterances {
                let utterance_count = utterances.len();
                let segments: Vec<SegmentInput> = utterances
                    .iter()
                    .map(|u| SegmentInput {
                        speaker_label: u.speaker.clone(),
                        text: u.text.clone(),
                        start_time_ms: u.start,
                        end_time_ms: u.end,
                        confidence: Some(u.confidence),
                        sentiment: None, // Would need sentiment_analysis_results for per-segment sentiment
                    })
                    .collect();

                if !segments.is_empty() {
                    let _ =
                        transcript_segment::create_batch(db, updated_transcription.id, segments)
                            .await?;
                    info!(
                        "Created {} transcript segments for transcription {}",
                        utterance_count, updated_transcription.id
                    );
                }
            }

            // Process transcript with LeMUR for intelligent extraction
            // Get coaching context (coach/coachee info) for LeMUR prompts
            if let Err(e) = process_transcription_with_lemur(
                db,
                config,
                &updated_transcription,
                &payload.transcript_id,
            )
            .await
            {
                warn!(
                    "LeMUR processing failed, falling back to keyword extraction: {:?}",
                    e
                );

                // Fallback to simple keyword-based extraction
                let action_items = extract_action_items(&full_transcript);
                if !action_items.is_empty() {
                    info!(
                        "Extracted {} action items from transcript {} (fallback)",
                        action_items.len(),
                        updated_transcription.id
                    );

                    for action_text in action_items {
                        match AiSuggestedItemApi::create(
                            db,
                            updated_transcription.id,
                            AiSuggestionType::Action,
                            action_text.clone(),
                            Some(action_text),
                            None, // confidence
                            None, // stated_by_user_id
                            None, // assigned_to_user_id
                            None, // source_segment_id
                        )
                        .await
                        {
                            Ok(suggestion) => {
                                debug!("Created AI suggestion (fallback): {}", suggestion.id);
                            }
                            Err(e) => {
                                warn!("Failed to create AI suggestion: {:?}", e);
                            }
                        }
                    }
                }
            }

            // Update meeting recording status to completed
            let _: MeetingRecordingModel = MeetingRecordingApi::update_status(
                db,
                transcription.meeting_recording_id,
                MeetingRecordingStatus::Completed,
                None,
            )
            .await?;
        }
        TranscriptStatus::Error => {
            warn!("AssemblyAI transcription failed: {}", payload.transcript_id);

            let _: TranscriptionModel = TranscriptionApi::update_status(
                db,
                transcription.id,
                TranscriptionStatus::Failed,
                payload.error.clone(),
            )
            .await?;

            // Update meeting recording with error
            let _: MeetingRecordingModel = MeetingRecordingApi::update_status(
                db,
                transcription.meeting_recording_id,
                MeetingRecordingStatus::Failed,
                payload.error,
            )
            .await?;
        }
        TranscriptStatus::Processing | TranscriptStatus::Queued => {
            debug!(
                "AssemblyAI transcription still processing: {}",
                payload.transcript_id
            );
            // No action needed - these are status updates during processing
        }
    }

    Ok((
        StatusCode::OK,
        Json(WebhookResponse {
            status: "ok".to_string(),
        }),
    ))
}

/// Trigger AssemblyAI transcription for a completed recording.
///
/// This looks up the coach's AssemblyAI API key and creates a transcription
/// request with the recording URL.
async fn trigger_assemblyai_transcription(
    db: &sea_orm::DatabaseConnection,
    config: &service::config::Config,
    recording_id: domain::Id,
    coaching_session_id: domain::Id,
    video_url: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Get the coaching session to find the relationship
    let session = CoachingSessionApi::find_by_id(db, coaching_session_id).await?;

    // 2. Get the coaching relationship to find the coach
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // 3. Get the coach's user integrations
    let user_integrations = UserIntegrationApi::find_by_user_id(db, relationship.coach_id)
        .await?
        .ok_or("Coach has no integrations configured")?;

    // 4. Get the AssemblyAI API key
    let api_key = user_integrations
        .assembly_ai_api_key
        .as_ref()
        .ok_or("AssemblyAI API key not configured for coach")?;

    // 5. Create a transcription record
    let mut transcription = TranscriptionApi::create(db, recording_id).await?;
    transcription.status = TranscriptionStatus::Processing;

    // 6. Build the webhook URL for AssemblyAI callbacks
    let webhook_url = config
        .webhook_base_url()
        .map(|base| format!("{}/webhooks/assemblyai", base));
    let webhook_secret = config.webhook_secret().map(|s| s.to_string());

    // 7. Create AssemblyAI client and send transcription request
    let client = AssemblyAiClient::new(api_key, config.assembly_ai_base_url())?;

    let request =
        create_standard_transcript_request(video_url.to_string(), webhook_url, webhook_secret);

    let response = client.create_transcript(request).await?;

    // 8. Update transcription with AssemblyAI transcript ID
    transcription.assemblyai_transcript_id = Some(response.id.clone());
    TranscriptionApi::update(db, transcription.id, transcription).await?;

    info!(
        "Created AssemblyAI transcript {} for recording {}",
        response.id, recording_id
    );

    Ok(())
}

/// Fetch the full transcript from AssemblyAI.
///
/// AssemblyAI webhooks only notify that a transcript is ready - the actual
/// transcript data must be fetched via a separate API call.
async fn fetch_assemblyai_transcript(
    db: &sea_orm::DatabaseConnection,
    config: &service::config::Config,
    transcription: &TranscriptionModel,
    transcript_id: &str,
) -> Result<
    domain::gateway::assembly_ai::TranscriptResponse,
    Box<dyn std::error::Error + Send + Sync>,
> {
    // 1. Get the meeting recording to find the coaching session
    let recording = MeetingRecordingApi::find_by_id(db, transcription.meeting_recording_id).await?;

    // 2. Get the coaching session to find the relationship
    let session = CoachingSessionApi::find_by_id(db, recording.coaching_session_id).await?;

    // 3. Get the coaching relationship to find the coach
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // 4. Get the coach's user integrations
    let user_integrations = UserIntegrationApi::find_by_user_id(db, relationship.coach_id)
        .await?
        .ok_or("Coach has no integrations configured")?;

    // 5. Get the AssemblyAI API key
    let api_key = user_integrations
        .assembly_ai_api_key
        .as_ref()
        .ok_or("AssemblyAI API key not configured for coach")?;

    // 6. Create AssemblyAI client and fetch the full transcript
    let client = AssemblyAiClient::new(api_key, config.assembly_ai_base_url())?;
    let transcript = client.get_transcript(transcript_id).await?;

    Ok(transcript)
}

/// Process transcription with LeMUR for intelligent action/agreement extraction.
///
/// Uses AssemblyAI's LeMUR API to:
/// 1. Extract actions with assignees (coach or coachee)
/// 2. Extract agreements (mutual commitments)
/// 3. Generate coaching-focused summary
///
/// Respects the coach's auto_approve_ai_suggestions setting to either:
/// - Create AI suggestions for review (auto_approve = false)
/// - Create Actions/Agreements directly (auto_approve = true)
async fn process_transcription_with_lemur(
    db: &sea_orm::DatabaseConnection,
    config: &service::config::Config,
    transcription: &TranscriptionModel,
    transcript_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Processing transcription {} with LeMUR", transcription.id);

    // 1. Get coaching context from meeting recording -> session -> relationship
    let recording = MeetingRecordingApi::find_by_id(db, transcription.meeting_recording_id).await?;
    let session = CoachingSessionApi::find_by_id(db, recording.coaching_session_id).await?;
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // 2. Get coach and coachee names for LeMUR prompts
    let coach = UserApi::find_by_id(db, relationship.coach_id).await?;
    let coachee = UserApi::find_by_id(db, relationship.coachee_id).await?;

    let coach_name = format!("{} {}", coach.first_name, coach.last_name);
    let coachee_name = format!("{} {}", coachee.first_name, coachee.last_name);

    // 3. Get coach's integration settings for auto-approve and API key
    let user_integrations = UserIntegrationApi::find_by_user_id(db, relationship.coach_id)
        .await?
        .ok_or("Coach has no integrations configured")?;
    let auto_approve = user_integrations.auto_approve_ai_suggestions;
    let api_key = user_integrations
        .assembly_ai_api_key
        .as_ref()
        .ok_or("AssemblyAI API key not configured for coach")?;

    info!(
        "LeMUR processing for coach {} (auto_approve: {})",
        relationship.coach_id, auto_approve
    );

    // 4. Create LeMUR client
    let client = AssemblyAiClient::new(api_key, config.assembly_ai_base_url())?;

    // 5. Extract actions and agreements using LeMUR
    let extraction = client
        .extract_actions_and_agreements(transcript_id, &coach_name, &coachee_name)
        .await?;

    info!(
        "LeMUR extracted {} actions and {} agreements",
        extraction.actions.len(),
        extraction.agreements.len()
    );

    // 6. Build speaker mapping (speaker label -> user_id)
    // AssemblyAI uses "A", "B" etc. We'll map based on who speaks more (typically coachee)
    // For now, use a simple heuristic - this could be improved with voice enrollment
    let speaker_to_user = build_speaker_mapping(
        db,
        transcription.id,
        relationship.coach_id,
        relationship.coachee_id,
    )
    .await;

    // 7. Process extracted actions
    for action in extraction.actions {
        let stated_by = speaker_to_user.get(&action.stated_by_speaker).copied();
        let assigned_to = match action.assigned_to_role.to_lowercase().as_str() {
            "coach" => Some(relationship.coach_id),
            "coachee" => Some(relationship.coachee_id),
            _ => None,
        };

        if auto_approve {
            // Create Action entity directly using a partial model
            use domain::actions::Model as ActionModel;
            use domain::status::Status as ActionStatus;

            let now = chrono::Utc::now();
            let action_model = ActionModel {
                id: domain::Id::default(),
                coaching_session_id: session.id,
                user_id: relationship.coach_id,
                body: Some(action.content.clone()),
                due_by: Some(now.into()),
                status: ActionStatus::NotStarted,
                status_changed_at: now.into(),
                created_at: now.into(),
                updated_at: now.into(),
            };

            match ActionApi::create(db, action_model, relationship.coach_id).await {
                Ok(created_action) => {
                    info!(
                        "Auto-created Action {} from LeMUR for session {}",
                        created_action.id, session.id
                    );
                }
                Err(e) => {
                    warn!("Failed to auto-create action: {:?}", e);
                }
            }
        } else {
            // Create AI suggestion for review
            match AiSuggestedItemApi::create(
                db,
                transcription.id,
                AiSuggestionType::Action,
                action.content.clone(),
                Some(action.source_text),
                Some(action.confidence),
                stated_by,
                assigned_to,
                None, // source_segment_id - TODO: map via timestamp
            )
            .await
            {
                Ok(suggestion) => {
                    info!(
                        "Created AI action suggestion {} for review (transcription: {})",
                        suggestion.id, transcription.id
                    );
                }
                Err(e) => {
                    warn!("Failed to create AI action suggestion: {:?}", e);
                }
            }
        }
    }

    // 8. Process extracted agreements
    for agreement in extraction.agreements {
        let stated_by = speaker_to_user.get(&agreement.stated_by_speaker).copied();

        if auto_approve {
            // Create Agreement entity directly using a partial model
            use domain::agreements::Model as AgreementModel;

            let agreement_model = AgreementModel {
                id: domain::Id::default(),
                coaching_session_id: session.id,
                user_id: relationship.coach_id,
                body: Some(agreement.content.clone()),
                created_at: chrono::Utc::now().into(),
                updated_at: chrono::Utc::now().into(),
            };

            match AgreementApi::create(db, agreement_model, relationship.coach_id).await {
                Ok(created_agreement) => {
                    info!(
                        "Auto-created Agreement {} from LeMUR for session {}",
                        created_agreement.id, session.id
                    );
                }
                Err(e) => {
                    warn!("Failed to auto-create agreement: {:?}", e);
                }
            }
        } else {
            // Create AI suggestion for review (agreements have no assignee)
            match AiSuggestedItemApi::create(
                db,
                transcription.id,
                AiSuggestionType::Agreement,
                agreement.content.clone(),
                Some(agreement.source_text),
                Some(agreement.confidence),
                stated_by,
                None, // agreements have no single assignee
                None, // source_segment_id
            )
            .await
            {
                Ok(suggestion) => {
                    info!(
                        "Created AI agreement suggestion {} for review (transcription: {})",
                        suggestion.id, transcription.id
                    );
                }
                Err(e) => {
                    warn!("Failed to create AI agreement suggestion: {:?}", e);
                }
            }
        }
    }

    // 9. Generate and store coaching summary
    match client
        .generate_coaching_summary(transcript_id, &coach_name, &coachee_name)
        .await
    {
        Ok(summary) => {
            // Store as JSON in the transcription summary field
            if let Ok(summary_json) = serde_json::to_string(&summary) {
                let mut updated_transcription = transcription.clone();
                updated_transcription.summary = Some(summary_json);
                if let Err(e) =
                    TranscriptionApi::update(db, transcription.id, updated_transcription).await
                {
                    warn!("Failed to update transcription with LeMUR summary: {:?}", e);
                } else {
                    info!(
                        "Stored LeMUR coaching summary for transcription {}",
                        transcription.id
                    );
                }
            }
        }
        Err(e) => {
            warn!("Failed to generate LeMUR coaching summary: {:?}", e);
            // Don't fail the whole process - keep the chapter-based summary
        }
    }

    Ok(())
}

/// Build a mapping from speaker labels to user IDs.
///
/// Uses a simple heuristic: in a coaching session, the coachee typically speaks more.
/// This could be improved with voice enrollment or explicit speaker identification.
async fn build_speaker_mapping(
    db: &sea_orm::DatabaseConnection,
    transcription_id: domain::Id,
    coach_id: domain::Id,
    coachee_id: domain::Id,
) -> std::collections::HashMap<String, domain::Id> {
    let mut mapping = std::collections::HashMap::new();

    // Get transcript segments to analyze speaker distribution
    match transcript_segment::find_by_transcription_id(db, transcription_id).await {
        Ok(segments) => {
            // Count words per speaker
            let mut speaker_word_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();

            for segment in &segments {
                let word_count = segment.text.split_whitespace().count();
                *speaker_word_counts
                    .entry(segment.speaker_label.clone())
                    .or_insert(0) += word_count;
            }

            // Sort speakers by word count (descending)
            let mut speakers: Vec<_> = speaker_word_counts.into_iter().collect();
            speakers.sort_by(|a, b| b.1.cmp(&a.1));

            // Heuristic: speaker with most words is likely the coachee
            // (coaches typically ask questions and listen more)
            if speakers.len() >= 2 {
                mapping.insert(speakers[0].0.clone(), coachee_id);
                mapping.insert(speakers[1].0.clone(), coach_id);
            } else if speakers.len() == 1 {
                // Only one speaker detected - unusual but possible
                mapping.insert(speakers[0].0.clone(), coachee_id);
            }

            debug!("Built speaker mapping: {:?}", mapping);
        }
        Err(e) => {
            warn!(
                "Failed to get transcript segments for speaker mapping: {:?}",
                e
            );
        }
    }

    mapping
}
