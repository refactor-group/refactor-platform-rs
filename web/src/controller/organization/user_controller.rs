use crate::extractors::organization_member_access::OrganizationMemberAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{controller::ApiResponse, AppState, Error};
use axum::extract::Path;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use domain::Id;
use domain::{emails as EmailsAPI, user as UserApi, users};
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
    let users = UserApi::find_by_organization(app_state.db_conn_ref(), organization_id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), users)))
}

/// CREATE a User for an organization
/// This function creates a new user associated with the specified organization.
#[utoipa::path(
    post,
    path = "/organizations/{organization_id}/users",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "The ID of the organization"),
    ),
    request_body = domain::users::Model,
    responses(
        (status = 201, description = "User created successfully", body = domain::users::Model),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub(crate) async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    AuthenticatedUser(_authenticated_user): AuthenticatedUser,
    OrganizationMemberAccess(organization_id): OrganizationMemberAccess,
    Json(user_model): Json<users::Model>,
) -> Result<impl IntoResponse, Error> {
    let user =
        UserApi::create_by_organization(app_state.db_conn_ref(), organization_id, user_model)
            .await?;
    info!("User created: {user:?}");

    EmailsAPI::notify_welcome_email(app_state.db_conn_ref(), &app_state.config, &user).await;

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
    AuthenticatedUser(_authenticated_user): AuthenticatedUser,
    OrganizationMemberAccess(_organization_id): OrganizationMemberAccess,
    Path((_org_id, user_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    let user = UserApi::find_by_id(app_state.db_conn_ref(), user_id).await?;

    if user.password.is_some() {
        return Err(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Validation(
                "User has already completed setup".into(),
            ),
        }
        .into());
    }

    EmailsAPI::create_and_send_welcome_email(app_state.db_conn_ref(), &app_state.config, &user)
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
    OrganizationMemberAccess(_organization_id): OrganizationMemberAccess,
    Path((_organization_id_1, user_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    info!("Deleting user: {user_id:?}");
    UserApi::delete(app_state.db_conn_ref(), user_id).await?;
    Ok(Json(ApiResponse::<()>::no_content(
        StatusCode::NO_CONTENT.into(),
    )))
}
