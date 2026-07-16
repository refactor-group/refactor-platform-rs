#[cfg(test)]
#[cfg(feature = "mock")]
mod organization_user_access_integration_tests {
    use crate::extractors::organization_user_access::OrganizationUserAccess;
    use crate::middleware::auth::require_auth;
    use crate::AppState;

    use axum::{body::Body, extract::Request, middleware::from_fn, routing::get, Router};
    use axum_login::{
        tower_sessions::{MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::user::Backend;
    use domain::{user_roles, users, Id};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use std::sync::Arc;
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;

    const CALLER_EMAIL: &str = "admin@org-a.test";
    const CALLER_PASSWORD: &str = "password123";

    fn user_with_email(email: &str) -> users::Model {
        let now = Utc::now();
        users::Model {
            id: Id::new_v4(),
            email: email.to_string(),
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            display_name: None,
            password: Some(generate_hash(CALLER_PASSWORD)),
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

    fn role_in_org(user_id: Id, organization_id: Id) -> user_roles::Model {
        let now = Utc::now();
        user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(organization_id),
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    /// Test-only handler: echoes the yielded target user's email so the test can
    /// assert the extractor hands the loaded model to the handler.
    async fn protected_route(OrganizationUserAccess(user): OrganizationUserAccess) -> String {
        user.email
    }

    /// Builds the app and logs the caller in, returning the session cookie and app.
    /// `org_membership_rows` is the result the extractor's `find_by_organization`
    /// query will return for the target organization.
    async fn login_and_build(
        org_membership_rows: Vec<(users::Model, user_roles::Model)>,
    ) -> (Router, String) {
        let caller = user_with_email(CALLER_EMAIL);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                // 1. login -> find_by_email(caller)
                .append_query_results([vec![(
                    caller.clone(),
                    role_in_org(caller.id, Id::new_v4()),
                )]])
                // 2. require_auth -> get_user(caller)
                .append_query_results([vec![(
                    caller.clone(),
                    role_in_org(caller.id, Id::new_v4()),
                )]])
                // 3. extractor -> find_by_organization(target org)
                .append_query_results([org_membership_rows])
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
                        "/organizations/:organization_id/users/:user_id",
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
            .body(Body::from(format!(
                "email={CALLER_EMAIL}&password={CALLER_PASSWORD}"
            )))
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

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn yields_user_when_target_is_member_of_path_organization() {
        let organization_id = Id::new_v4();
        let target = user_with_email("member@org-a.test");
        let target_role = role_in_org(target.id, organization_id);

        let (app, cookie) = login_and_build(vec![(target.clone(), target_role)]).await;

        let request = Request::builder()
            .uri(format!(
                "/organizations/{organization_id}/users/{}",
                target.id
            ))
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(body_string(response).await, target.email);
    }

    #[tokio::test]
    async fn rejects_cross_tenant_target_with_not_found() {
        // The IDOR regression: target belongs to a DIFFERENT org than the path org.
        // `find_by_organization(path_org)` returns only the path org's members, which
        // do not include the cross-tenant target -> 404, no model leaked.
        let path_organization_id = Id::new_v4();
        let other_org_member = user_with_email("someone@org-a.test");
        let other_org_role = role_in_org(other_org_member.id, path_organization_id);

        // Target lives in a different organization entirely.
        let cross_tenant_target = user_with_email("victim@org-b.test");

        let (app, cookie) = login_and_build(vec![(other_org_member, other_org_role)]).await;

        let request = Request::builder()
            .uri(format!(
                "/organizations/{path_organization_id}/users/{}",
                cross_tenant_target.id
            ))
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_nonexistent_target_with_not_found() {
        let organization_id = Id::new_v4();
        let missing_user_id = Id::new_v4();

        // Org has no members matching the requested user_id.
        let (app, cookie) = login_and_build(Vec::<(users::Model, user_roles::Model)>::new()).await;

        let request = Request::builder()
            .uri(format!(
                "/organizations/{organization_id}/users/{missing_user_id}"
            ))
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
