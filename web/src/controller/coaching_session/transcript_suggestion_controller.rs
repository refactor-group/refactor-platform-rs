use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, coaching_session_access::CoachingSessionAccess,
    compare_api_version::CompareApiVersion,
    transcript_suggestion_access::TranscriptSuggestionAccess,
};
use crate::{AppState, Error};
use axum::async_trait;
use axum::body::Bytes;
use axum::extract::{FromRequest, Query, Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use domain::error::{DomainErrorKind, Error as DomainError};
use domain::transcript_suggestion::review::{self, ActionEdits, Scan};
use domain::transcript_suggestion::Status;
use log::*;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use service::config::ApiVersion;

/// Only suggested Actions exist so far.
const ACTION_TARGET: &str = "action";

#[derive(Debug, Deserialize)]
pub struct IndexParams {
    target: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScanParams {
    target: Option<String>,
}

/// GET a coaching session's suggested Actions
///
/// Lists pending suggestions by default, or dismissed ones with `status=dismissed`, oldest
/// first. Only `target=action`, the default, is supported.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcript_suggestions",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("target" = Option<String>, Query, description = "Kind of suggestion to list; only `action` (the default) is supported"),
        ("status" = Option<String>, Query, description = "pending (default) or dismissed"),
    ),
    responses(
        (status = 200, description = "Suggestions in the requested status retrieved", body = [domain::transcript_suggestion::review::ActionSuggestion]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching session not found"),
        (status = 422, description = "Unsupported target or status"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET transcript suggestions for session {}", session.id);

    supported_target(params.target.as_deref())?;
    let status = listed_status(params.status.as_deref())?;

    let suggestions =
        review::list_actions(app_state.db_conn_ref(), &session, status, Utc::now()).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), suggestions)))
}

/// GET whether a coaching session's transcript was scanned for suggested Actions
///
/// Reports the scan of the session's latest completed transcript: `unavailable` when
/// suggestions are not configured or no transcript completed, `none` when it was never
/// scanned, otherwise `pending`, `processing`, `completed` or `failed`.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcript_suggestions/scan",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("target" = Option<String>, Query, description = "Kind of suggestion; only `action` (the default) is supported"),
    ),
    responses(
        (status = 200, description = "Scan state retrieved", body = domain::transcript_suggestion::review::Scan),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not the session's coach or coachee"),
        (status = 404, description = "Coaching session not found"),
        (status = 422, description = "Unsupported target"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn scan(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
    Query(params): Query<ScanParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET transcript scan for session {}", session.id);

    supported_target(params.target.as_deref())?;
    let status = review::scan_status(app_state.db_conn_ref(), &app_state.config, &session).await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        Scan { status },
    )))
}

/// POST start a scan of a coaching session's transcript for suggested Actions
///
/// Queues a scan of the latest completed transcript when it was never scanned or the scan
/// failed. A running or completed scan is never repeated.
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/transcript_suggestions/scan",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("target" = Option<String>, Query, description = "Kind of suggestion; only `action` (the default) is supported"),
    ),
    responses(
        (status = 202, description = "Scan queued", body = domain::transcript_suggestion::review::Scan),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Not the session's coach or coachee"),
        (status = 404, description = "Coaching session not found"),
        (status = 409, description = "A scan is running or already completed"),
        (status = 422, description = "Unsupported target, or suggestions are not available for this session"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn start_scan(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
    Query(params): Query<ScanParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST transcript scan for session {}", session.id);

    supported_target(params.target.as_deref())?;
    let status = review::start_scan(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        &app_state.config,
        &session,
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(ApiResponse::new(
            StatusCode::ACCEPTED.into(),
            Scan { status },
        )),
    ))
}

/// POST accept a pending suggested Action
///
/// Creates the Action, optionally applying the edits in the body, and returns it. An empty
/// body accepts the suggestion as stored.
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/transcript_suggestions/{suggestion_id}/accept",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("suggestion_id" = Uuid, Path, description = "Suggestion id"),
    ),
    request_body(content = Option<ActionEdits>, description = "Optional edits applied before accepting"),
    responses(
        (status = 201, description = "Action created", body = domain::action::ActionWithAssignees),
        (status = 400, description = "Malformed body"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The coachee may not accept this suggestion"),
        (status = 404, description = "Suggestion not found in this session"),
        (status = 409, description = "The suggestion is no longer pending"),
        (status = 422, description = "Invalid edits"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn accept(
    CompareApiVersion(_v): CompareApiVersion,
    TranscriptSuggestionAccess { suggestion }: TranscriptSuggestionAccess,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    OptionalJson(edits): OptionalJson<ActionEdits>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST accept transcript suggestion {}", suggestion.id);

    let created = review::accept_action(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        &app_state.config,
        &user,
        suggestion,
        edits,
        Utc::now(),
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), created)))
}

