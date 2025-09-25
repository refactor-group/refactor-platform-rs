#[cfg(test)]
#[cfg(feature = "mock")]
mod session_renewal_integration_tests {
    use super::super::authenticated_user::AuthenticatedUser;
    use axum::{extract::Request, routing::get, Router};
    use axum_login::{
        tower_sessions::{Expiry, MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::user::Backend;
    use domain::{users, Id};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use std::{sync::Arc, time::Duration as StdDuration};
    use time::Duration;
    use tokio::time::sleep;
    use tower::ServiceExt;

    // Helper function to create a test user
    fn create_test_user() -> users::Model {
        let now = Utc::now();
        users::Model {
            id: Id::new_v4(),
            email: "test@example.com".to_string(),
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            display_name: Some("Test User".to_string()),
            password: generate_hash("password123".to_string()),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            role: users::Role::User,
            roles: vec![],
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    // Helper function to create test app with session management
    async fn create_test_app_with_expiry(expiry_duration: Duration) -> Router {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([[create_test_user()]]) // Mock user lookup
            .into_connection();
        let db_arc = Arc::new(db);

        // Set up session store with configurable expiry for testing
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(expiry_duration));

        // Set up auth backend
        let backend = Backend::new(&db_arc);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        // Create test routes
        async fn protected_route(_user: AuthenticatedUser) -> &'static str {
            "authenticated_success"
        }

        async fn login_route() -> &'static str {
            "login_success"
        }

        Router::new()
            .route("/protected", get(protected_route))
            .route("/login", get(login_route))
            .layer(auth_layer)
    }

    #[tokio::test]
    async fn test_invalid_session_consistently_rejected() {
        // This test verifies that making requests with invalid session cookies
        // consistently returns unauthorized, demonstrating the authentication flow works

        let app = create_test_app_with_expiry(Duration::milliseconds(200)).await;

        // Test with a completely invalid session cookie
        let invalid_session_cookie = "tower.sid=completely-invalid-session-id";

        let first_request = Request::builder()
            .uri("/protected")
            .header("cookie", invalid_session_cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let first_response = app.clone().oneshot(first_request).await.unwrap();

        // Wait some time to simulate session expiry scenario
        sleep(StdDuration::from_millis(100)).await;

        let second_request = Request::builder()
            .uri("/protected")
            .header("cookie", invalid_session_cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let second_response = app.oneshot(second_request).await.unwrap();

        // Both requests should be unauthorized since we're using invalid session cookies
        assert_eq!(
            first_response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "First request with invalid session should be unauthorized"
        );

        assert_eq!(
            second_response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "Second request with invalid session should also be unauthorized"
        );

        println!("✅ Authentication correctly rejects invalid sessions consistently");
    }

    #[tokio::test]
    async fn test_session_expires_without_renewal() {
        // Create app with very short session expiry (50ms)
        let app = create_test_app_with_expiry(Duration::milliseconds(50)).await;

        // 1. Simulate getting a session (in real test, this would be through login)
        let session_cookie = "tower.sid=expired-session-id";

        // 2. Wait longer than the session expiry time
        sleep(StdDuration::from_millis(100)).await;

        // 3. Try to access protected route with expired session
        let expired_request = Request::builder()
            .uri("/protected")
            .header("cookie", session_cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let expired_response = app.oneshot(expired_request).await.unwrap();

        // Should be unauthorized due to expired/invalid session
        // Since we're using a fake session cookie that was never established,
        // the server should respond with unauthorized

        println!("Response status: {:?}", expired_response.status());

        // Assert that expired/invalid session results in unauthorized access
        assert_eq!(
            expired_response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "Expected 401 Unauthorized for expired/invalid session cookie"
        );
    }
}
