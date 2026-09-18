use crate::AppState;
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::{coaching_relationships, organizations, user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, MockDatabase};
use service::config::Config;
use std::sync::Arc;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

fn requester() -> users::Model {
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

/// Mirrors the production route. Authorization lives entirely in the handler's
/// `OrganizationAdminAccess` extractor, so no route layer is involved.
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
            post(crate::controller::user_session_controller::login),
        )
        .route(
            "/organizations/:organization_id/coaching_relationships",
            post(super::create),
        )
        .with_state(app_state)
        .layer(auth_layer)
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

async fn create_request(app: &Router, cookie: &str, organization_id: Id) -> StatusCode {
    let body = format!(
        r#"{{"coach_id":"{}","coachee_id":"{}"}}"#,
        Id::new_v4(),
        Id::new_v4()
    );
    let request = Request::builder()
        .uri(format!(
            "/organizations/{organization_id}/coaching_relationships"
        ))
        .method("POST")
        .header("cookie", cookie)
        .header("x-version", "1.0.0-beta1")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

/// Login's two user lookups, then the organization `OrganizationAdminAccess` resolves.
/// The role check that follows is in memory, so authorization needs nothing further.
fn mock_through_authorization(
    user: &users::Model,
    role: &user_roles::Model,
    organization_id: Id,
) -> MockDatabase {
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![test_organization(organization_id)]])
}

/// Authorization let the request through. The create itself runs off the end of the
/// mock queue, so nothing past the gate is worth pinning.
fn assert_authorized(status: StatusCode) {
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_ne!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_allows_an_organization_admin() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(mock_through_authorization(&user, &role, organization_id).into_connection());
    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    assert_authorized(create_request(&app, &cookie, organization_id).await);
}

#[tokio::test]
async fn create_allows_a_global_super_admin() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);

    let db = Arc::new(mock_through_authorization(&user, &role, organization_id).into_connection());
    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    assert_authorized(create_request(&app, &cookie, organization_id).await);
}

#[tokio::test]
async fn create_denies_a_plain_member() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);

    let db = Arc::new(mock_through_authorization(&user, &role, organization_id).into_connection());
    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    assert_eq!(
        create_request(&app, &cookie, organization_id).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn create_denies_an_admin_of_a_different_organization() {
    // Admin somewhere, and a member here, but not an admin here.
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(Id::new_v4()), users::Role::Admin);

    let db = Arc::new(mock_through_authorization(&user, &role, organization_id).into_connection());
    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    assert_eq!(
        create_request(&app, &cookie, organization_id).await,
        StatusCode::FORBIDDEN
    );
}

fn relationship(
    id: Id,
    organization_id: Id,
    coach_id: Id,
    coachee_id: Id,
) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id,
        organization_id,
        coach_id,
        coachee_id,
        slug: "coach-coachee".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// Login's two user lookups, then the relationship `CoachingRelationshipAccess` resolves.
fn mock_through_relationship_read(
    user: &users::Model,
    role: &user_roles::Model,
    relationship: &coaching_relationships::Model,
) -> Arc<sea_orm::DatabaseConnection> {
    Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![relationship.clone()]])
            .into_connection(),
    )
}

async fn read_request(
    app: &Router,
    cookie: &str,
    organization_id: Id,
    relationship_id: Id,
) -> StatusCode {
    let request = Request::builder()
        .uri(format!(
            "/organizations/{organization_id}/coaching_relationships/{relationship_id}"
        ))
        .header("cookie", cookie)
        .header("x-version", "1.0.0-beta1")
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

fn build_read_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
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
            post(crate::controller::user_session_controller::login),
        )
        .route(
            "/organizations/:organization_id/coaching_relationships/:relationship_id",
            axum::routing::get(super::read),
        )
        .with_state(app_state)
        .layer(auth_layer)
}

#[tokio::test]
async fn read_denies_a_non_participant() {
    let organization_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);
    // A member of the organization, but this relationship is between two other people.
    let other = relationship(relationship_id, organization_id, Id::new_v4(), Id::new_v4());

    let db = mock_through_relationship_read(&user, &role, &other);
    let app = build_read_app(db);
    let cookie = login_cookie(&app).await;

    assert_eq!(
        read_request(&app, &cookie, organization_id, relationship_id).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn read_denies_a_participant_removed_from_the_organization() {
    let organization_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let user = requester();
    // Still the coachee, but their only role is in some other organization.
    let role = test_role(user.id, Some(Id::new_v4()), users::Role::User);
    let own = relationship(relationship_id, organization_id, Id::new_v4(), user.id);

    let db = mock_through_relationship_read(&user, &role, &own);
    let app = build_read_app(db);
    let cookie = login_cookie(&app).await;

    assert_eq!(
        read_request(&app, &cookie, organization_id, relationship_id).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn read_allows_a_participant_who_is_still_a_member() {
    let organization_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);
    let own = relationship(relationship_id, organization_id, Id::new_v4(), user.id);

    let db = mock_through_relationship_read(&user, &role, &own);
    let app = build_read_app(db);
    let cookie = login_cookie(&app).await;

    let status = read_request(&app, &cookie, organization_id, relationship_id).await;
    assert_ne!(status, StatusCode::FORBIDDEN);
    assert_ne!(status, StatusCode::UNAUTHORIZED);
}

/// The organization segment addresses the relationship. Pointing a valid relationship
/// id at the wrong organization must not resolve, even for a legitimate participant.
#[tokio::test]
async fn read_denies_a_relationship_addressed_under_the_wrong_organization() {
    let organization_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);
    // The caller really is the coachee and really is a member, but the relationship
    // belongs to a different organization than the one in the URL.
    let elsewhere = relationship(relationship_id, Id::new_v4(), Id::new_v4(), user.id);

    let db = mock_through_relationship_read(&user, &role, &elsewhere);
    let app = build_read_app(db);
    let cookie = login_cookie(&app).await;

    assert_eq!(
        read_request(&app, &cookie, organization_id, relationship_id).await,
        StatusCode::NOT_FOUND
    );
}
