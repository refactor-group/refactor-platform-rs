use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{action as ActionApi, actions::Model, Id};

use serde_json::json;
use service::config::ApiVersion;
use std::collections::HashMap;

use log::*;

/// POST create a new Action
#[utoipa::path(
    post,
    path = "/actions",
    params(ApiVersion),
    request_body = actions::Model,
    responses(
        (status = 201, description = "Successfully Created a New Action", body = [actions::Model]),
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
    Json(action_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST Create a New Action from: {:?}", action_model);

    let action = ActionApi::create(app_state.db_conn_ref(), action_model, user.id).await?;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), action)))
}

/// GET a particular Action specified by its id.
#[utoipa::path(
    get,
    path = "/actions/{id}",
    params(
        ApiVersion,
        ("id" = String, Path, description = "Action id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a specific Action by its id", body = [notes::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Action not found"),
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
    debug!("GET Action by id: {}", id);

    let action = ActionApi::find_by_id(app_state.db_conn_ref(), id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), action)))
}

#[utoipa::path(
    put,
    path = "/actions/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Id of action to update"),
    ),
    request_body = actions::Model,
    responses(
        (status = 200, description = "Successfully Updated Action", body = [actions::Model]),
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
    Json(action_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT Update Action with id: {}", id);

    let action = ActionApi::update(app_state.db_conn_ref(), id, action_model).await?;

    debug!("Updated Action: {:?}", action);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), action)))
}

#[utoipa::path(
    put,
    path = "/actions/{id}/status",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Id of action to update"),
        ("value" = Option<String>, Query, description = "Status value to update"),
    ),
    request_body = actions::Model,
    responses(
        (status = 200, description = "Successfully Updated Action", body = [actions::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update_status(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    Query(status): Query<String>,
    Path(id): Path<Id>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT Update Action Status with id: {}", id);

    let action =
        ActionApi::update_status(app_state.db_conn_ref(), id, status.as_str().into()).await?;

    debug!("Updated Action: {:?}", action);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), action)))
}

#[utoipa::path(
    get,
    path = "/actions",
    params(
        ApiVersion,
        ("coaching_session_id" = Option<Id>, Query, description = "Filter by coaching_session_id")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Actions", body = [actions::Model]),
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
    debug!("GET all Actions");
    debug!("Filter Params: {:?}", params);

    let actions = ActionApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Actions: {:?}", actions);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), actions)))
}

/// DELETE an Action specified by its primary key.
#[utoipa::path(
    delete,
    path = "/actions/{id}",
    params(
        ApiVersion,
        ("id" = i32, Path, description = "Action id to delete")
    ),
    responses(
        (status = 200, description = "Successfully deleted a certain Action by its id", body = [i32]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Action not found"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("DELETE Action by id: {}", id);

    ActionApi::delete_by_id(app_state.db_conn_ref(), id).await?;
    Ok(Json(json!({"id": id})))
}
