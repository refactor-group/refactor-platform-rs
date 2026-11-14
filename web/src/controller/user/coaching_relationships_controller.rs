use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
    user_self_access::UserSelfAccess,
};
use crate::params::user::coaching_relationship::IndexParams;
use crate::{AppState, Error};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::coaching_relationship as CoachingRelationshipApi;
use domain::search::visibility_for;
use service::config::ApiVersion;

use log::*;

/// GET all coaching relationships for a user with optional role filtering
#[utoipa::path(
    get,
    path = "/users/{user_id}/coaching-relationships",
    params(
        ApiVersion,
        ("user_id" = Uuid, Path, description = "User ID to retrieve coaching relationships for"),
        ("role" = Option<String>, Query, description = "Filter by role: all, coach, or coachee (default: all)"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved coaching relationships for user", body = [domain::coaching_relationship::CoachingRelationshipWithUserNames]),
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
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Coaching Relationships for User: {user_id}");

    let params = params.with_user_id(user_id);

    // Web layer's RoleFilter implements RoleFilterable trait, so pass directly
    let relationships = CoachingRelationshipApi::find_by_user_id_with_user_names(
        app_state.db_conn_ref(),
        &visibility_for(&user),
        params.role,
    )
    .await?;

    debug!(
        "Found {} coaching relationships for user {user_id}",
        relationships.len()
    );

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), relationships)))
}

#[cfg(test)]
#[path = "coaching_relationships_controller_sqlite_tests.rs"]
mod sqlite_tests;
