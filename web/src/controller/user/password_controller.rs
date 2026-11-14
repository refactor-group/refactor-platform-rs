use crate::extractors::{compare_api_version::CompareApiVersion, user_self_access::UserSelfAccess};
use crate::{controller::ApiResponse, params::user::UpdatePasswordParams};
use crate::{AppState, Error};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use domain::user as UserApi;
use service::config::ApiVersion;

/// update a user's password
#[utoipa::path(
    put,
    path = "/users/{user_id}/password",
    params(
        ApiVersion,
        ("user_id" = Uuid, Path, description = "User ID", example = "1234567890"),
    ),
    request_body = UpdatePasswordParams,
    responses(
        (status = 200, description = "Successfully updated a User's password"),
        (status = 400, description = "Invalid user_id"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update_password(
    UserSelfAccess(user): UserSelfAccess,
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Json(params): Json<UpdatePasswordParams>,
) -> Result<impl IntoResponse, Error> {
    let user_id = user.id;
    UserApi::update_password(app_state.db_conn_ref(), user_id, params).await?;
    Ok(Json(ApiResponse::new(StatusCode::NO_CONTENT.into(), ())))
}

#[cfg(test)]
#[path = "password_controller_sqlite_tests.rs"]
mod sqlite_tests;
