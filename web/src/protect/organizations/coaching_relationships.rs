use crate::error::domain_error_into_response;
use crate::protect::{Predicate, UserIsAdmin};
use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};

use domain::{coaching_relationship as CoachingRelationshipApi, Id};

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// and that the authenticated user is an admin
/// Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn create(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(organization_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let checks: Vec<Predicate> = vec![Predicate::new(UserIsAdmin, vec![organization_id])];

    crate::protect::authorize(&app_state, authenticated_user, request, next, checks).await
}

/// Checks that the authenticated user is a participant (coach or coachee)
/// in the coaching relationship specified by `relationship_id`.
pub(crate) async fn actions(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path((_organization_id, relationship_id)): Path<(Id, Id)>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let relationship_result: Result<_, domain::error::Error> =
        CoachingRelationshipApi::find_by_id(app_state.db_conn_ref(), relationship_id)
            .await
            .map_err(Into::into);

    match relationship_result {
        Ok(relationship) => {
            if relationship.includes_user(authenticated_user.id) {
                next.run(request).await
            } else {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
        }
        Err(e) => domain_error_into_response(e),
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::Request;
    use axum::middleware::from_fn;
    use axum::routing::get;
    use axum::Router;
    use axum_login::tower_sessions::{MemoryStore, SessionManagerLayer};
    use axum_login::AuthManagerLayerBuilder;
    use chrono::Utc;
    use domain::user::Backend;
    use domain::{coaching_relationships, user_roles, users, Id};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;

    use crate::middleware::auth::require_auth;
    use crate::AppState;

    fn create_test_user(id: Id) -> users::Model {
        let now = Utc::now();
        users::Model {
            id,
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

    async fn dummy_handler() -> &'static str {
        "ok"
    }

    async fn login(app: &Router) -> String {
        let login_request = Request::builder()
            .uri("/login")
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("email=test@example.com&password=password123"))
            .unwrap();

        let login_response = app.clone().oneshot(login_request).await.unwrap();

        login_response
            .headers()
            .get("set-cookie")
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie")
            .to_string()
    }

    /// Proves the protect middleware rejects a user who is NOT a participant
    /// in the coaching relationship. This tests the middleware in isolation
    /// with a dummy handler — the middleware itself returns 401 before the
    /// handler is ever called.
    #[tokio::test]
    async fn actions_middleware_rejects_non_participant() {
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = Utc::now();

        let test_user = create_test_user(user_id);
        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(organization_id),
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Relationship where the user is NOT a participant
        let relationship = coaching_relationships::Model {
            id: relationship_id,
            organization_id,
            coach_id: Id::new_v4(),   // different user
            coachee_id: Id::new_v4(), // different user
            slug: "other-relationship".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        // FIFO: 2 user mocks for login, then find_by_id at position 2
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![relationship]])
                .into_connection(),
        );

        let app_state = AppState::new(
            service::AppState::new(service::config::Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
        );

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        let app = Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route(
                        "/organizations/:organization_id/coaching_relationships/:relationship_id/actions",
                        get(dummy_handler),
                    )
                    .route_layer(axum::middleware::from_fn_with_state(
                        app_state.clone(),
                        super::actions,
                    ))
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state);

        let cookie = login(&app).await;

        let request = Request::builder()
            .uri(format!(
                "/organizations/{}/coaching_relationships/{}/actions",
                organization_id, relationship_id
            ))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "Non-participant should be rejected by protect middleware"
        );
    }

    /// Proves the protect middleware allows a user who IS the coach
    /// in the coaching relationship. The dummy handler returns 200.
    #[tokio::test]
    async fn actions_middleware_allows_coach() {
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = Utc::now();

        let test_user = create_test_user(user_id);
        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(organization_id),
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Relationship where the user IS the coach
        let relationship = coaching_relationships::Model {
            id: relationship_id,
            organization_id,
            coach_id: user_id, // authenticated user is the coach
            coachee_id: Id::new_v4(),
            slug: "my-relationship".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        // FIFO: 2 user mocks for login, then find_by_id at position 2
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![relationship]])
                .into_connection(),
        );

        let app_state = AppState::new(
            service::AppState::new(service::config::Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
        );

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        let app = Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route(
                        "/organizations/:organization_id/coaching_relationships/:relationship_id/actions",
                        get(dummy_handler),
                    )
                    .route_layer(axum::middleware::from_fn_with_state(
                        app_state.clone(),
                        super::actions,
                    ))
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state);

        let cookie = login(&app).await;

        let request = Request::builder()
            .uri(format!(
                "/organizations/{}/coaching_relationships/{}/actions",
                organization_id, relationship_id
            ))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "Coach should be allowed through protect middleware"
        );
    }
}
