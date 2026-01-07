//! Controller for transcript and transcription operations.
//!
//! Handles retrieval of transcripts and transcript segments for coaching sessions.

use crate::controller::ApiResponse;
use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::{AppState, Error};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use domain::coaching_relationship as CoachingRelationshipApi;
use domain::coaching_session as CoachingSessionApi;
use domain::meeting_recording as MeetingRecordingApi;
use domain::transcript_segment;
use domain::transcript_segments::Model as TranscriptSegmentModel;
use domain::transcription as TranscriptionApi;
use domain::transcriptions::Model as TranscriptionModel;
use domain::Id;
use log::*;
use service::config::ApiVersion;

/// GET /coaching_sessions/{id}/transcript
///
/// Get the transcription for a coaching session.
/// Returns the transcript with summary, full text, and metadata.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}/transcript",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Transcription retrieved", body = transcriptions::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "No transcription found for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn get_transcript(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET transcript for session: {session_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach or coachee can view the transcript
    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to view transcript for session {} but is not a participant",
            user.id, session_id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "Not authorized to view this transcript".to_string(),
                )),
            ),
        }));
    }

    // 3. Get the latest recording for this session
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::NotFound,
                    ),
                ),
            })
        })?;

    // 4. Get the transcription for this recording
    let transcription: TranscriptionModel =
        TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
            .await?
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                    source: None,
                    error_kind: domain::error::DomainErrorKind::Internal(
                        domain::error::InternalErrorKind::Entity(
                            domain::error::EntityErrorKind::NotFound,
                        ),
                    ),
                })
            })?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), transcription)))
}

/// GET /coaching_sessions/{id}/transcript/segments
///
/// Get the transcript segments (utterances) for a coaching session.
/// Returns speaker-labeled segments with timestamps.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}/transcript/segments",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Transcript segments retrieved", body = Vec<transcript_segments::Model>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "No transcript found for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn get_transcript_segments(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET transcript segments for session: {session_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach or coachee can view the transcript
    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to view transcript segments for session {} but is not a participant",
            user.id, session_id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "Not authorized to view this transcript".to_string(),
                )),
            ),
        }));
    }

    // 3. Get the latest recording for this session
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::NotFound,
                    ),
                ),
            })
        })?;

    // 4. Get the transcription for this recording
    let transcription: TranscriptionModel =
        TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
            .await?
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                    source: None,
                    error_kind: domain::error::DomainErrorKind::Internal(
                        domain::error::InternalErrorKind::Entity(
                            domain::error::EntityErrorKind::NotFound,
                        ),
                    ),
                })
            })?;

    // 5. Get the segments for this transcription
    let segments: Vec<TranscriptSegmentModel> =
        transcript_segment::find_by_transcription_id(db, transcription.id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), segments)))
}

/// GET /coaching_sessions/{id}/summary
///
/// Get just the AI-generated summary for a coaching session.
/// This is a convenience endpoint that returns only the summary text.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}/summary",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Summary retrieved", body = SummaryResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "No summary available for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn get_session_summary(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET summary for session: {session_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach or coachee can view the summary
    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to view summary for session {} but is not a participant",
            user.id, session_id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "Not authorized to view this summary".to_string(),
                )),
            ),
        }));
    }

    // 3. Get the latest recording for this session
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::NotFound,
                    ),
                ),
            })
        })?;

    // 4. Get the transcription for this recording
    let transcription: TranscriptionModel =
        TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
            .await?
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                    source: None,
                    error_kind: domain::error::DomainErrorKind::Internal(
                        domain::error::InternalErrorKind::Entity(
                            domain::error::EntityErrorKind::NotFound,
                        ),
                    ),
                })
            })?;

    // 5. Check if we have a summary
    let summary = transcription.summary.ok_or_else(|| {
        Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::NotFound),
            ),
        })
    })?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        SummaryResponse {
            session_id,
            summary,
            word_count: transcription.word_count,
            confidence_score: transcription.confidence_score,
        },
    )))
}

