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
use domain::error::{DomainErrorKind, EntityErrorKind, Error as DomainError, InternalErrorKind};
use domain::transcript_suggestion::review::{self, ActionEdits};
use domain::Id;
use log::*;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use service::config::ApiVersion;

/// Only suggested Actions exist so far.
const ACTION_TARGET: &str = "action";

#[derive(Debug, Deserialize)]
pub struct IndexParams {
    target: Option<String>,
}

/// GET the pending suggested Actions for a coaching session
///
/// Lists suggestions oldest first. Only `target=action`, the default, is supported.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcript_suggestions",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("target" = Option<String>, Query, description = "Kind of suggestion to list; only `action` (the default) is supported"),
    ),
    responses(
        (status = 200, description = "Pending suggestions retrieved", body = [domain::transcript_suggestion::review::ActionSuggestion]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching session not found"),
        (status = 422, description = "Unsupported target"),
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

    params
        .target
        .as_deref()
        .filter(|target| *target != ACTION_TARGET)
        .map_or(Ok(()), |target| {
            Err(validation(format!(
                "'{target}' is not a supported target. Expected: {ACTION_TARGET}."
            )))
        })?;

    let suggestions =
        review::list_pending_actions(app_state.db_conn_ref(), &session, Utc::now()).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), suggestions)))
}

/// PUT edit a pending suggested Action
///
/// Replaces the suggestion's body, due date and assignees and returns the updated suggestion.
#[utoipa::path(
    put,
    path = "/coaching_sessions/{coaching_session_id}/transcript_suggestions/{suggestion_id}",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("suggestion_id" = Uuid, Path, description = "Suggestion id"),
    ),
    request_body = ActionEdits,
    responses(
        (status = 200, description = "Suggestion updated", body = domain::transcript_suggestion::review::ActionSuggestion),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The coachee may not edit this suggestion"),
        (status = 404, description = "Suggestion not found in this session"),
        (status = 409, description = "The suggestion is no longer pending"),
        (status = 422, description = "Invalid edits"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    TranscriptSuggestionAccess {
        session,
        suggestion,
    }: TranscriptSuggestionAccess,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(edits): Json<ActionEdits>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT transcript suggestion {}", suggestion.id);

    let db = app_state.db_conn_ref();
    let id: Id = suggestion.id;
    review::edit_action(
        db,
        app_state.event_publisher.as_ref(),
        &user,
        suggestion,
        edits,
    )
    .await?;

    let updated = review::list_pending_actions(db, &session, Utc::now())
        .await?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(not_found)?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
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
    TranscriptSuggestionAccess { suggestion, .. }: TranscriptSuggestionAccess,
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
    TranscriptSuggestionAccess { suggestion, .. }: TranscriptSuggestionAccess,
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
    TranscriptSuggestionAccess { suggestion, .. }: TranscriptSuggestionAccess,
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

fn validation(message: String) -> Error {
    Error::Domain(DomainError {
        source: None,
        error_kind: DomainErrorKind::Validation(message),
    })
}

fn not_found() -> Error {
    Error::Domain(DomainError {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound)),
    })
}
