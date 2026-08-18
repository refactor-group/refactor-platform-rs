use crate::extractors::organization_admin_access::OrganizationAdminAccess;
use crate::middleware::auth::require_auth;
use crate::AppState;
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, middleware::from_fn, routing::get, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::{organizations, user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, MockDatabase};
use service::config::Config;
use std::sync::Arc;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

fn test_user() -> users::Model {
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
        default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(),
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn test_role(user_id: Id, organization_id: Option<Id>, role: users::Role) -> user_roles::Model {
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

fn test_organization(organization_id: Id) -> organizations::Model {
    let now = Utc::now();
    organizations::Model {
        id: organization_id,
        name: "Refactor Group".to_owned(),
        slug: "refactor-group".to_owned(),
        logo: None,
        created_at: now.into(),
        updated_at: now.into(),
        archived_at: None,
        archived_by: None,
    }
}

async fn protected_route(_: OrganizationAdminAccess) -> &'static str {
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

    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(1)))
        .with_always_save(true);
    let auth_layer = AuthManagerLayerBuilder::new(Backend::new(&db), session_layer).build();

    Router::new()
        .route(
            "/login",
            axum::routing::post(crate::controller::user_session_controller::login),
        )
        .merge(
            Router::new()
                .route(
                    "/organizations/:organization_id/admin",
                    get(protected_route),
                )
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
        .expect("login should return a session cookie")
        .to_string()
}

/// Drives the extractor with a requester holding `role`, against an organization
/// that exists unless `organization_exists` is false.
async fn request_status(
    organization_id: Id,
    role: user_roles::Model,
    organization_exists: bool,
) -> StatusCode {
    let user = test_user();
    let mut mock = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![(user.clone(), role.clone())]]);

    mock = if organization_exists {
        mock.append_query_results([vec![test_organization(organization_id)]])
    } else {
        mock.append_query_results([Vec::<organizations::Model>::new()])
    };

    let app = build_app(Arc::new(mock.into_connection()));
    let cookie = login_cookie(&app).await;

    let request = Request::builder()
        .uri(format!("/organizations/{organization_id}/admin"))
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();

    app.clone().oneshot(request).await.unwrap().status()
}

#[tokio::test]
async fn extractor_returns_200_for_a_global_super_admin() {
    let organization_id = Id::new_v4();
    let role = test_role(Id::new_v4(), None, users::Role::SuperAdmin);

    assert_eq!(
        request_status(organization_id, role, true).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn extractor_returns_200_for_an_admin_of_the_target_organization() {
    let organization_id = Id::new_v4();
    let role = test_role(Id::new_v4(), Some(organization_id), users::Role::Admin);

    assert_eq!(
        request_status(organization_id, role, true).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn extractor_returns_403_for_a_plain_member_of_the_target_organization() {
    let organization_id = Id::new_v4();
    let role = test_role(Id::new_v4(), Some(organization_id), users::Role::User);

    assert_eq!(
        request_status(organization_id, role, true).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn extractor_returns_403_for_an_admin_of_a_different_organization() {
    let organization_id = Id::new_v4();
    let role = test_role(Id::new_v4(), Some(Id::new_v4()), users::Role::Admin);

    assert_eq!(
        request_status(organization_id, role, true).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn extractor_returns_404_when_the_organization_does_not_exist() {
    let organization_id = Id::new_v4();
    let role = test_role(Id::new_v4(), Some(organization_id), users::Role::Admin);

    assert_eq!(
        request_status(organization_id, role, false).await,
        StatusCode::NOT_FOUND
    );
}

/// I-3. A caller who does not administer the organization must not learn whether it
/// exists. The two statuses are compared rather than asserted separately, because
/// the property is that they are indistinguishable, not that either has a
/// particular value.
///
/// Before the existence lookup was moved after the admin check, an absent
/// organization answered 404 and an existing one 403, so any authenticated user
/// could enumerate organization ids.
#[tokio::test]
async fn extractor_does_not_disclose_organization_existence_to_a_non_admin() {
    let organization_id = Id::new_v4();
    let member = || test_role(Id::new_v4(), Some(organization_id), users::Role::User);

    let against_existing = request_status(organization_id, member(), true).await;
    let against_absent = request_status(organization_id, member(), false).await;

    assert_eq!(
        against_existing, against_absent,
        "a non-admin must get the same answer whether the organization exists"
    );
    assert_eq!(
        against_existing,
        StatusCode::FORBIDDEN,
        "and that answer must be the refusal, not the disclosure"
    );
}

/// The companion to the case above: a foreign-org admin is equally a non-admin
/// here, and is the likelier attacker since they hold real admin rights somewhere.
#[tokio::test]
async fn extractor_does_not_disclose_organization_existence_to_a_foreign_admin() {
    let organization_id = Id::new_v4();
    let foreign_admin = || test_role(Id::new_v4(), Some(Id::new_v4()), users::Role::Admin);

    let against_existing = request_status(organization_id, foreign_admin(), true).await;
    let against_absent = request_status(organization_id, foreign_admin(), false).await;

    assert_eq!(
        against_existing, against_absent,
        "a foreign-org admin must get the same answer whether the organization exists"
    );
    // Anchored, or a regression to 404 on both branches would satisfy the equality
    // above while disclosing exactly what this test exists to hide.
    assert_eq!(
        against_existing,
        StatusCode::FORBIDDEN,
        "and that answer must be the refusal, not the disclosure"
    );
}
