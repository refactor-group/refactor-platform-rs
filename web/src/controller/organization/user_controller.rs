use crate::error::WebErrorKind;
use crate::extractors::organization_admin_access::OrganizationAdminAccess;
use crate::extractors::organization_member_access::OrganizationMemberAccess;
use crate::extractors::organization_user_access::OrganizationUserAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::user::CreateMemberParams;
use crate::{controller::ApiResponse, AppState, Error};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use domain::users::Role;
use domain::Actor;
use domain::{emails as EmailsAPI, user as UserApi};
use service::config::ApiVersion;

use log::*;

/// INDEX all Users
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/users",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "The ID of the organization to retrieve users for")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Users", body = [domain::users::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    OrganizationMemberAccess(organization_id): OrganizationMemberAccess,
) -> Result<impl IntoResponse, Error> {
    let users =
        UserApi::find_by_organization_with_invite_status(app_state.db_conn_ref(), organization_id)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), users)))
}

/// CREATE a User for an organization
///
/// Creates a new user associated with the specified organization, optionally
/// assigning them a coach in the same transaction.
#[utoipa::path(
    post,
    path = "/organizations/{organization_id}/users",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "The ID of the organization"),
    ),
    request_body = CreateMemberParams,
    responses(
        (status = 201, description = "User created successfully", body = domain::users::Model),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 422, description = "The requested coach cannot be assigned"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub(crate) async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    OrganizationAdminAccess {
        organization,
        authenticated_user,
    }: OrganizationAdminAccess,
    Json(params): Json<CreateMemberParams>,
) -> Result<impl IntoResponse, Error> {
    let user = UserApi::create_in_organization(
        app_state.db_conn_ref(),
        Actor::new(authenticated_user.id),
        organization.id,
        params.user,
        params.coach_id,
    )
    .await?;
    // Id only. The Debug form of the model carries the hashed password.
    info!("User created: {}", user.id);
    info!(
        "role_change actor={} target={} org={} previous=none new={}",
        authenticated_user.id,
        user.id,
        organization.id,
        Role::User
    );

    // Must stay after the commit: a failed coach assignment invites nobody.
    EmailsAPI::notify_welcome_email(
        app_state.db_conn_ref(),
        &app_state.config,
        &user,
        &authenticated_user,
    )
    .await;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), user)))
}

/// Resend an invite email to a user who has not yet completed account setup.
///
/// Generates a new magic link token (invalidating any previous one) and
/// sends the welcome email again.
#[utoipa::path(
    post,
    path = "/organizations/{organization_id}/users/{user_id}/resend-invite",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "The ID of the organization"),
        ("user_id" = Id, Path, description = "The ID of the user to resend the invite to"),
    ),
    responses(
        (status = 200, description = "Invite resent successfully", body = domain::users::Model),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "User has already completed setup"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub(crate) async fn resend_invite(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    OrganizationAdminAccess {
        authenticated_user, ..
    }: OrganizationAdminAccess,
    OrganizationUserAccess(user): OrganizationUserAccess,
) -> Result<impl IntoResponse, Error> {
    if user.password.is_some() {
        return Err(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Validation(
                "User has already completed setup".into(),
            ),
        }
        .into());
    }

    EmailsAPI::create_and_send_welcome_email(
        app_state.db_conn_ref(),
        &app_state.config,
        &user,
        &authenticated_user,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), user)))
}

/// DELETE a User for an organization
#[utoipa::path(
    delete,
    path = "/organizations/{organization_id}/users/{user_id}",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "The ID of the organization"),
        ("user_id" = Id, Path, description = "The ID of the user to delete")
    ),
    responses(
        (status = 200, description = "User deleted successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    OrganizationAdminAccess {
        authenticated_user, ..
    }: OrganizationAdminAccess,
    OrganizationUserAccess(user): OrganizationUserAccess,
) -> Result<impl IntoResponse, Error> {
    if user.id == authenticated_user.id {
        return Err(Error::Web(WebErrorKind::Forbidden));
    }

    info!("Deleting user: {:?}", user.id);
    let destroyed_roles = UserApi::delete(
        app_state.db_conn_ref(),
        Actor::new(authenticated_user.id),
        user.id,
    )
    .await?;

    // Account deletion drops every role the user held, a global SuperAdmin grant
    // included, so each is reported on the same channel as a scoped role change.
    for destroyed in &destroyed_roles {
        let scope = destroyed
            .organization_id
            .map_or_else(|| "global".to_string(), |id| id.to_string());
        info!(
            "role_change actor={} target={} org={scope} previous={} new=none reason=account_deleted",
            authenticated_user.id, user.id, destroyed.role
        );
    }

    Ok(Json(ApiResponse::<()>::no_content(
        StatusCode::NO_CONTENT.into(),
    )))
}
