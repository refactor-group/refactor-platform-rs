use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{organization as OrganizationApi, Id};
use service::config::ApiVersion;

use log::*;

/// GET all organizations for a specific user
#[utoipa::path(
    get,
    path = "/users/{user_id}/organizations",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve organizations for")
    ),
    responses(
        (status = 200, description = "Successfully retrieved organizations for user", body = [domain::organizations::Model]),
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
    debug!("GET Organizations for User: {user_id}");

    let organizations = OrganizationApi::find_by_user(app_state.db_conn_ref(), user_id).await?;

    debug!("Found {} organizations for user {user_id}", organizations.len());

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), organizations)))
}