/// Response for the summary endpoint
#[derive(Debug, serde::Serialize)]
pub struct SummaryResponse {
    pub session_id: Id,
    pub summary: String,
    pub word_count: Option<i32>,
    pub confidence_score: Option<f64>,
}

/// Response for action extraction endpoint
/// Returns immediately to indicate extraction has been triggered.
/// Actions will be created asynchronously and can be fetched via the Actions API.
#[derive(Debug, serde::Serialize)]
pub struct ExtractActionsResponse {
    pub session_id: Id,
    pub transcription_id: Id,
    /// Status of the extraction request: "processing" or "error"
    pub status: String,
    /// Human-readable message about the extraction status
    pub message: String,
}

/// Response for agreement extraction endpoint
#[derive(Debug, serde::Serialize)]
pub struct ExtractAgreementsResponse {
    pub session_id: Id,
    pub transcription_id: Id,
    pub agreements: Vec<domain::gateway::assembly_ai::ExtractedAgreement>,
    pub created_count: usize,
}

/// POST /coaching_sessions/{id}/transcript/extract-actions
///
/// Manually trigger LeMUR to extract action items from the session's transcript.
/// Returns immediately and processes extraction in the background.
/// Actions will be created asynchronously and can be fetched via the Actions API.
#[utoipa::path(
    post,
    path = "/coaching_sessions/{id}/transcript/extract-actions",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 202, description = "Extraction triggered", body = ExtractActionsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a coach for this session"),
        (status = 404, description = "No transcription found for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn extract_actions(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    info!("POST extract-actions for session: {session_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access (coach only)
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach can trigger extraction
    if relationship.coach_id != user.id {
        warn!(
            "User {} attempted to extract actions for session {} but is not the coach",
            user.id, session_id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "Only the coach can trigger action extraction".to_string(),
                )),
            ),
        }));
    }

    // 3. Get the latest recording and transcription
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::NotFound,
                    ),
                ),
            })
        })?;

    let transcription: TranscriptionModel =
        TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
            .await?
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                    source: None,
                    error_kind: domain::error::DomainErrorKind::Internal(
                        domain::error::InternalErrorKind::Entity(
                            domain::error::EntityErrorKind::NotFound,
                        ),
                    ),
                })
            })?;

    // 4. Get AssemblyAI transcript ID
    let assemblyai_transcript_id =
        transcription
            .assemblyai_transcript_id
            .clone()
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                        "No AssemblyAI transcript ID found - transcription may still be processing"
                            .to_string(),
                    )),
                ),
            })
            })?;

    // 5. Get coach's AssemblyAI API key
    let user_integrations = domain::user_integration::find_by_user_id(db, relationship.coach_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::Other(
                            "Coach has no integrations configured".to_string(),
                        ),
                    ),
                ),
            })
        })?;

    let api_key = user_integrations
        .assembly_ai_api_key
        .clone()
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::Other(
                            "AssemblyAI API key not configured".to_string(),
                        ),
                    ),
                ),
            })
        })?;

    // 6. Get coach and coachee for LeMUR prompts
    let coach = domain::user::find_by_id(db, relationship.coach_id).await?;
    let coachee = domain::user::find_by_id(db, relationship.coachee_id).await?;

    // Clone data needed for background task
    let db_clone = app_state.database_connection.clone();
    let config_clone = app_state.config.clone();
    let session_clone = session.clone();
    let relationship_clone = relationship.clone();
    let coach_clone = coach.clone();
    let coachee_clone = coachee.clone();

    // 7. Spawn background task to call LeMUR and create actions
    tokio::spawn(async move {
        let coach_name = format!("{} {}", coach_clone.first_name, coach_clone.last_name);
        let coachee_name = format!("{} {}", coachee_clone.first_name, coachee_clone.last_name);

        // Create AssemblyAI client
        let client = match domain::gateway::assembly_ai::AssemblyAiClient::new(
            &api_key,
            config_clone.assembly_ai_base_url(),
        ) {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to create AssemblyAI client: {:?}", e);
                return;
            }
        };

        // Call LeMUR to extract actions
        let extraction = match client
            .extract_actions_and_agreements(&assemblyai_transcript_id, &coach_name, &coachee_name)
            .await
        {
            Ok(e) => e,
            Err(e) => {
                error!(
                    "LeMUR extraction failed for session {}: {:?}",
                    session_clone.id, e
                );
                return;
            }
        };

        info!(
            "LeMUR extracted {} actions for session {} (background)",
            extraction.actions.len(),
            session_clone.id
        );

        // Create Action entities
        for action in extraction.actions {
            use domain::actions::Model as ActionModel;
            use domain::status::Status as ActionStatus;

            // Build action content, optionally prepending assignee name if it matches coach/coachee
            let action_content = if let Some(ref assigned_name) = action.assigned_to_name {
                // Validate that the extracted name matches coach or coachee (case-insensitive)
                let name_lower = assigned_name.to_lowercase();
                let coach_first_lower = coach_clone.first_name.to_lowercase();
                let coach_last_lower = coach_clone.last_name.to_lowercase();
                let coachee_first_lower = coachee_clone.first_name.to_lowercase();
                let coachee_last_lower = coachee_clone.last_name.to_lowercase();

                let matches_coach = name_lower == coach_first_lower
                    || name_lower == coach_last_lower
                    || name_lower == format!("{} {}", coach_first_lower, coach_last_lower);
                let matches_coachee = name_lower == coachee_first_lower
                    || name_lower == coachee_last_lower
                    || name_lower == format!("{} {}", coachee_first_lower, coachee_last_lower);

                if matches_coach || matches_coachee {
                    format!("[{}] {}", assigned_name, action.content)
                } else {
                    action.content.clone()
                }
            } else {
                action.content.clone()
            };

            // Parse due_by date if provided, otherwise use current time as fallback
            let due_by_datetime = action
                .due_by
                .as_ref()
                .and_then(|date_str| {
                    chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                        .ok()
                        .map(|date| date.and_hms_opt(23, 59, 59).unwrap().and_utc())
                })
                .unwrap_or_else(chrono::Utc::now);

            let now = chrono::Utc::now();
            let action_model = ActionModel {
                id: domain::Id::default(),
                coaching_session_id: session_clone.id,
                user_id: relationship_clone.coach_id,
                body: Some(action_content),
                due_by: Some(due_by_datetime.into()),
                status: ActionStatus::NotStarted,
                status_changed_at: now.into(),
                created_at: now.into(),
                updated_at: now.into(),
            };

            match domain::action::create(&db_clone, action_model, relationship_clone.coach_id).await
            {
                Ok(created_action) => {
                    info!(
                        "Created Action {} from LeMUR extraction for session {}",
                        created_action.id, session_clone.id
                    );
                }
                Err(e) => {
                    warn!("Failed to create action: {:?}", e);
                }
            }
        }
    });

    // Return immediately with 202 Accepted
    Ok((
        StatusCode::ACCEPTED,
        Json(ApiResponse::new(
            StatusCode::ACCEPTED.into(),
            ExtractActionsResponse {
                session_id,
                transcription_id: transcription.id,
                status: "processing".to_string(),
                message: "Action extraction has been triggered. Actions will appear shortly."
                    .to_string(),
            },
        )),
    ))
}

