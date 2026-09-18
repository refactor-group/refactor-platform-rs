//! Reading and changing the role a user holds within one organization.
//!
//! All four verbs address the same path, `.../users/{user_id}/role`, and share the
//! `OrganizationAdminAccess` gate. `POST` grants a role to a user who holds none
//! here, `PUT` changes one in place, `DELETE` removes the membership, and `GET`
//! reports it.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::error::{DomainErrorKind, Error as DomainError};
use domain::users::Role;
use domain::Actor;
use domain::{emails as EmailsAPI, user_role as UserRoleApi, Id};
use log::*;
use service::config::ApiVersion;

use crate::error::WebErrorKind;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::extractors::organization_admin_access::OrganizationAdminAccess;
use crate::params::user::{AttachRoleParams, UpdateRoleParams};
use crate::{controller::ApiResponse, AppState, Error};

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
        ("organization_id" = Id, Path, description = "The ID of the organization"),
        ("user_id" = Id, Path, description = "The ID of the user to attach"),
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
        // The generic 422 log carries no actor or target, and no first-party client
        // sends this role, so an attempt is worth attributing on its own.
        warn!(
            "role_change_denied actor={} target={} org={} attempted=super_admin \
             reason=super_admin_not_grantable_in_organization",
            authenticated_user.id, user_id, organization.id
        );
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
        Actor::new(authenticated_user.id),
        organization.id,
        user_id,
        params.role.clone(),
        params.coach_id,
    )
    .await?;
    info!(
        "role_change actor={} target={} org={} previous=none new={}",
        authenticated_user.id, user_id, organization.id, params.role
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

/// READ the role a User holds in an organization.
///
/// Returns the membership row scoped to this organization. Roles the user holds
/// elsewhere, and any global role, are not reported here.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/users/{user_id}/role",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "The ID of the organization"),
        ("user_id" = Id, Path, description = "The ID of the user whose role to read"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved the user's role", body = domain::user_roles::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller does not administer the organization"),
        (status = 404, description = "User holds no role in the organization"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub(crate) async fn read_role(
    CompareApiVersion(_v): CompareApiVersion,
    OrganizationAdminAccess { organization, .. }: OrganizationAdminAccess,
    Path((_organization_id, user_id)): Path<(Id, Id)>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let membership =
        UserRoleApi::find_role_in_organization(app_state.db_conn_ref(), organization.id, user_id)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), membership)))
}

/// UPDATE the role a User holds in an organization.
///
/// Changes the role in place, atomically, leaving the user's coaching
/// relationships and sessions in the organization untouched.
#[utoipa::path(
    put,
    path = "/organizations/{organization_id}/users/{user_id}/role",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "The ID of the organization"),
        ("user_id" = Id, Path, description = "The ID of the user whose role to change"),
    ),
    request_body = UpdateRoleParams,
    responses(
        (status = 200, description = "Role updated successfully", body = domain::users::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller does not administer the organization, or targeted themselves"),
        (status = 404, description = "User holds no role in the organization"),
        (status = 409, description = "User is the only admin of the organization, or the organization is archived"),
        (status = 422, description = "SuperAdmin cannot be granted within an organization"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub(crate) async fn update_role(
    CompareApiVersion(_v): CompareApiVersion,
    OrganizationAdminAccess {
        organization,
        authenticated_user,
    }: OrganizationAdminAccess,
    Path((_organization_id, user_id)): Path<(Id, Id)>,
    State(app_state): State<AppState>,
    Json(params): Json<UpdateRoleParams>,
) -> Result<impl IntoResponse, Error> {
    if params.role == Role::SuperAdmin {
        // The generic 422 log carries no actor or target, and no first-party client
        // sends this role, so an attempt is worth attributing on its own.
        warn!(
            "role_change_denied actor={} target={} org={} attempted=super_admin \
             reason=super_admin_not_grantable_in_organization",
            authenticated_user.id, user_id, organization.id
        );
        return Err(DomainError {
            source: None,
            error_kind: DomainErrorKind::Validation(
                "SuperAdmin cannot be granted within an organization.".into(),
            ),
        }
        .into());
    }

    if user_id == authenticated_user.id {
        return Err(Error::Web(WebErrorKind::Forbidden));
    }

    let (previous_role, user) = UserRoleApi::update_role_in_organization(
        app_state.db_conn_ref(),
        Actor::new(authenticated_user.id),
        organization.id,
        user_id,
        params.role.clone(),
    )
    .await?;
    // Silent on a no-op: nothing was written, and the audit table has no row to
    // match a line claiming otherwise.
    if previous_role != params.role {
        info!(
            "role_change actor={} target={} org={} previous={previous_role} new={}",
            authenticated_user.id, user_id, organization.id, params.role
        );
    }

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), user)))
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
        ("organization_id" = Id, Path, description = "The ID of the organization"),
        ("user_id" = Id, Path, description = "The ID of the user to remove"),
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

    let previous_role = UserRoleApi::remove_from_organization(
        app_state.db_conn_ref(),
        Actor::new(authenticated_user.id),
        organization.id,
        user_id,
    )
    .await?;
    info!(
        "role_change actor={} target={} org={} previous={previous_role} new=none",
        authenticated_user.id, user_id, organization.id
    );

    Ok(Json(ApiResponse::<()>::no_content(
        StatusCode::NO_CONTENT.into(),
    )))
}
