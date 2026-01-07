use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::user::coaching_relationship::{IndexParams, RoleFilter};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{coaching_relationship as CoachingRelationshipApi, Id};
use service::config::ApiVersion;

use log::*;

/// GET all coaching relationships for a user with optional role filtering
#[utoipa::path(
    get,
    path = "/users/{user_id}/coaching-relationships",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve coaching relationships for"),
        ("role" = Option<String>, Query, description = "Filter by role: all, coach, or coachee (default: all)"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved coaching relationships for user", body = [domain::coaching_relationship::CoachingRelationshipWithUserNames]),
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
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Coaching Relationships for User: {user_id}");

    let params = params.with_user_id(user_id);

    // Map web layer RoleFilter to domain layer RoleFilter
    let role_filter = match params.role {
        RoleFilter::All => CoachingRelationshipApi::RoleFilter::All,
        RoleFilter::Coach => CoachingRelationshipApi::RoleFilter::Coach,
        RoleFilter::Coachee => CoachingRelationshipApi::RoleFilter::Coachee,
    };

    let relationships = CoachingRelationshipApi::find_by_user_id_with_user_names(
        app_state.db_conn_ref(),
        user_id,
        role_filter,
    )
    .await?;

    debug!(
        "Found {} coaching relationships for user {user_id} (role filter: {:?})",
        relationships.len(),
        params.role
    );

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), relationships)))
}
