use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::Id;

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, RejectionType},
    AppState,
};

pub(crate) struct UserMemberAccess(pub Id);

#[async_trait]
impl<S> FromRequestParts<S> for UserMemberAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let AuthenticatedUser(_authenticated_user) =
            AuthenticatedUser::from_request_parts(parts, &state).await?;

        let Path(path_params) = Path::<HashMap<String, String>>::from_request_parts(parts, &state)
            .await
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid path parameters".to_string(),
                )
            })?;

        let user_id_str = path_params
            .get("user_id")
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Invalid user id".to_string()))?;

        let user_id = user_id_str
            .parse::<Id>()
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid user id".to_string()))?;

        Ok(UserMemberAccess(user_id))
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use crate::middleware::auth::require_auth;

    use super::*;
    use axum::{body::Body, middleware::from_fn};
    use axum::{extract::Request, routing::get, Router};
    use axum_login::{
        tower_sessions::{MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::user::Backend;
    use domain::user_roles;
    use domain::users;
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;

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

    async fn protected_route(UserMemberAccess(_user_id): UserMemberAccess) -> &'static str {
        "extractor_success"
    }

    #[tokio::test]
    async fn test_extractor_returns_200_for_a_valid_user_id_path_parameter() {
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();
        let now = Utc::now();

        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: None,
            user_id: test_user.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .into_connection(),
        );

        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
        );

        // Set up session layer
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        // Create app with login route and protected route
        let app = Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route(
                        "/organizations/:organization_id/users/:user_id",
                        get(protected_route),
                    )
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state);

        // First, log in to create an authenticated session
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
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie");

        let protected_request = Request::builder()
            .uri(format!("/organizations/{}/users/{}", organization_id, user_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_extractor_returns_401_when_user_is_unauthenticated() {
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();

        let db = Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection());

        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
        );

        // Set up session layer
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        let app = Router::new()
            .merge(
                Router::new()
                    .route(
                        "/organizations/:organization_id/users/:user_id",
                        get(protected_route),
                    )
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state);

        let protected_request = Request::builder()
            .uri(format!("/organizations/{}/users/{}", organization_id, user_id).as_str())
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
