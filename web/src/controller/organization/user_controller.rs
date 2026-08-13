use crate::error::WebErrorKind;
use crate::extractors::organization_admin_access::OrganizationAdminAccess;
use crate::extractors::organization_member_access::OrganizationMemberAccess;
use crate::extractors::organization_user_access::OrganizationUserAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::user::{AttachRoleParams, CreateMemberParams};
use crate::{controller::ApiResponse, AppState, Error};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::error::{DomainErrorKind, Error as DomainError};
use domain::users::Role;
use domain::{emails as EmailsAPI, user as UserApi, user_role as UserRoleApi, Id};
use service::config::ApiVersion;

use log::*;

/// INDEX all Users
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/users",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "The ID of the organization to retrieve users for")
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
        ("organization_id" = Uuid, Path, description = "The ID of the organization"),
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
        organization.id,
        params.user,
        params.coach_id,
    )
    .await?;
    info!("User created: {user:?}");

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
        ("organization_id" = Uuid, Path, description = "The ID of the organization"),
        ("user_id" = Uuid, Path, description = "The ID of the user to resend the invite to"),
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

/// ATTACH an existing User to an organization with a role.
///
/// Grants the user a role in the organization without creating an account,
/// optionally assigning them a coach in the same transaction. The caller must
/// administer the organization and must already be able to see the target user,
/// otherwise the response is indistinguishable from a missing user.
///
/// A coaching relationship surviving an earlier removal is reused, so re-adding a
/// member with their original coach restores their history rather than conflicting.
#[utoipa::path(
    post,
    path = "/organizations/{organization_id}/users/{user_id}/role",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "The ID of the organization"),
        ("user_id" = Uuid, Path, description = "The ID of the user to attach"),
    ),
    request_body = AttachRoleParams,
    responses(
        (status = 201, description = "User attached successfully", body = domain::users::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller does not administer the organization"),
        (status = 404, description = "No such user, or a user the caller may not see"),
        (status = 409, description = "User already belongs to the organization"),
        (status = 422, description = "SuperAdmin cannot be granted within an organization, or the requested coach is not a member of it"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub(crate) async fn attach_role(
    CompareApiVersion(_v): CompareApiVersion,
    OrganizationAdminAccess {
        organization,
        authenticated_user,
    }: OrganizationAdminAccess,
    Path((_organization_id, user_id)): Path<(Id, Id)>,
    State(app_state): State<AppState>,
    Json(params): Json<AttachRoleParams>,
) -> Result<impl IntoResponse, Error> {
    if params.role == Role::SuperAdmin {
        return Err(DomainError {
            source: None,
            error_kind: DomainErrorKind::Validation(
                "SuperAdmin cannot be granted within an organization.".into(),
            ),
        }
        .into());
    }

    // 404 rather than 403: an invisible user must look exactly like a missing one.
    if !UserRoleApi::can_administer_user(app_state.db_conn_ref(), &authenticated_user, user_id)
        .await?
    {
        return Err(Error::Web(WebErrorKind::NotFound));
    }

    let user = UserRoleApi::attach_to_organization(
        app_state.db_conn_ref(),
        organization.id,
        user_id,
        params.role.clone(),
        params.coach_id,
    )
    .await?;
    info!(
        "User {user_id} attached to organization {}",
        organization.id
    );

    // Must stay after the commit: a failed coach assignment notifies nobody.
    EmailsAPI::notify_added_to_organization(
        &app_state.config,
        &user,
        &authenticated_user,
        &organization,
        params.role,
    )
    .await;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), user)))
}

/// REMOVE a User from an organization, leaving their account intact.
///
/// Deletes the user's role in this organization only. Their other memberships
/// and their account are untouched.
#[utoipa::path(
    delete,
    path = "/organizations/{organization_id}/users/{user_id}/role",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "The ID of the organization"),
        ("user_id" = Uuid, Path, description = "The ID of the user to remove"),
    ),
    responses(
        (status = 204, description = "User removed from the organization"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller does not administer the organization, or targeted themselves"),
        (status = 404, description = "User holds no role in the organization"),
        (status = 409, description = "User is the only admin of the organization"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub(crate) async fn remove_role(
    CompareApiVersion(_v): CompareApiVersion,
    OrganizationAdminAccess {
        organization,
        authenticated_user,
    }: OrganizationAdminAccess,
    Path((_organization_id, user_id)): Path<(Id, Id)>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    if user_id == authenticated_user.id {
        return Err(Error::Web(WebErrorKind::Forbidden));
    }

    UserRoleApi::remove_from_organization(app_state.db_conn_ref(), organization.id, user_id)
        .await?;
    info!(
        "User {user_id} removed from organization {}",
        organization.id
    );

    Ok(Json(ApiResponse::<()>::no_content(
        StatusCode::NO_CONTENT.into(),
    )))
}

/// DELETE a User for an organization
#[utoipa::path(
    delete,
    path = "/organizations/{organization_id}/users/{user_id}",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "The ID of the organization"),
        ("user_id" = Uuid, Path, description = "The ID of the user to delete")
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
    UserApi::delete(app_state.db_conn_ref(), user.id).await?;
    Ok(Json(ApiResponse::<()>::no_content(
        StatusCode::NO_CONTENT.into(),
    )))
}
