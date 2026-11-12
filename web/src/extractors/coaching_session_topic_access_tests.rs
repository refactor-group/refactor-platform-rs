use std::sync::Arc;

use crate::middleware::auth::require_auth;

use super::*;
use axum::{body::Body, middleware::from_fn};
use axum::{
    extract::Request,
    http::StatusCode,
    routing::{delete, post},
    Router,
};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::users;
use domain::{coaching_relationships, coaching_sessions, user_roles, Id};
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
        default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(),
        working_hours_start: domain::working_hours::default_start(),
        working_hours_end: domain::working_hours::default_end(),
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// Fixtures share one organization so a participant is also a member of it.
fn fixture_organization_id() -> Id {
    Id::from_u128(1)
}

fn test_role(user_id: Id) -> user_roles::Model {
    let now = Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        role: users::Role::User,
        organization_id: Some(fixture_organization_id()),
        user_id,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn test_session(session_id: Id, relationship_id: Id) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::Model {
        id: session_id,
        coaching_relationship_id: relationship_id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        title: None,
        collab_document_name: None,
        date: now.naive_utc(),
        duration_minutes: domain::duration::Duration::default_minutes(),
        meeting_url: None,
        provider: None,
        hydrated_at: None,
        notice_given_at: chrono::Utc::now().into(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn test_relationship(relationship_id: Id, coachee_id: Id) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id: relationship_id,
        coach_id: Id::new_v4(),
        coachee_id,
        organization_id: fixture_organization_id(),
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn test_topic(
    topic_id: Id,
    coaching_session_id: Id,
    user_id: Id,
) -> coaching_session_topics::Model {
    let now = Utc::now();
    coaching_session_topics::Model {
        id: topic_id,
        coaching_session_id,
        body: "A topic".to_string(),
        user_id,
        display_order: 0,
        status: domain::topic_status::Status::Open,
        moved_from_session_id: None,
        undo_snapshot: None,
        deleted_at: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

// Mounts the delete extractor behind require_auth so a logged-in DELETE exercises the full
// composed chain (session participant -> topic-belongs-to-session -> author-or-coach).
async fn delete_route(
    CoachingSessionTopicDeleteAccess(_topic): CoachingSessionTopicDeleteAccess,
) -> &'static str {
    "extracted_success"
}

// Mounts the undo extractor behind require_auth so a logged-in POST exercises the
// state-derived undo guard (defer: any participant; delete: author only).
async fn undo_route(
    CoachingSessionTopicUndoAccess(_topic): CoachingSessionTopicUndoAccess,
) -> &'static str {
    "extracted_success"
}

fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
    let app_state = AppState::new(
        service::AppState::new(Config::default(), &db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
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
                .route(
                    "/coaching_sessions/:coaching_session_id/topics/:topic_id",
                    delete(delete_route),
                )
                .route(
                    "/coaching_sessions/:coaching_session_id/topics/:topic_id/undo",
                    post(undo_route),
                )
                .route_layer(from_fn(require_auth)),
        )
        .layer(auth_layer)
        .with_state(app_state)
}

async fn do_login(app: &Router) -> String {
    let req = Request::builder()
        .uri("/login")
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("email=test@example.com&password=password123"))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    resp.headers()
        .get("set-cookie")
        .and_then(|c| c.to_str().ok())
        .expect("Login should return session cookie")
        .to_string()
}

#[tokio::test]
async fn delete_extractor_404_when_topic_belongs_to_other_session() {
    let session_id = Id::new_v4();
    let other_session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let topic_id = Id::new_v4();
    let user = create_test_user();
    let role = test_role(user.id);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                test_relationship(relationship_id, user.id),
            )]])
            // Topic belongs to a different session -> session-match guard fails to 404.
            .append_query_results(vec![vec![test_topic(topic_id, other_session_id, user.id)]])
            .into_connection(),
    );

    let app = build_app(Arc::clone(&db));
    let cookie = do_login(&app).await;

    let req = Request::builder()
        .uri(format!("/coaching_sessions/{session_id}/topics/{topic_id}"))
        .method("DELETE")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Builds a soft-deleted topic (deleted_at set) authored by `user_id`.
fn deleted_test_topic(
    topic_id: Id,
    coaching_session_id: Id,
    user_id: Id,
) -> coaching_session_topics::Model {
    let now = Utc::now();
    coaching_session_topics::Model {
        deleted_at: Some(now.into()),
        ..test_topic(topic_id, coaching_session_id, user_id)
    }
}

// Security teeth: a soft-deleted topic authored by someone else is NOT undoable by a
// non-author participant -> 404. The author-only branch fires only for deleted topics.
#[tokio::test]
async fn undo_extractor_404_for_deleted_topic_by_non_author() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let topic_id = Id::new_v4();
    let other_user_id = Id::new_v4();
    let user = create_test_user();
    let role = test_role(user.id);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                test_relationship(relationship_id, user.id),
            )]])
            // Soft-deleted topic authored by someone else -> author guard fails to 404.
            .append_query_results(vec![vec![deleted_test_topic(
                topic_id,
                session_id,
                other_user_id,
            )]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .into_connection(),
    );

    let app = build_app(Arc::clone(&db));
    let cookie = do_login(&app).await;

    let req = Request::builder()
        .uri(format!(
            "/coaching_sessions/{session_id}/topics/{topic_id}/undo"
        ))
        .method("POST")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// A topic that belongs to a different session -> 404 (session-match guard).
#[tokio::test]
async fn undo_extractor_404_when_topic_belongs_to_other_session() {
    let session_id = Id::new_v4();
    let other_session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let topic_id = Id::new_v4();
    let user = create_test_user();
    let role = test_role(user.id);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                test_relationship(relationship_id, user.id),
            )]])
            // Topic belongs to a different session -> session-match guard fails to 404.
            .append_query_results(vec![vec![test_topic(topic_id, other_session_id, user.id)]])
            .into_connection(),
    );

    let app = build_app(Arc::clone(&db));
    let cookie = do_login(&app).await;

    let req = Request::builder()
        .uri(format!(
            "/coaching_sessions/{session_id}/topics/{topic_id}/undo"
        ))
        .method("POST")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
