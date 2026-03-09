use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::{user as UserApi, Id};

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, RejectionType},
    AppState,
};
use log::*;

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Passes if:
/// * User is a SuperAdmin (has `SuperAdmin` role with `organization_id = NULL`), OR
/// * User has any role in the specified organization
pub(crate) struct OrganizationMemberAccess(pub Id);

#[async_trait]
impl<S> FromRequestParts<S> for OrganizationMemberAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let Path(path_params) = Path::<HashMap<String, String>>::from_request_parts(parts, &state)
            .await
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid path parameters".to_string(),
                )
            })?;

        let organization_id_str = path_params.get("organization_id").ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid organization id".to_string(),
            )
        })?;

        let organization_id = organization_id_str.parse::<Id>().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid organization id".to_string(),
            )
        })?;

        let AuthenticatedUser(authenticated_user) =
            AuthenticatedUser::from_request_parts(parts, &state).await?;

        // SuperAdmins have access to all organizations
        if authenticated_user
            .roles
            .iter()
            .any(|r| r.role == domain::users::Role::SuperAdmin && r.organization_id.is_none())
        {
            return Ok(OrganizationMemberAccess(organization_id));
        }

        let user_organization_role_exists =
            match UserApi::find_by_organization(state.db_conn_ref(), organization_id).await {
                Ok(users) => users.iter().any(|user| user.id == authenticated_user.id),
                Err(_) => {
                    error!("Organization not found with ID {organization_id:?}");
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "Invalid organization ID".to_string(),
                    ));
                }
            };

        if !user_organization_role_exists {
            return Err((
                StatusCode::UNAUTHORIZED,
                "You are not authorized to access the organization".to_string(),
            ));
        }

        Ok(OrganizationMemberAccess(organization_id))
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
    use domain::{organizations, users};
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

    fn create_test_organization(organization_id: Id) -> organizations::Model {
        let now = Utc::now();
        organizations::Model {
            id: organization_id,
            name: "Refactor Group".to_owned(),
            slug: "refactor-group".to_owned(),
            logo: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    async fn protected_route(
        OrganizationMemberAccess(_organization_id): OrganizationMemberAccess,
    ) -> &'static str {
        "extractor_success"
    }

    #[tokio::test]
    async fn test_extractor_returns_200_for_users_with_organizational_roles() {
        let organization_id = Id::new_v4();
        let now = Utc::now();

        let test_user = create_test_user();
        let _ = create_test_organization(organization_id);

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
                        "/organizations/:organization_id/coaching_relationships",
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
            .uri(format!("/organizations/{}/coaching_relationships", organization_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_extractor_returns_200_for_super_admin_users_without_organizational_roles() {
        let organization_id = Id::new_v4();
        let now = Utc::now();

        let test_user = create_test_user();
        let _ = create_test_organization(organization_id);

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
                        "/organizations/:organization_id/coaching_relationships",
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
            .uri(format!("/organizations/{}/coaching_relationships", organization_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_extractor_returns_401_organization_for_regular_users_without_organization_roles()
    {
        let organization_id = Id::new_v4();
        let now = Utc::now();

        let test_user = create_test_user();
        let _ = create_test_organization(organization_id);

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
                        "/organizations/:organization_id/coaching_relationships",
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
            .uri(format!("/organizations/{}/coaching_relationships", organization_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
