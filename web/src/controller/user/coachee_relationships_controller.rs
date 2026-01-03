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

/// GET all coaching relationships where the user is the coachee
#[utoipa::path(
    get,
    path = "/users/{user_id}/coachee-relationships",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve coachee relationships for"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved coachee relationships for user", body = [domain::coaching_relationship::CoachingRelationshipWithUserNames]),
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
    debug!("GET Coachee Relationships for User: {user_id}");

    let relationships = CoachingRelationshipApi::find_by_coachee_id_with_user_names(
        app_state.db_conn_ref(),
        user_id,
    )
    .await?;

    debug!(
        "Found {} coachee relationships for user {user_id}",
        relationships.len()
    );

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), relationships)))
}
