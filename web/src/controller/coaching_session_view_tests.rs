use crate::controller::coaching_session_controller::view;
use crate::middleware::auth::require_auth;
use crate::AppState;
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, middleware::from_fn, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::{coaching_relationships, coaching_sessions, user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, MockDatabase, Value};
use service::config::Config;
use std::collections::BTreeMap;
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
        collab_document_name: None,
        date: Utc::now().naive_utc(),
        duration_minutes: domain::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: Some(now.into()),
    }
}

fn app_state(db: &Arc<sea_orm::DatabaseConnection>) -> AppState {
    AppState::new(
        service::AppState::new(Config::default(), db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
        None,
    )
}

fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
    let state = app_state(&db);
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
                .route("/coaching_sessions/:coaching_session_id/view", post(view))
                .route_layer(from_fn(require_auth)),
        )
        .layer(auth_layer)
        .with_state(state)
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

// Participant (coachee here) gets 200 and the marker is upserted.
#[tokio::test]
async fn view_returns_200_for_participant() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let user = test_user();
    let role = test_role(user.id);
    let now = Utc::now();

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                coaching_relationships::Model {
                    id: relationship_id,
                    coach_id: Id::new_v4(),
                    coachee_id: user.id,
                    organization_id: Id::new_v4(),
                    slug: "test".to_string(),
                    created_at: now.into(),
                    updated_at: now.into(),
                },
            )]])
            .append_query_results(vec![vec![BTreeMap::from([
                (
                    "previous_last_viewed_at".to_owned(),
                    Value::from(None::<chrono::DateTime<chrono::FixedOffset>>),
                ),
                ("last_viewed_at".to_owned(), Value::from(now.fixed_offset())),
            ])]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let request = Request::builder()
        .uri(format!("/coaching_sessions/{session_id}/view"))
        .method("POST")
        .header("cookie", cookie)
        .header("x-version", "1.0.0-beta1")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// Non-participant gets 403 from CoachingSessionAccess (mark_viewed never runs).
#[tokio::test]
async fn view_returns_403_for_non_participant() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let user = test_user();
    let role = test_role(user.id);
    let now = Utc::now();

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                coaching_relationships::Model {
                    id: relationship_id,
                    coach_id: Id::new_v4(),
                    coachee_id: Id::new_v4(),
                    organization_id: Id::new_v4(),
                    slug: "test".to_string(),
                    created_at: now.into(),
                    updated_at: now.into(),
                },
            )]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let request = Request::builder()
        .uri(format!("/coaching_sessions/{session_id}/view"))
        .method("POST")
        .header("cookie", cookie)
        .header("x-version", "1.0.0-beta1")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
