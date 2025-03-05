use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{controller::ApiResponse, params::user::*};
use crate::{AppState, Error};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use domain::{user as UserApi, users};
use service::config::ApiVersion;

use log::*;

/// CREATE a new User
#[utoipa::path(
    post,
    path = "/users",
    params(
        ApiVersion,
    ),
    request_body = users::Model,
    responses(
        (status = 200, description = "Successfully created a new User", body = [users::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
    )]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Json(user_model): Json<users::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("CREATE new User from: {:?}", user_model);

    let user: users::Model = UserApi::create(app_state.db_conn_ref(), user_model).await?;

    debug!("Newly created Users {:?}", &user);

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), user)))
}

/// UPDATE a User
/// NOTE: that this is for updating the current user and as such uses the user
/// from the AuthenticatedUser extractor. If we decide to allow a user to update
/// another user, we may want to consider something like a PUT /myself endpoint for
/// the current user updating their own data.
#[utoipa::path(
    put,
    path = "/users",
    params(
        ApiVersion,
        UpdateParams
    ),
    request_body = UpdateParams,
    responses(
        (status = 204, description = "Successfully updated a User", body = ()),
        (status = 401, description = "Unauthorized"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(params): Json<UpdateParams>,
) -> Result<impl IntoResponse, Error> {
    UserApi::update(app_state.db_conn_ref(), user.id, params).await?;
    Ok(Json(ApiResponse::new(StatusCode::NO_CONTENT.into(), ())))
}
