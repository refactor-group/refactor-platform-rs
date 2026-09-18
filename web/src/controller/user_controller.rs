use crate::error::WebErrorKind;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{controller::ApiResponse, params::user::*};
use crate::{AppState, Error};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::users::Role;
use domain::{user as UserApi, user_lookup, user_role as UserRoleApi, Id};
use service::config::ApiVersion;

/// INDEX the Users matching an exact email address.
///
/// Returns an array of zero or one results. An empty array is the only
/// not-found signal, so a caller cannot distinguish an unknown email from one
/// belonging to a user they are not allowed to see.
#[utoipa::path(
    get,
    path = "/users",
    params(
        ApiVersion,
        LookupParams,
    ),
    responses(
        (status = 200, description = "Successfully retrieved the matching Users", body = [domain::user_role::UserLookupResult]),
        (status = 400, description = "Missing or blank email"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Requester administers no organization"),
        (status = 429, description = "Too many lookups from this requester"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Query(params): Query<LookupParams>,
) -> Result<impl IntoResponse, Error> {
    if params.email.trim().is_empty() {
        return Err(Error::Web(WebErrorKind::Input));
    }

    let administers_any_organization = authenticated_user.roles.iter().any(|role| {
        (role.role == Role::Admin && role.organization_id.is_some())
            || (role.role == Role::SuperAdmin && role.organization_id.is_none())
    });

    if !administers_any_organization {
        return Err(Error::Web(WebErrorKind::Forbidden));
    }

    // After authorization so nobody can fill another requester's allowance, and
    // before the lookup so a refusal does no work.
    user_lookup::record_attempt_or_reject(app_state.db_conn_ref(), authenticated_user.id).await?;

    let users = UserRoleApi::lookup_by_email_scoped(
        app_state.db_conn_ref(),
        &authenticated_user,
        &params.email,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), users)))
}

/// GET a User
///
#[utoipa::path(
    get,
    path = "/users/{user_id}",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID", example = "1234567890"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved a User", body = domain::users::Model),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    let user = UserApi::find_by_id(app_state.db_conn_ref(), user_id).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), user)))
}

/// UPDATE a User
/// NOTE: that this is for updating the current user
#[utoipa::path(
    put,
    path = "/users",
    params(
        ApiVersion
    ),
    request_body = UpdateParams,
    responses(
        (status = 204, description = "Successfully updated a User", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
    Json(params): Json<UpdateParams>,
) -> Result<impl IntoResponse, Error> {
    UserApi::update(app_state.db_conn_ref(), user_id, params).await?;
    Ok(Json(ApiResponse::new(StatusCode::NO_CONTENT.into(), ())))
}
