use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{note as NoteApi, notes, Id};
use service::config::ApiVersion;
use std::collections::HashMap;

use log::*;

/// POST create a new Note
#[utoipa::path(
    post,
    path = "/notes",
    params(ApiVersion),
    request_body = notes::Model,
    responses(
        (status = 201, description = "Successfully Created a New Note", body = [notes::Model]),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]

pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Json(note_model): Json<notes::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST Create a New Note from: {:?}", note_model);

    let note = NoteApi::create(app_state.db_conn_ref(), note_model, user.id).await?;

    debug!("New Note: {:?}", note);

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), note)))
}

#[utoipa::path(
    put,
    path = "/notes/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Id of note to update"),
    ),
    request_body = notes::Model,
    responses(
        (status = 200, description = "Successfully Updated Note", body = [notes::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
    Json(note_model): Json<notes::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT Update Note with id: {}", id);

    let note = NoteApi::update(app_state.db_conn_ref(), id, note_model).await?;

    debug!("Updated Note: {:?}", note);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), note)))
}

#[utoipa::path(
    get,
    path = "/notes",
    params(
        ApiVersion,
        ("coaching_session_id" = Option<Id>, Query, description = "Filter by coaching_session_id")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Notes", body = [coaching_sessions::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Notes");
    debug!("Filter Params: {:?}", params);

    let notes = NoteApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Notes: {:?}", notes);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), notes)))
}

/// GET a particular Note specified by its id.
#[utoipa::path(
    get,
    path = "/notes/{id}",
    params(
        ApiVersion,
        ("id" = String, Path, description = "Note id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a certain Note by its id", body = [notes::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Note not found"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Organization by id: {}", id);

    let note: Option<notes::Model> = NoteApi::find_by_id(app_state.db_conn_ref(), id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), note)))
}
