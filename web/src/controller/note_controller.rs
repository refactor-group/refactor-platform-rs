use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser,
    coaching_session_access::{authorize_session, CoachingSessionQueryAccess},
    compare_api_version::CompareApiVersion,
    note_access::NoteAccess,
};
use crate::{AppState, Error};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::{note as NoteApi, notes};
use service::config::ApiVersion;
use std::collections::HashMap;

use log::*;

/// POST create a new Note
#[utoipa::path(
    post,
    path = "/notes",
    params(ApiVersion),
    request_body = domain::notes::Model,
    responses(
        (status = 201, description = "Successfully Created a New Note", body = domain::notes::Model),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Coaching session not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]

pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(note_model): Json<notes::Model>,
) -> Result<Response, Error> {
    debug!("POST Create a New Note from: {note_model:?}");

    if let Err(rejection) =
        authorize_session(&app_state, &user, note_model.coaching_session_id).await
    {
        return Ok(rejection.into_response());
    }

    let note = NoteApi::create(app_state.db_conn_ref(), note_model, user.id).await?;

    debug!("New Note: {note:?}");

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), note)).into_response())
}

#[utoipa::path(
    put,
    path = "/notes/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Id of note to update"),
    ),
    request_body = domain::notes::Model,
    responses(
        (status = 200, description = "Successfully Updated Note", body = domain::notes::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Note not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    NoteAccess { note }: NoteAccess,
    State(app_state): State<AppState>,
    Json(note_model): Json<notes::Model>,
) -> Result<impl IntoResponse, Error> {
    let id = note.id;
    debug!("PUT Update Note with id: {id}");

    let note = NoteApi::update(app_state.db_conn_ref(), id, note_model).await?;

    debug!("Updated Note: {note:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), note)))
}

#[utoipa::path(
    get,
    path = "/notes",
    params(
        ApiVersion,
        ("coaching_session_id" = Option<Uuid>, Query, description = "Filter by coaching_session_id")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Notes", body = [domain::notes::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CoachingSessionQueryAccess(_session, _relationship): CoachingSessionQueryAccess,
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Notes");
    debug!("Filter Params: {params:?}");

    let notes = NoteApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Notes: {notes:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), notes)))
}

/// GET a particular Note specified by its id.
#[utoipa::path(
    get,
    path = "/notes/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Note id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a certain Note by its id", body = domain::notes::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Note not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    NoteAccess { note }: NoteAccess,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Note by id: {}", note.id);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), note)))
}

#[cfg(test)]
#[path = "note_controller_sqlite_tests.rs"]
mod sqlite_tests;
