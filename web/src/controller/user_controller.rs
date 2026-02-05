use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{controller::ApiResponse, params::user::*};
use crate::{AppState, Error};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::{user as UserApi, Id};
use service::config::ApiVersion;

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
        (status = 200, description = "Successfully retrieved a User", body = User),
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
