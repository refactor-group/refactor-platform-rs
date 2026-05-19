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
use domain::{personal_access_token as PatApi, Id};
use service::config::ApiVersion;

/// POST /users/:user_id/tokens
///
/// Create a new personal access token for the authenticated user.
/// If an active PAT already exists, it is deactivated and replaced.
/// The raw token value is returned once in the response and never stored.
#[utoipa::path(
    post,
    path = "/users/{user_id}/tokens",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID"),
    ),
    responses(
        (status = 201, description = "Successfully created a personal access token"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 500, description = "Internal server error"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    let (raw_token, pat) = PatApi::create_token(app_state.db_conn_ref(), user_id).await?;

    let response = serde_json::json!({
        "token": raw_token,
        "id": pat.id,
        "status": pat.status,
        "created_at": pat.created_at,
    });

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), response)))
}

/// GET /users/:user_id/tokens
///
/// Show the active personal access token metadata for the authenticated user.
/// The raw token value is never returned — only metadata (id, status, timestamps).
#[utoipa::path(
    get,
    path = "/users/{user_id}/tokens",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved active token metadata"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No active token exists"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    let pat = PatApi::find_active_by_user(app_state.db_conn_ref(), user_id).await?;

    match pat {
        Some(token) => Ok(Json(ApiResponse::new(StatusCode::OK.into(), token)).into_response()),
        None => Ok((StatusCode::NOT_FOUND, "No active token exists").into_response()),
    }
}

/// PUT /users/:user_id/tokens/:token_id/deactivate
///
/// Deactivate a personal access token. Idempotent — deactivating an already-inactive token succeeds.
#[utoipa::path(
    put,
    path = "/users/{user_id}/tokens/{token_id}/deactivate",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID"),
        ("token_id" = Id, Path, description = "Token ID"),
    ),
    responses(
        (status = 200, description = "Successfully deactivated the token"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 500, description = "Internal server error"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn deactivate(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path((_user_id, token_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    let pat = PatApi::deactivate_token(app_state.db_conn_ref(), token_id).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), pat)))
}
