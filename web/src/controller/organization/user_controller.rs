use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{controller::ApiResponse, AppState, Error};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::{emails as EmailsAPI, user as UserApi, users, Id};
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
    Path(organization_id): Path<Id>,
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
    Path(organization_id): Path<Id>,
    Json(user_model): Json<users::Model>,
) -> Result<impl IntoResponse, Error> {
    let user =
        UserApi::create_by_organization(app_state.db_conn_ref(), organization_id, user_model)
            .await?;
    info!("User created: {user:?}");

    // Best-effort welcome email — log failures, don't block user creation
    if let Err(e) = EmailsAPI::notify_welcome_email(&app_state.config, &user).await {
        warn!(
            "Failed to send welcome email for user {}: {e:?}",
            user.email
        );
    }

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), user)))
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
    AuthenticatedUser(_authenticated_user): AuthenticatedUser,
    Path((_organization_id, user_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    info!("Deleting user: {user_id:?}");
    UserApi::delete(app_state.db_conn_ref(), user_id).await?;
    Ok(Json(ApiResponse::<()>::no_content(
        StatusCode::NO_CONTENT.into(),
    )))
}
