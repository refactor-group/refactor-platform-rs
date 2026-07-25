use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};

use domain::error::{DomainErrorKind, EntityErrorKind, Error as DomainError, InternalErrorKind};
use domain::{organization as OrganizationApi, Id};

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, parse_path_id, RejectionType},
    AppState,
};
use log::*;

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Passes if:
/// * User is a SuperAdmin (has `SuperAdmin` role with `organization_id = NULL`), OR
/// * User has an admin role in the specified organization
pub(crate) struct OrganizationAdminAccess(pub Id);

#[async_trait]
impl<S> FromRequestParts<S> for OrganizationAdminAccess
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

        let organization_id = parse_path_id(&path_params, "organization_id")?;

        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, &state).await?;

        if let Err(err) = OrganizationApi::find_by_id(state.db_conn_ref(), organization_id).await {
            let domain_err: DomainError = err.into();
            return match domain_err.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound)) => {
                    Err((
                        StatusCode::NOT_FOUND,
                        format!("Organization {organization_id} not found"),
                    ))
                }
                _ => {
                    error!(
                      "find_by_id({organization_id:?}) failed while verifying organization existence: {domain_err:?}"
                  );
                    Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to verify organization existence".to_string(),
                    ))
                }
            };
        }

        user.roles
            .iter()
            .any(|r| {
                r.role == domain::users::Role::SuperAdmin && r.organization_id.is_none()
                    || r.role == domain::users::Role::Admin
                        && r.organization_id == Some(organization_id)
            })
            .then_some(OrganizationAdminAccess(organization_id))
            .ok_or((StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string()))
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;

    use axum::{
        body::Body, extract::Request, middleware::from_fn, response::Response, routing::get, Router,
    };
    use axum_login::{
        tower_sessions::{MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::user::Backend;
    use domain::{organizations, user_roles, users};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use std::sync::Arc;
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;

    use crate::middleware::auth::require_auth;

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

    fn create_test_user_roles(
        user_id: Id,
        role: users::Role,
        organization_id: Option<Id>,
    ) -> user_roles::Model {
        let now = Utc::now();
        user_roles::Model {
            id: Id::new_v4(),
            role,
            organization_id,
            user_id,
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
        OrganizationAdminAccess(_organization_id): OrganizationAdminAccess,
    ) -> &'static str {
        "extractor_success"
    }

    async fn login_and_build(
        test_user: users::Model,
        test_role: user_roles::Model,
        test_organization: Vec<organizations::Model>,
    ) -> (Router, String) {
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([test_organization.clone()])
                .into_connection(),
        );

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
            .expect("Login should return session cookie")
            .to_string();

        (app, cookie)
    }

    async fn make_protected_request(app: Router, cookie: String, organization_id: Id) -> Response {
        let protected_request = Request::builder()
            .uri(format!("/organizations/{}/coaching_relationships", organization_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        app.oneshot(protected_request).await.unwrap()
    }

    #[tokio::test]
    async fn admin_of_path_org_is_authorized() {
        let organization_id = Id::new_v4();
        let test_user = create_test_user();
        let test_organization = vec![create_test_organization(organization_id)];
        let test_role =
            create_test_user_roles(test_user.id, users::Role::Admin, Some(organization_id));
        let (app, cookie) = login_and_build(test_user, test_role, test_organization).await;

        assert_eq!(
            make_protected_request(app, cookie, organization_id)
                .await
                .status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn super_admin_is_authorized() {
        let organization_id = Id::new_v4();
        let test_user = create_test_user();
        let test_organization = vec![create_test_organization(organization_id)];
        let test_role = create_test_user_roles(test_user.id, users::Role::SuperAdmin, None);
        let (app, cookie) = login_and_build(test_user, test_role, test_organization).await;

        assert_eq!(
            make_protected_request(app, cookie, organization_id)
                .await
                .status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn non_admin_member_is_unauthorized() {
        let organization_id = Id::new_v4();
        let test_user = create_test_user();
        let test_organization = vec![create_test_organization(organization_id)];
        let test_role =
            create_test_user_roles(test_user.id, users::Role::User, Some(organization_id));
        let (app, cookie) = login_and_build(test_user, test_role, test_organization).await;

        assert_eq!(
            make_protected_request(app, cookie, organization_id)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn mismatched_organization_is_unauthorized() {
        let organization_id = Id::new_v4();
        let other_id = Id::new_v4();
        let test_user = create_test_user();
        let test_organization = vec![create_test_organization(organization_id)];
        let test_role = create_test_user_roles(test_user.id, users::Role::Admin, Some(other_id));
        let (app, cookie) = login_and_build(test_user, test_role, test_organization).await;

        assert_eq!(
            make_protected_request(app, cookie, organization_id)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn missing_organization_is_not_found() {
        let other_id = Id::new_v4();
        let test_user = create_test_user();
        let test_role = create_test_user_roles(test_user.id, users::Role::Admin, Some(other_id));
        let empty_organization = vec![];
        let (app, cookie) = login_and_build(test_user, test_role, empty_organization).await;

        assert_eq!(
            make_protected_request(app, cookie, other_id).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
