use axum::{
    extract::Request,
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
        middleware::from_fn,
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
    async fn test_require_auth_returns_401_with_no_session() {
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
            .route_layer(from_fn(require_auth))
            .layer(auth_layer)
            .with_state(app_state);

        let request = Request::builder().uri("/test").body(Body::empty()).unwrap();
        let response: Response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_require_auth_returns_401_with_invalid_session_cookie() {
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
            .route_layer(from_fn(require_auth))
            .layer(auth_layer)
            .with_state(app_state);

        let request = Request::builder()
            .uri("/test")
            .header("cookie", "tower.sid=invalid-session-id")
            .body(Body::empty())
            .unwrap();
        let response: Response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_require_auth_allows_authenticated_request_to_proceed() {
        use chrono::Utc;
        use domain::{user_roles, users, Id};
        use password_auth::generate_hash;
        use sea_orm::{DatabaseBackend, MockDatabase};

        // Create a test user that matches the existing test pattern
        let test_user = users::Model {
            id: Id::new_v4(),
            email: "test@domain.com".to_string(),
            first_name: "test".to_string(),
            last_name: "login".to_string(),
            display_name: Some("test login".to_string()),
            password: generate_hash("password2"),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            created_at: Utc::now().into(),
            updated_at: Utc::now().into(),
            role: domain::users::Role::User,
            roles: vec![],
        };

        let config = Config::default();
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([[(test_user.clone(), None::<user_roles::Model>)]]) // For find_with_related in authentication
                .append_query_results([[test_user.clone()]]) // For get_user after login (simple find)
                .append_query_results([[test_user.clone()]]) // For session user lookup in protected route (simple find)
                .into_connection(),
        );
        let app_state = crate::AppState::new(config, &db);

        // Set up session layer
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        // Create app with login route and protected test route
        let app = Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route("/test", get(test_handler))
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state);

        // First, log in to create an authenticated session
        let login_request = Request::builder()
            .uri("/login")
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("email=test@domain.com&password=password2"))
            .unwrap();

        let login_response = app.clone().oneshot(login_request).await.unwrap();

        // Extract session cookie from login response
        let cookie = login_response
            .headers()
            .get("set-cookie")
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie");

        // Now make authenticated request to protected route
        let protected_request = Request::builder()
            .uri("/test")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response: Response = app.oneshot(protected_request).await.unwrap();

        // Should get 200 OK showing require_auth allowed the request through
        assert_eq!(response.status(), StatusCode::OK);
    }
}
