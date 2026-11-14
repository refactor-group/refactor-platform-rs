use crate::controller::ApiResponse;
use crate::extractors::{compare_api_version::CompareApiVersion, user_self_access::UserSelfAccess};
use crate::{AppState, Error};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::organization as OrganizationApi;
use service::config::ApiVersion;

use log::*;

/// GET all organizations for a specific user
#[utoipa::path(
    get,
    path = "/users/{user_id}/organizations",
    params(
        ApiVersion,
        ("user_id" = Uuid, Path, description = "User ID to retrieve organizations for")
    ),
    responses(
        (status = 200, description = "Successfully retrieved organizations for user", body = [domain::organizations::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "User not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    UserSelfAccess(user_id): UserSelfAccess,
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Organizations for User: {user_id}");

    let organizations = OrganizationApi::find_by_user(
        app_state.db_conn_ref(),
        user_id,
        OrganizationApi::StatusFilter::Active,
    )
    .await?;

    debug!(
        "Found {} organizations for user {user_id}",
        organizations.len()
    );

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), organizations)))
}

#[cfg(test)]
#[path = "organization_controller_sqlite_tests.rs"]
mod sqlite_tests;
