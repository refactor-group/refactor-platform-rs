//! Removing a coach from an organization revokes their coaching session write access.

use std::sync::Arc;

use crate::middleware::auth::require_auth;

use super::*;
use axum::{body::Body, middleware::from_fn, middleware::from_fn_with_state};
use axum::{extract::Request, routing::put, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::users;
use domain::{coaching_relationships, coaching_sessions, user_roles};
use password_auth::generate_hash;
use sea_orm::DatabaseConnection;
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
        default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(),
        role: users::Role::User,
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn create_test_session(session_id: Id, relationship_id: Id) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::Model {
        id: session_id,
        coaching_relationship_id: relationship_id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        collab_document_name: None,
        date: now.naive_utc(),
        duration_minutes: domain::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: Some(now.into()),
    }
}

/// A relationship in `organization_id` where the user is the coach.
fn create_coached_relationship(
    relationship_id: Id,
    coach_id: Id,
    organization_id: Id,
) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id: relationship_id,
        coach_id,
        coachee_id: Id::new_v4(),
        organization_id,
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// A relationship in `organization_id` where the user is the coachee.
fn create_coachee_relationship(
    relationship_id: Id,
    coachee_id: Id,
    organization_id: Id,
) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id: relationship_id,
        coach_id: Id::new_v4(),
        coachee_id,
        organization_id,
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn create_test_role(user_id: Id, organization_id: Option<Id>) -> user_roles::Model {
    let now = Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        role: users::Role::User,
        organization_id,
        user_id,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

async fn protected_route() -> &'static str {
    "updated"
}

/// Logs in over the real router, then sends the guarded PUT with the session cookie.
async fn status_for_protected_request(db: Arc<DatabaseConnection>, session_id: Id) -> StatusCode {
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
                    "/coaching_sessions/:coaching_session_id",
                    put(protected_route),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    crate::protect::coaching_sessions::update,
                ))
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

    let protected_request = Request::builder()
        .uri(format!("/coaching_sessions/{}", session_id).as_str())
        .method("PUT")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();

    app.clone()
        .oneshot(protected_request)
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn denies_a_coach_removed_from_the_organization() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let organization_id = Id::new_v4();
    let test_user = create_test_user();
    let test_session = create_test_session(session_id, relationship_id);

    // The user is the coach but holds no role rows at all: removed from every organization.
    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(test_user.clone(), None::<user_roles::Model>)]])
            .append_query_results([vec![(test_user.clone(), None::<user_roles::Model>)]])
            .append_query_results(vec![vec![(
                test_session,
                create_coached_relationship(relationship_id, test_user.id, organization_id),
            )]])
            .into_connection(),
    );

    assert_eq!(
        status_for_protected_request(db, session_id).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn denies_a_coachee_who_is_still_a_member() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let organization_id = Id::new_v4();
    let test_user = create_test_user();
    let test_session = create_test_session(session_id, relationship_id);
    // Membership in the relationship's own organization, but only as the coachee.
    let test_role = create_test_role(test_user.id, Some(organization_id));

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(test_user.clone(), test_role.clone())]])
            .append_query_results([vec![(test_user.clone(), test_role.clone())]])
            .append_query_results(vec![vec![(
                test_session,
                create_coachee_relationship(relationship_id, test_user.id, organization_id),
            )]])
            .into_connection(),
    );

    assert_eq!(
        status_for_protected_request(db, session_id).await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn grants_access_to_a_coach_who_is_still_a_member() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let organization_id = Id::new_v4();
    let test_user = create_test_user();
    let test_session = create_test_session(session_id, relationship_id);
    // Membership in the relationship's own organization.
    let test_role = create_test_role(test_user.id, Some(organization_id));

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(test_user.clone(), test_role.clone())]])
            .append_query_results([vec![(test_user.clone(), test_role.clone())]])
            .append_query_results(vec![vec![(
                test_session,
                create_coached_relationship(relationship_id, test_user.id, organization_id),
            )]])
            .into_connection(),
    );

    assert_eq!(
        status_for_protected_request(db, session_id).await,
        StatusCode::OK
    );
}
