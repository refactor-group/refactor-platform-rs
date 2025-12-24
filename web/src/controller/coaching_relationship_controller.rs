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
/// Updates the coaching relationship settings. Different fields are accessible
/// depending on the user's role:
/// - **Coach**: Can update `meeting_url` and `coach_ai_privacy_level`
/// - **Coachee**: Can only update `coachee_ai_privacy_level`
///
/// This enables mutual consent for AI features - both coach and coachee must
/// agree to the same privacy level for AI features to be available.
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
        (status = 400, description = "Bad request - invalid fields for user's role"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this relationship"),
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

    // First, verify the relationship exists
    let relationship = CoachingRelationshipApi::find_by_id(app_state.db_conn_ref(), id).await?;

    // Determine user's role in this relationship
    let is_coach = relationship.coach_id == user.id;
    let is_coachee = relationship.coachee_id == user.id;

    // User must be either coach or coachee
    if !is_coach && !is_coachee {
        warn!(
            "User {} attempted to update coaching relationship {} but is not a participant",
            user.id, id
        );
        return Err(forbidden_error(
            "Not authorized to update this relationship",
        ));
    }

    // Validate that users can only update their allowed fields
    if is_coachee {
        // Coachees can only update their own privacy level
        if params.meeting_url.is_some() || params.coach_ai_privacy_level.is_some() {
            warn!(
                "Coachee {} attempted to update coach-only fields on relationship {}",
                user.id, id
            );
            return Err(bad_request_error(
                "Coachees can only update their own AI privacy level",
            ));
        }
    }

    if is_coach {
        // Coaches cannot update coachee's privacy level
        if params.coachee_ai_privacy_level.is_some() {
            warn!(
                "Coach {} attempted to update coachee's privacy level on relationship {}",
                user.id, id
            );
            return Err(bad_request_error(
                "Coaches cannot update the coachee's AI privacy level",
            ));
        }
    }

    // Determine which fields to update based on role
    let meeting_url = if is_coach { params.meeting_url } else { None };
    let coach_privacy = if is_coach {
        params.coach_ai_privacy_level
    } else {
        None
    };
    let coachee_privacy = if is_coachee {
        params.coachee_ai_privacy_level
    } else {
        None
    };

    let updated = CoachingRelationshipApi::update(
        app_state.db_conn_ref(),
        id,
        meeting_url,
        coach_privacy,
        coachee_privacy,
    )
    .await?;

    debug!("Updated CoachingRelationship: {updated:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
}

/// Helper to create a forbidden error
fn forbidden_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                message.to_string(),
            )),
        ),
    })
}

/// Helper to create a bad request error
fn bad_request_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                message.to_string(),
            )),
        ),
    })
}
