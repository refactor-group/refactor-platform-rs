use serde::Deserialize;
use utoipa::ToSchema;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::action::{IndexParams, SortField};
use crate::params::WithSortDefaults;
use crate::{AppState, Error};
use domain::{action as ActionApi, actions::Model, Id};
use log::*;
use service::config::ApiVersion;

/// Request body for creating or updating an action.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ActionRequest {
    #[serde(flatten)]
    pub action: Model,
    /// Optional list of user IDs to assign to this action.
    /// For updates, if provided, replaces all existing assignees.
    /// If omitted during update, assignees remain unchanged.
    pub assignee_ids: Option<Vec<Id>>,
}

/// POST create a new Action
#[utoipa::path(
    post,
    path = "/actions",
    params(ApiVersion),
    request_body = ActionRequest,
    responses(
        (status = 201, description = "Successfully Created a New Action", body = [domain::action::ActionWithAssignees]),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
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
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Json(request): Json<ActionRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST Create a New Action from: {:?}", request.action);

    let action = ActionApi::create_with_assignees(
        app_state.db_conn_ref(),
        request.action,
        user.id,
        request.assignee_ids,
    )
    .await?;

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
        (status = 200, description = "Successfully retrieved a specific Action by its id", body = [domain::action::ActionWithAssignees]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Action not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
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
    debug!("GET Action by id: {id}");

    let action = ActionApi::find_by_id_with_assignees(app_state.db_conn_ref(), id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), action)))
}

#[utoipa::path(
    put,
    path = "/actions/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Id of action to update"),
    ),
    request_body = ActionRequest,
    responses(
        (status = 200, description = "Successfully Updated Action", body = [domain::action::ActionWithAssignees]),
        (status = 401, description = "Unauthorized"),
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
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
    Json(request): Json<ActionRequest>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT Update Action with id: {id}");

    let action = ActionApi::update_with_assignees(
        app_state.db_conn_ref(),
        id,
        request.action,
        request.assignee_ids,
    )
    .await?;

    debug!("Updated Action: {action:?}");

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
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
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
    debug!("PUT Update Action Status with id: {id}");

    let action =
        ActionApi::update_status(app_state.db_conn_ref(), id, status.as_str().into()).await?;

    debug!("Updated Action: {action:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), action)))
}

#[utoipa::path(
    get,
    path = "/actions",
    params(
        ApiVersion,
        ("coaching_session_id" = Option<Id>, Query, description = "Filter by coaching_session_id"),
        ("sort_by" = Option<crate::params::action::SortField>, Query, description = "Sort by field. Valid values: 'due_by', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "due_by"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Actions", body = [domain::action::ActionWithAssignees]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
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
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Actions");
    debug!("Filter Params: {params:?}");

    // Apply default sorting parameters
    let mut params = params;
    IndexParams::apply_sort_defaults(
        &mut params.sort_by,
        &mut params.sort_order,
        SortField::DueBy,
    );

    let actions = ActionApi::find_by_with_assignees(app_state.db_conn_ref(), params).await?;

    debug!("Found Actions: {actions:?}");

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
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
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
    debug!("DELETE Action by id: {id}");

    ActionApi::delete_by_id(app_state.db_conn_ref(), id).await?;
    Ok(Json(json!({"id": id})))
}