/// POST dismiss a pending suggested Action
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/transcript_suggestions/{suggestion_id}/dismiss",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("suggestion_id" = Uuid, Path, description = "Suggestion id"),
    ),
    responses(
        (status = 204, description = "Suggestion dismissed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The coachee may not dismiss this suggestion"),
        (status = 404, description = "Suggestion not found in this session"),
        (status = 409, description = "The suggestion is not pending"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn dismiss(
    CompareApiVersion(_v): CompareApiVersion,
    TranscriptSuggestionAccess { suggestion }: TranscriptSuggestionAccess,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST dismiss transcript suggestion {}", suggestion.id);

    review::dismiss(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        &user,
        suggestion,
        Utc::now(),
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST restore a dismissed suggested Action to pending
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/transcript_suggestions/{suggestion_id}/restore",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("suggestion_id" = Uuid, Path, description = "Suggestion id"),
    ),
    responses(
        (status = 204, description = "Suggestion restored"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The coachee may not restore this suggestion"),
        (status = 404, description = "Suggestion not found in this session"),
        (status = 409, description = "The suggestion is not dismissed"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn restore(
    CompareApiVersion(_v): CompareApiVersion,
    TranscriptSuggestionAccess { suggestion }: TranscriptSuggestionAccess,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST restore transcript suggestion {}", suggestion.id);

    review::restore(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        &user,
        suggestion,
        Utc::now(),
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// A JSON body that may be absent; an empty body is `None`, a malformed one is a 400.
pub struct OptionalJson<T>(Option<T>);

#[async_trait]
impl<S, T> FromRequest<S> for OptionalJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Error;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|_| Error::Web(WebErrorKind::Input))?;
        Some(bytes)
            .filter(|b| !b.is_empty())
            .map(|b| serde_json::from_slice(&b))
            .transpose()
            .map(OptionalJson)
            .map_err(|_| Error::Web(WebErrorKind::Input))
    }
}

/// Rejects any target but `action`; absent means `action`.
fn supported_target(param: Option<&str>) -> Result<(), Error> {
    param
        .filter(|target| *target != ACTION_TARGET)
        .map_or(Ok(()), |target| {
            Err(validation(format!(
                "'{target}' is not a supported target. Expected: {ACTION_TARGET}."
            )))
        })
}

/// The status a list request asks for; pending when absent.
fn listed_status(param: Option<&str>) -> Result<Status, Error> {
    match param {
        None | Some("pending") => Ok(Status::Pending),
        Some("dismissed") => Ok(Status::Dismissed),
        Some(other) => Err(validation(format!(
            "'{other}' is not a supported status. Expected: pending or dismissed."
        ))),
    }
}

fn validation(message: String) -> Error {
    Error::Domain(DomainError {
        source: None,
        error_kind: DomainErrorKind::Validation(message),
    })
}

#[cfg(test)]
#[path = "transcript_suggestion_controller_tests.rs"]
mod tests;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "transcript_suggestion_scan_tests.rs"]
mod scan_tests;
