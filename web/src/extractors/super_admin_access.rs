use crate::extractors::{authenticated_user::AuthenticatedUser, RejectionType};
use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use domain::users;

/// Authorizes a system SuperAdmin (role `SuperAdmin` with no organization).
///
/// Carries the authenticated user so handlers can record who acted.
pub(crate) struct SuperAdminAccess {
    pub authenticated_user: users::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for SuperAdminAccess
where
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthenticatedUser(authenticated_user) =
            AuthenticatedUser::from_request_parts(parts, state).await?;

        let is_super_admin = authenticated_user
            .roles
            .iter()
            .any(|r| r.role == users::Role::SuperAdmin && r.organization_id.is_none());

        if is_super_admin {
            Ok(SuperAdminAccess { authenticated_user })
        } else {
            Err((
                StatusCode::FORBIDDEN,
                "SuperAdmin access required".to_string(),
            ))
        }
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use crate::middleware::auth::require_auth;
    use crate::AppState;

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
    use domain::{users, Id};
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
            password: Some(generate_hash("password123")),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(
            ),
            role: users::Role::User,
            roles: vec![],
            invite_status: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    async fn protected_route(_: SuperAdminAccess) -> &'static str {
        "extractor_success"
    }

    fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
            None,
            None,
        );

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route("/protected", get(protected_route))
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state)
    }

    async fn login_cookie(app: &Router) -> String {
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

    #[tokio::test]
    async fn test_extractor_returns_200_for_super_admin() {
        let now = Utc::now();
        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::SuperAdmin,
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

        let app = build_app(db);
        let cookie = login_cookie(&app).await;

        let protected_request = Request::builder()
            .uri("/protected")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_extractor_returns_403_for_regular_user() {
        let now = Utc::now();
        let organization_id = Id::new_v4();
        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(organization_id),
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

        let app = build_app(db);
        let cookie = login_cookie(&app).await;

        let protected_request = Request::builder()
            .uri("/protected")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// A SuperAdmin role scoped to an organization (organization_id = Some) is
    /// NOT a system super admin and must be rejected. Pins the org_id.is_none()
    /// clause of the extractor predicate.
    #[tokio::test]
    async fn test_extractor_returns_403_for_org_scoped_super_admin() {
        let now = Utc::now();
        let organization_id = Id::new_v4();
        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::SuperAdmin,
            organization_id: Some(organization_id),
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

        let app = build_app(db);
        let cookie = login_cookie(&app).await;

        let protected_request = Request::builder()
            .uri("/protected")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
