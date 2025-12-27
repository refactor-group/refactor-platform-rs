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
use domain::gateway::google_oauth::{GoogleMeetClient, GoogleOAuthClient, GoogleOAuthUrls};
use domain::user_integration;
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

/// CREATE a Google Meet for a CoachingRelationship.
///
/// Creates a new Google Meet space using the coach's Google account and
/// sets it as the meeting URL for this coaching relationship.
///
/// Requires the coach to have connected their Google account via OAuth.
#[utoipa::path(
    post,
    path = "/coaching_relationships/{id}/create-google-meet",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching relationship ID"),
    ),
    responses(
        (status = 200, description = "Successfully created Google Meet", body = CoachingRelationshipWithUserNames),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not the coach or Google not connected"),
        (status = 404, description = "Coaching relationship not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create_google_meet(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("CREATE Google Meet for CoachingRelationship {id}");

    // Verify the relationship exists
    let relationship = CoachingRelationshipApi::find_by_id(app_state.db_conn_ref(), id).await?;

    // Only the coach can create a meeting URL
    if relationship.coach_id != user.id {
        warn!(
            "User {} attempted to create Google Meet for relationship {} but is not the coach",
            user.id, id
        );
        return Err(forbidden_error(
            "Only the coach can create a Google Meet for this relationship",
        ));
    }

    // Get the user's Google integration
    let integration = user_integration::get_or_create(app_state.db_conn_ref(), user.id).await?;

    // Check if Google is connected
    let access_token = match &integration.google_access_token {
        Some(token) => token.clone(),
        None => {
            warn!(
                "User {} attempted to create Google Meet but Google is not connected",
                user.id
            );
            return Err(forbidden_error(
                "Please connect your Google account first in Settings > Integrations",
            ));
        }
    };

    let config = &app_state.config;

    // Check if token is expired and refresh if needed
    let access_token = if let Some(expiry) = &integration.google_token_expiry {
        if expiry < &chrono::Utc::now() {
            // Token is expired, try to refresh
            let refresh_token = integration.google_refresh_token.as_ref().ok_or_else(|| {
                warn!(
                    "Google token expired and no refresh token available for user {}",
                    user.id
                );
                forbidden_error(
                    "Google authorization expired. Please reconnect your Google account.",
                )
            })?;

            let client_id = config
                .google_client_id()
                .ok_or_else(|| internal_error("Google OAuth not configured"))?;
            let client_secret = config
                .google_client_secret()
                .ok_or_else(|| internal_error("Google OAuth not configured"))?;
            let redirect_uri = config
                .google_redirect_uri()
                .ok_or_else(|| internal_error("Google OAuth not configured"))?;

            let urls = GoogleOAuthUrls {
                auth_url: config.google_oauth_auth_url().to_string(),
                token_url: config.google_oauth_token_url().to_string(),
                userinfo_url: config.google_userinfo_url().to_string(),
            };

            let oauth_client =
                GoogleOAuthClient::new(&client_id, &client_secret, &redirect_uri, urls)?;
            let token_response = oauth_client.refresh_token(refresh_token).await?;

            // Update stored tokens
            let mut updated_integration = integration.clone();
            updated_integration.google_access_token = Some(token_response.access_token.clone());
            updated_integration.google_token_expiry =
                Some(chrono::Utc::now() + chrono::Duration::seconds(token_response.expires_in))
                    .map(|dt| dt.into());

            user_integration::update(
                app_state.db_conn_ref(),
                updated_integration.id,
                updated_integration,
            )
            .await?;

            token_response.access_token
        } else {
            access_token
        }
    } else {
        access_token
    };

    // Create the Google Meet space
    let meet_client = GoogleMeetClient::new(&access_token, config.google_meet_api_url())?;
    let space = meet_client.create_space().await?;

    info!(
        "Created Google Meet {} for coaching relationship {}",
        space.meeting_code, id
    );

    // Update the coaching relationship with the new meeting URL
    let updated = CoachingRelationshipApi::update(
        app_state.db_conn_ref(),
        id,
        Some(space.meeting_uri),
        None,
        None,
    )
    .await?;

    debug!("Updated CoachingRelationship with Google Meet: {updated:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
}

/// Helper to create an internal server error
fn internal_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Other(message.to_string()),
        ),
    })
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
