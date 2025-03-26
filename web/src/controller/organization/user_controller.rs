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
use domain::{user as UserApi, users, Id};
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
    Path(organization_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("INDEX all Users for Organization {:?}", organization_id);

    let users = UserApi::find_by_organization(app_state.db_conn_ref(), organization_id).await?;

    debug!("Found Users {:?}", &users);

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
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub(crate) async fn create(
    State(app_state): State<AppState>,
    AuthenticatedUser(_authenticated_user): AuthenticatedUser,
    Path(organization_id): Path<Id>,
    Json(user_model): Json<users::Model>,
) -> Result<impl IntoResponse, Error> {
    let user =
        UserApi::create_by_organization(app_state.db_conn_ref(), organization_id, user_model)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), user)))
}
