use axum::http::StatusCode;
use axum::response::IntoResponse;

/// GET liveness probe for the API server
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "API router is up and responding to requests", body = String),
        (status = 500, description = "Internal Server Error")
    )
)]
pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "healthy")
}