/// POST /coaching_sessions/{id}/transcript/extract-agreements
///
/// Manually trigger LeMUR to extract agreements from the session's transcript.
/// Creates Agreement entities directly (bypasses AI suggestions).
/// Useful for testing or re-processing a transcript.
#[utoipa::path(
    post,
    path = "/coaching_sessions/{id}/transcript/extract-agreements",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Agreements extracted and created", body = ExtractAgreementsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a coach for this session"),
        (status = 404, description = "No transcription found for this session"),
        (status = 503, description = "LeMUR service unavailable"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn extract_agreements(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    info!("POST extract-agreements for session: {session_id}");

    let db = app_state.db_conn_ref();
    let config = &app_state.config;

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access (coach only)
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach can trigger extraction
    if relationship.coach_id != user.id {
        warn!(
            "User {} attempted to extract agreements for session {} but is not the coach",
            user.id, session_id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "Only the coach can trigger agreement extraction".to_string(),
                )),
            ),
        }));
    }

    // 3. Get the latest recording and transcription
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::NotFound,
                    ),
                ),
            })
        })?;

    let transcription: TranscriptionModel =
        TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
            .await?
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                    source: None,
                    error_kind: domain::error::DomainErrorKind::Internal(
                        domain::error::InternalErrorKind::Entity(
                            domain::error::EntityErrorKind::NotFound,
                        ),
                    ),
                })
            })?;

    // 4. Get AssemblyAI transcript ID
    let transcript_id = transcription.assemblyai_transcript_id.as_ref().ok_or_else(|| {
        Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "No AssemblyAI transcript ID found - transcription may still be processing".to_string(),
                )),
            ),
        })
    })?;

    // 5. Get coach's AssemblyAI API key
    let user_integrations = domain::user_integration::find_by_user_id(db, relationship.coach_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::Other(
                            "Coach has no integrations configured".to_string(),
                        ),
                    ),
                ),
            })
        })?;

    let api_key = user_integrations
        .assembly_ai_api_key
        .as_ref()
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::Other(
                            "AssemblyAI API key not configured".to_string(),
                        ),
                    ),
                ),
            })
        })?;

    // 6. Get coach and coachee names for LeMUR prompts
    let coach = domain::user::find_by_id(db, relationship.coach_id).await?;
    let coachee = domain::user::find_by_id(db, relationship.coachee_id).await?;
    let coach_name = format!("{} {}", coach.first_name, coach.last_name);
    let coachee_name = format!("{} {}", coachee.first_name, coachee.last_name);

    // 7. Call LeMUR to extract actions and agreements
    let client =
        domain::gateway::assembly_ai::AssemblyAiClient::new(api_key, config.assembly_ai_base_url())
            .map_err(|e| {
                Error::Domain(domain::error::Error {
                    source: Some(Box::new(e)),
                    error_kind: domain::error::DomainErrorKind::External(
                        domain::error::ExternalErrorKind::Other(
                            "Failed to create AssemblyAI client".to_string(),
                        ),
                    ),
                })
            })?;

    let extraction = client
        .extract_actions_and_agreements(transcript_id, &coach_name, &coachee_name)
        .await
        .map_err(|e| {
            warn!("LeMUR extraction failed: {:?}", e);
            Error::Domain(e)
        })?;

    info!(
        "LeMUR extracted {} agreements for session {}",
        extraction.agreements.len(),
        session_id
    );

    // 8. Create Agreement entities directly
    let mut created_count = 0;
    for agreement in &extraction.agreements {
        use domain::agreements::Model as AgreementModel;

        let agreement_model = AgreementModel {
            id: domain::Id::default(),
            coaching_session_id: session.id,
            user_id: relationship.coach_id,
            body: Some(agreement.content.clone()),
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        };

        match domain::agreement::create(db, agreement_model, relationship.coach_id).await {
            Ok(created_agreement) => {
                info!(
                    "Created Agreement {} from manual LeMUR extraction for session {}",
                    created_agreement.id, session.id
                );
                created_count += 1;
            }
            Err(e) => {
                warn!("Failed to create agreement: {:?}", e);
            }
        }
    }

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        ExtractAgreementsResponse {
            session_id,
            transcription_id: transcription.id,
            agreements: extraction.agreements,
            created_count,
        },
    )))
}
