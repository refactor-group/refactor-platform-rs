use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{controller::ApiResponse, params::user::*};
use crate::{AppState, Error};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use domain::user as UserApi;
use service::config::ApiVersion;

/// UPDATE a User
/// NOTE: that this is for updating the current user and as such uses the user
/// from the AuthenticatedUser extractor. If we decide to allow a user to update
/// another user, we may want to consider something like a PUT /myself endpoint for
/// the current user updating their own data.
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
