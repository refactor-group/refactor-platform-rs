use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{action as ActionApi, Id};
use serde::Deserialize;
use service::config::ApiVersion;

use log::*;

/// Query parameters for filtering session actions.
#[derive(Debug, Deserialize)]
pub struct SessionActionsQuery {
    /// Filter by assignee status: "assigned", "unassigned", or omit for all
    #[serde(default)]
    pub filter: ActionApi::AssigneeFilter,
}

/// GET all actions from a user's coaching sessions with optional filtering
#[utoipa::path(
    get,
    path = "/users/{user_id}/session-actions",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve session actions for"),
        ("filter" = Option<String>, Query, description = "Filter: 'assigned' or 'unassigned' (default: all)"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved session actions for user", body = [domain::action::ActionWithAssignees]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "User not found"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
    Query(query): Query<SessionActionsQuery>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET Session Actions for User: {user_id} with filter={:?}",
        query.filter
    );

    let actions = ActionApi::find_by_user_sessions_with_assignees(
        app_state.db_conn_ref(),
        user_id,
        query.filter,
    )
    .await?;

    debug!("Found {} session actions for user {user_id}", actions.len());

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), actions)))
}
