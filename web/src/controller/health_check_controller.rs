use axum::http::StatusCode;
use axum::response::IntoResponse;

/// GET generate a collaboration token
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "API router is up and responding to requests", body = String),  
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "healthy")
}
