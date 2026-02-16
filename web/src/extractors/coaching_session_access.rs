use axum::{async_trait, extract::{FromRef, FromRequestParts, Path}, http::{StatusCode, request::Parts}};
use domain::{Id, coaching_session};

use crate::{AppState, extractors::{RejectionType, authenticated_user::AuthenticatedUser}};
use domain::coaching_sessions;
use domain::coaching_relationship;
use log::*;

pub(crate) struct CoachingSessionAccess(pub coaching_sessions::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let AuthenticatedUser(authenticated_user) = AuthenticatedUser::from_request_parts(parts, &state).await?;

        let Path(coaching_session_id) = match Path::<Id>::from_request_parts(parts, &state)
            .await
            {
                Ok(path) => path,
                Err(_e) => {
                    return Err((StatusCode::BAD_REQUEST, "Invalid coaching session id".to_string()));
                }
            };
        debug!("GET Coaching Session by ID: {coaching_session_id}");

        // Get the coaching session
        let coaching_session = match coaching_session::find_by_id(
            state.db_conn_ref(),
            coaching_session_id,
        )
        .await
        {
            Ok(session) => session,
            Err(e) => {
                error!("Error finding coaching session {coaching_session_id}: {e:?}");
                return Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()));
            }
        };

        debug!("Found Coaching Session: {coaching_session:?}");

        // Get the coaching relationship
        let coaching_relationship = match coaching_relationship::find_by_id(
            state.db_conn_ref(),
            coaching_session.coaching_relationship_id,
        )
        .await
        {
            Ok(relationship) => relationship,
            Err(e) => {
                error!(
                    "Error finding coaching relationship {}: {e:?}",
                    coaching_session.coaching_relationship_id
                );
                return Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()));
            }
        };

        // Check if user is coach or coachee
        if (
            coaching_relationship.coach_id == authenticated_user.id
            || coaching_relationship.coachee_id == authenticated_user.id) == false
        {
            return Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()));
        }

        Ok(CoachingSessionAccess(coaching_session))
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use crate::middleware::auth::require_auth;

    use super::*;
    use axum::{body::Body, middleware::from_fn};
    use domain::{coaching_relationships, user_roles};
    use sea_orm::{DatabaseBackend, MockDatabase};
    use axum::{extract::Request, routing::get, Router};
    use password_auth::generate_hash;
    use domain::user::Backend;
    use chrono::Utc;
    use axum_login::{
        tower_sessions::{MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use service::config::Config;
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;
    use domain::users;

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

    async fn protected_route(CoachingSessionAccess(_coaching_session): CoachingSessionAccess) -> &'static str {
        "extracted_success"
    }

    #[tokio::test]
    async fn test_coaching_session_extractor_success() {
        // Create mock database with expected results
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = Utc::now();
        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(Id::new_v4()),
            user_id: test_user.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let test_session = coaching_sessions::Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: chrono::Utc::now().naive_utc(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results(vec![vec![test_session.clone()]])
                .append_query_results(
                    vec![
                        vec![coaching_relationships::Model {
                            id: relationship_id,
                            coach_id: Id::new_v4(),
                            coachee_id: test_user.id,
                            organization_id: Id::new_v4(),
                            slug: "test".to_string(),
                            created_at: now.into(),
                            updated_at: now.into(),
                        }]
                    ]
                )
                .into_connection()
        );

        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default()
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
                    .route("/coaching_sessions/:coaching_session_id", get(protected_route))
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

        // Extract session cookie from login response
        let cookie = login_response
            .headers()
            .get("set-cookie")
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie");

        let protected_request = Request::builder()
            .uri(format!("/coaching_sessions/{}", session_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_coaching_session_extractor_returns_401_when_user_is_unauthenticated() {
        // Create mock database with expected results
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = Utc::now();
        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(Id::new_v4()),
            user_id: test_user.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let test_session = coaching_sessions::Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: chrono::Utc::now().naive_utc(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results(vec![vec![test_session.clone()]])
                .into_connection()
        );

        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default()
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
                    .route("/coaching_sessions/:coaching_session_id", get(protected_route))
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state);

        let protected_request = Request::builder()
            .uri(format!("/coaching_sessions/{}", session_id).as_str())
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_coaching_session_extractor_returns_401_when_session_does_not_exist() {
        // Create mock database with expected results
        let session_id = Id::new_v4();
        let now = Utc::now();
        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(Id::new_v4()),
            user_id: test_user.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .into_connection()
        );

        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default()
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
                    .route("/coaching_sessions/:coaching_session_id", get(protected_route))
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

        // Extract session cookie from login response
        let cookie = login_response
            .headers()
            .get("set-cookie")
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie");

        let protected_request = Request::builder()
            .uri(format!("/coaching_sessions/{}", session_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_coaching_session_extractor_returns_401_when_coaching_session_exists_but_user_relationship_does_not() {
        // Create mock database with expected results
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = Utc::now();
        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(Id::new_v4()),
            user_id: test_user.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let test_session = coaching_sessions::Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: chrono::Utc::now().naive_utc(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results(vec![vec![test_session.clone()]])
                .into_connection()
        );

        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default()
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
                    .route("/coaching_sessions/:coaching_session_id", get(protected_route))
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

        // Extract session cookie from login response
        let cookie = login_response
            .headers()
            .get("set-cookie")
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie");

        let protected_request = Request::builder()
            .uri(format!("/coaching_sessions/{}", session_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
