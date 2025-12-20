//! Controller for coaching relationship operations.
//!
//! Handles operations on coaching relationships that are not nested under organizations,
//! such as updating meeting URLs and AI privacy levels.

use crate::controller::ApiResponse;
use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::params::coaching_relationship::UpdateParams;
use crate::{AppState, Error};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use domain::coaching_relationship as CoachingRelationshipApi;
use domain::Id;
use log::*;
use service::config::ApiVersion;

/// UPDATE a CoachingRelationship.
///
/// Updates the meeting URL and/or AI privacy level for a coaching relationship.
/// Only the coach can update these settings.
#[utoipa::path(
    put,
    path = "/coaching_relationships/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching relationship ID"),
    ),
    request_body = UpdateParams,
    responses(
        (status = 200, description = "Successfully updated the coaching relationship", body = CoachingRelationshipWithUserNames),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching relationship not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
    Json(params): Json<UpdateParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("UPDATE CoachingRelationship {id} with params: {params:?}");

    // First, verify the relationship exists and user is the coach
    let relationship = CoachingRelationshipApi::find_by_id(app_state.db_conn_ref(), id).await?;

    if relationship.coach_id != user.id {
        warn!(
            "User {} attempted to update coaching relationship {} but is not the coach",
            user.id, id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(
                    domain::error::EntityErrorKind::Unauthenticated,
                ),
            ),
        }));
    }

    let updated = CoachingRelationshipApi::update(
        app_state.db_conn_ref(),
        id,
        params.meeting_url,
        params.ai_privacy_level,
    )
    .await?;

    debug!("Updated CoachingRelationship: {updated:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
}
