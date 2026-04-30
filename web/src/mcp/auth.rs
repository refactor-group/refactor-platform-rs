use crate::AppState;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use domain::personal_access_token;
use log::*;

/// Extract the bearer token from the Authorization header.
fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|auth_header| {
            auth_header
                .strip_prefix("Bearer ")
                .map(|stripped| stripped.to_string())
        })
}

/// Axum middleware that validates a PAT bearer token for MCP endpoints.
///
/// Extracts `Authorization: Bearer <token>`, validates via domain layer,
/// and inserts the authenticated `users::Model` into request extensions.
/// Returns 401 if the token is missing, invalid, or inactive.
pub(crate) async fn require_pat_auth(
    State(app_state): State<AppState>,
    headers: HeaderMap,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let raw_token = match extract_bearer_token(&headers) {
        Some(token) => token,
        None => {
            warn!("MCP request missing Authorization header");
            return (
                StatusCode::UNAUTHORIZED,
                "Unauthorized: missing bearer token",
            )
                .into_response();
        }
    };

    match personal_access_token::validate_token(app_state.db_conn_ref(), &raw_token).await {
        Ok((user, pat)) => {
            // Fire-and-forget: update last_used_at for observability.
            // Non-critical — don't fail the request if this errors.
            let db = app_state.database_connection.clone();
            let pat_id = pat.id;
            tokio::spawn(async move {
                if let Err(e) = personal_access_token::touch_last_used(&*db, pat_id).await {
                    warn!("Failed to update PAT last_used_at: {e}");
                }
            });

            // Insert the authenticated user into request extensions
            // so tool handlers can access it via RequestContext.extensions
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Err(_) => {
            warn!("MCP request with invalid PAT");
            (StatusCode::UNAUTHORIZED, "Unauthorized: invalid token").into_response()
        }
    }
}
