use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{controller::ApiResponse, params::user::UpdatePasswordParams};
use crate::{AppState, Error};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::{user as UserApi, Id};
use service::config::ApiVersion;

/// update a user's password
#[utoipa::path(
    put,
    path = "/users/{user_id}/password",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID", example = "1234567890"),
    ),
    responses(
        (status = 200, description = "Successfully updated a User's password"),
        (status = 401, description = "Unauthorized"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update_password(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
    Json(params): Json<UpdatePasswordParams>,
) -> Result<impl IntoResponse, Error> {
    UserApi::update_password(app_state.db_conn_ref(), user_id, params).await?;
    Ok(Json(ApiResponse::new(StatusCode::NO_CONTENT.into(), ())))
}
