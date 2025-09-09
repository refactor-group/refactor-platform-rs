use crate::AppState;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_login::AuthSession;

/// Authentication middleware that returns 401 Unauthorized for unauthenticated requests.
///
/// This replaces axum-login's `login_required!` macro which redirects to login URLs.
/// For API endpoints, we want to return proper HTTP status codes instead of redirects.
pub async fn require_auth(
    State(_app_state): State<AppState>,
    auth_session: AuthSession<domain::user::Backend>,
    request: Request,
    next: Next,
) -> Response {
    match auth_session.user {
        Some(_user) => {
            // User is authenticated, continue to the handler
            next.run(request).await
        }
        None => {
            // User is not authenticated or session expired
            (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
        }
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware::from_fn_with_state,
        response::Response,
        routing::get,
        Router,
    };
    use axum_login::{
        tower_sessions::{Expiry, MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use domain::user::Backend;
    use service::config::Config;
    use std::sync::Arc;
    use time::Duration;
    use tower::ServiceExt;

    async fn test_handler() -> &'static str {
        "authenticated"
    }

    #[tokio::test]
    async fn test_require_auth_with_no_session_returns_401() {
        let config = Config::default();
        let db = Arc::new(
            sea_orm::MockDatabase::new(sea_orm::DatabaseBackend::Postgres).into_connection(),
        );
        let app_state = crate::AppState::new(config, &db);

        // Set up session layer
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)));

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        let app = Router::new()
            .route("/test", get(test_handler))
            .route_layer(from_fn_with_state(app_state.clone(), require_auth))
            .layer(auth_layer)
            .with_state(app_state);

        let request = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let response: Response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
