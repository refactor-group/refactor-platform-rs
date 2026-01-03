use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{coaching_relationship as CoachingRelationshipApi, Id};
use service::config::ApiVersion;

use log::*;

/// GET roles summary for a specific user across all coaching relationships
#[utoipa::path(
    get,
    path = "/users/{user_id}/relationship-roles-summary",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve roles summary for"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved roles summary for user", body = domain::coaching_relationship::RolesSummary),
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
    debug!("GET Relationship Roles Summary for User: {user_id}");

    let summary =
        CoachingRelationshipApi::get_roles_summary(app_state.db_conn_ref(), user_id).await?;

    debug!(
        "User {user_id} is_coach={}, is_coachee={}, coach_count={}, coachee_count={}",
        summary.is_coach,
        summary.is_coachee,
        summary.coach_relationship_count,
        summary.coachee_relationship_count
    );

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), summary)))
}
