use std::sync::Arc;

use crate::middleware::auth::require_auth;

use super::*;
use axum::{body::Body, middleware::from_fn};
use axum::{
    extract::Request,
    routing::{delete, patch, post},
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
        role: users::Role::User,
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn test_role(user_id: Id) -> user_roles::Model {
    let now = Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        role: users::Role::User,
        organization_id: Some(Id::new_v4()),
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
        title: None,
        collab_document_name: None,
        date: now.naive_utc(),
        duration_minutes: domain::duration::Duration::default_minutes(),
        meeting_url: None,
        provider: None,
        hydrated_at: None,
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
        organization_id: Id::new_v4(),
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn test_relationship_with_coach(
    relationship_id: Id,
    coach_id: Id,
    coachee_id: Id,
) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id: relationship_id,
        coach_id,
        coachee_id,
        organization_id: Id::new_v4(),
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
        priority: Some(domain::topic_priority::Priority::High),
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

// Mounts the coachee extractor behind require_auth so a logged-in PATCH exercises the
// coachee-only rating guard (coachee -> ok, coach -> 403).
async fn coachee_route(
    CoachingSessionTopicCoacheeAccess(_topic): CoachingSessionTopicCoacheeAccess,
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
                    "/coaching_sessions/:coaching_session_id/topics/:topic_id/rating",
                    patch(coachee_route),
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
async fn delete_extractor_ok_when_author() {
    let session_id = Id::new_v4();
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
            .append_query_results(vec![vec![test_topic(topic_id, session_id, user.id)]])
            .append_query_results([vec![(user.clone(), role.clone())]])
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
    assert_eq!(resp.status(), StatusCode::OK);
}

// A coachee deleting the coach's topic: not the author, not the coach -> 404.
#[tokio::test]
async fn delete_extractor_404_when_coachee_deletes_coachs_topic() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let topic_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let user = create_test_user(); // caller acts as the coachee
    let role = test_role(user.id);
    let relationship = test_relationship_with_coach(relationship_id, coach_id, user.id);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                relationship.clone(),
            )]])
            // Topic authored by the coach -> caller is not the author.
            .append_query_results(vec![vec![test_topic(topic_id, session_id, coach_id)]])
            // Coach check: caller is the coachee, not the coach -> 404.
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                relationship.clone(),
            )]])
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

// A coach deleting the coachee's topic: not the author, but is the coach -> allowed.
#[tokio::test]
async fn delete_extractor_ok_when_coach_deletes_coachees_topic() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let topic_id = Id::new_v4();
    let coachee_id = Id::new_v4();
    let user = create_test_user(); // caller acts as the coach
    let role = test_role(user.id);
    let relationship = test_relationship_with_coach(relationship_id, user.id, coachee_id);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                relationship.clone(),
            )]])
            // Topic authored by the coachee -> caller (coach) is not the author.
            .append_query_results(vec![vec![test_topic(topic_id, session_id, coachee_id)]])
            // Coach check: caller IS the coach of the relationship -> allowed.
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                relationship.clone(),
            )]])
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
    assert_eq!(resp.status(), StatusCode::OK);
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

#[tokio::test]
async fn coachee_extractor_ok_when_user_is_coachee() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let topic_id = Id::new_v4();
    let user = create_test_user();
    let role = test_role(user.id);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            // CoachingSessionTopicAccess: participant check (caller is the coachee) + topic load.
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                test_relationship(relationship_id, user.id),
            )]])
            .append_query_results(vec![vec![test_topic(topic_id, session_id, user.id)]])
            // Coachee gate: re-resolve the relationship; caller IS the coachee -> rating allowed.
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                test_relationship(relationship_id, user.id),
            )]])
            .into_connection(),
    );

    let app = build_app(Arc::clone(&db));
    let cookie = do_login(&app).await;

    let req = Request::builder()
        .uri(format!(
            "/coaching_sessions/{session_id}/topics/{topic_id}/rating"
        ))
        .method("PATCH")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn coachee_extractor_403_when_user_is_coach() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let topic_id = Id::new_v4();
    let coachee_id = Id::new_v4();
    let user = create_test_user();
    let role = test_role(user.id);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            // CoachingSessionTopicAccess: caller is the coach (a participant) -> passes; topic loads.
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                test_relationship_with_coach(relationship_id, user.id, coachee_id),
            )]])
            .append_query_results(vec![vec![test_topic(topic_id, session_id, coachee_id)]])
            // Coachee gate: re-resolve the relationship; caller is the coach, not the coachee -> 403.
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                test_relationship_with_coach(relationship_id, user.id, coachee_id),
            )]])
            .into_connection(),
    );

    let app = build_app(Arc::clone(&db));
    let cookie = do_login(&app).await;

    let req = Request::builder()
        .uri(format!(
            "/coaching_sessions/{session_id}/topics/{topic_id}/rating"
        ))
        .method("PATCH")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
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

// Undoing a defer is open to either participant: a live (not soft-deleted) topic
// authored by someone else is still undoable by the caller -> 200.
#[tokio::test]
async fn undo_extractor_ok_for_live_topic_by_non_author() {
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
            // Live topic authored by someone else: defer-undo needs no author check.
            .append_query_results(vec![vec![test_topic(topic_id, session_id, other_user_id)]])
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
    assert_eq!(resp.status(), StatusCode::OK);
}

// Undoing a delete is author-only: the soft-deleted topic's author -> 200.
#[tokio::test]
async fn undo_extractor_ok_for_deleted_topic_by_author() {
    let session_id = Id::new_v4();
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
            // Soft-deleted topic authored by the caller -> author branch passes.
            .append_query_results(vec![vec![deleted_test_topic(
                topic_id, session_id, user.id,
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
    assert_eq!(resp.status(), StatusCode::OK);
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
