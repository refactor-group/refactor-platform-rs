use crate::{controller::ApiResponse, extractors::compare_api_version::CompareApiVersion};
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
