use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{action as ActionApi, Id};
use service::config::ApiVersion;

use log::*;

/// GET all actions assigned to a specific user across all coaching sessions
#[utoipa::path(
    get,
    path = "/users/{user_id}/assigned-actions",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve assigned actions for"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved assigned actions for user", body = [domain::action::ActionWithAssignees]),
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
) -> Result<impl IntoResponse, Error> {
    debug!("GET Assigned Actions for User: {user_id}");

    let actions =
        ActionApi::find_by_assignee_with_assignees(app_state.db_conn_ref(), user_id).await?;

    debug!(
        "Found {} assigned actions for user {user_id}",
        actions.len()
    );

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), actions)))
}
