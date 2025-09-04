use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use axum_login::{tower_sessions::Session, AuthSession};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReadOnlySession {
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "time::serde::iso8601::option", default)]
    pub expires_at: Option<OffsetDateTime>,
}

impl ReadOnlySession {
    pub fn unauthenticated() -> Self {
        Self {
            authenticated: false,
            expires_at: None,
        }
    }

    pub fn authenticated(expires_at: OffsetDateTime) -> Self {
        Self {
            authenticated: true,
            expires_at: Some(expires_at),
        }
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for ReadOnlySession
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // First, try to get the AuthSession to check if user is authenticated
        let auth_session_result: Result<domain::user::AuthSession, _> =
            AuthSession::from_request_parts(parts, state).await;

        match auth_session_result {
            Ok(auth_session) if auth_session.user.is_some() => {
                // User is authenticated, now get the session expiry
                // Extract the tower_sessions::Session to get expiry information
                let session_result: Result<Session, _> =
                    Session::from_request_parts(parts, state).await;

                if let Ok(session) = session_result {
                    // Get the expiry from the session
                    // Note: tower_sessions stores expiry as a key in the session
                    // The actual implementation depends on the session configuration
                    let expiry = session.expiry_date();
                    Ok(ReadOnlySession::authenticated(expiry))
                } else {
                    // Authenticated but can't get expiry, return without expiry
                    Ok(ReadOnlySession {
                        authenticated: true,
                        expires_at: None,
                    })
                }
            }
            _ => {
                // Not authenticated
                Ok(ReadOnlySession::unauthenticated())
            }
        }
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::Request,
        http::StatusCode,
        response::Response,
        routing::{get, post},
        Router,
    };
    use axum_login::{
        tower_sessions::{Expiry, MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::user::{Backend, Credentials};
    use domain::{users, Id};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use serde_json::Value;
    use std::sync::Arc;
    use time::Duration;
    use tower::ServiceExt;

    // Helper function to create a test user
    fn create_test_user() -> users::Model {
        users::Model {
            id: Id::new_v4(),
            email: "test@example.com".to_string(),
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            display_name: Some("Test User".to_string()),
            password: generate_hash("password123"),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            role: users::Role::User,
            created_at: Utc::now().into(),
            updated_at: Utc::now().into(),
        }
    }

    // Helper function to create test app with configurable session duration
    async fn create_test_app_with_expiry(expiry_seconds: i64) -> Router {
        let user = create_test_user();
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([[user.clone()]]) // For user lookup
                .append_query_results([[user.clone()]]) // For authentication
                .into_connection(),
        );

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::seconds(expiry_seconds)));

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        Router::new()
            .route("/login", post(test_login_handler))
            .route("/test", get(test_readonly_session_handler))
            .layer(auth_layer)
    }

    // Test handler that uses ReadOnlySession extractor
    async fn test_readonly_session_handler(session: ReadOnlySession) -> Response<Body> {
        let json = serde_json::to_string(&session).unwrap();
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(json))
            .unwrap()
    }

    // Simple login handler for testing
    async fn test_login_handler(
        mut auth_session: domain::user::AuthSession,
        axum::Form(creds): axum::Form<Credentials>,
    ) -> Response<Body> {
        if let Ok(Some(user)) = auth_session.authenticate(creds).await {
            if auth_session.login(&user).await.is_ok() {
                return Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from("logged in"))
                    .unwrap();
            }
        }
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_readonly_session_unauthenticated() {
        let app = create_test_app_with_expiry(3600).await;

        let request = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let session: ReadOnlySession = serde_json::from_slice(&body_bytes).unwrap();

        assert!(!session.authenticated, "Should not be authenticated");
        assert!(
            session.expires_at.is_none(),
            "Should have no expiry when unauthenticated"
        );
    }

    #[tokio::test]
    async fn test_readonly_session_authenticated() {
        let app = create_test_app_with_expiry(3600).await;

        // First login to establish session
        let login_request = Request::builder()
            .uri("/login")
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("email=test@example.com&password=password123"))
            .unwrap();

        let login_response = app.clone().oneshot(login_request).await.unwrap();
        assert_eq!(login_response.status(), StatusCode::OK);

        // Extract session cookie
        let cookie = login_response
            .headers()
            .get("set-cookie")
            .and_then(|h| h.to_str().ok())
            .expect("Should have session cookie");

        // Test authenticated session
        let test_request = Request::builder()
            .uri("/test")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(test_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let session: ReadOnlySession = serde_json::from_slice(&body_bytes).unwrap();

        assert!(session.authenticated, "Should be authenticated");
        assert!(
            session.expires_at.is_some(),
            "Should have expiry when authenticated"
        );
    }

    #[tokio::test]
    async fn test_readonly_session_expiry_present() {
        let app = create_test_app_with_expiry(1800).await; // 30 minutes

        // Login and get cookie
        let login_request = Request::builder()
            .uri("/login")
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("email=test@example.com&password=password123"))
            .unwrap();

        let login_response = app.clone().oneshot(login_request).await.unwrap();
        let cookie = login_response
            .headers()
            .get("set-cookie")
            .and_then(|h| h.to_str().ok())
            .unwrap();

        // Test that expiry is correctly extracted
        let test_request = Request::builder()
            .uri("/test")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(test_request).await.unwrap();
        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let session: ReadOnlySession = serde_json::from_slice(&body_bytes).unwrap();

        assert!(session.authenticated);
        let expiry = session.expires_at.expect("Should have expiry timestamp");

        // Expiry should be in the future
        let now = time::OffsetDateTime::now_utc();
        assert!(expiry > now, "Expiry should be in the future");

        // Should be approximately 30 minutes from now (allowing some test execution time)
        let expected_expiry_range = now + Duration::seconds(1700)..now + Duration::seconds(1900);
        assert!(
            expected_expiry_range.contains(&expiry),
            "Expiry should be approximately 30 minutes from now. Expected: {:?}, Got: {}",
            expected_expiry_range,
            expiry
        );
    }

    #[tokio::test]
    async fn test_readonly_session_serialization() {
        // Test authenticated session serialization
        let expiry = time::OffsetDateTime::now_utc() + Duration::seconds(3600);
        let authenticated_session = ReadOnlySession::authenticated(expiry);

        let json = serde_json::to_string(&authenticated_session).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["authenticated"], true);
        assert!(parsed["expires_at"].is_string());

        // Test unauthenticated session serialization
        let unauthenticated_session = ReadOnlySession::unauthenticated();
        let json = serde_json::to_string(&unauthenticated_session).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["authenticated"], false);
        assert!(parsed.get("expires_at").is_none() || parsed["expires_at"].is_null());
    }

    #[tokio::test]
    async fn test_readonly_session_invalid_cookie() {
        let app = create_test_app_with_expiry(3600).await;

        // Test with invalid/corrupted cookie
        let request = Request::builder()
            .uri("/test")
            .header("cookie", "id=invalid-session-id")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let session: ReadOnlySession = serde_json::from_slice(&body_bytes).unwrap();

        assert!(
            !session.authenticated,
            "Invalid cookie should result in unauthenticated state"
        );
        assert!(
            session.expires_at.is_none(),
            "Invalid cookie should have no expiry"
        );
    }
}
