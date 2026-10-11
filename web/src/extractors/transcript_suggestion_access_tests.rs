use std::sync::Arc;

use crate::middleware::auth::require_auth;

use super::*;
use axum::{body::Body, middleware::from_fn};
use axum::{extract::Request, http::StatusCode, routing::get, Router};
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
        organization_id: fixture_organization_id(),
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn test_suggestion(
    suggestion_id: Id,
    coaching_session_id: Id,
) -> domain::transcript_suggestion::Model {
    let now = Utc::now();
    domain::transcript_suggestion::Model {
        id: suggestion_id,
        coaching_session_id,
        transcript_analysis_id: Id::new_v4(),
        target: domain::transcript_suggestion::Target::Action,
        proposal: serde_json::json!({}),
        segment_id: Id::new_v4(),
        quote: "a quote".to_string(),
        duplicate_of_id: None,
        status: domain::transcript_suggestion::Status::Pending,
        resolved_by: None,
        resolved_at: None,
        created_entity_id: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

async fn suggestion_route(
    TranscriptSuggestionAccess { suggestion }: TranscriptSuggestionAccess,
) -> String {
    format!("{}:{}", suggestion.coaching_session_id, suggestion.id)
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
                    "/coaching_sessions/:coaching_session_id/transcript_suggestions/:suggestion_id",
                    get(suggestion_route),
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

async fn get_suggestion(app: Router, session_id: Id, suggestion_id: Id) -> StatusCode {
    let cookie = do_login(&app).await;
    let req = Request::builder()
        .uri(format!(
            "/coaching_sessions/{session_id}/transcript_suggestions/{suggestion_id}"
        ))
        .method("GET")
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap().status()
}

#[tokio::test]
async fn participant_gets_the_suggestion() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let suggestion_id = Id::new_v4();
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
            .append_query_results(vec![vec![test_suggestion(suggestion_id, session_id)]])
            .into_connection(),
    );

    let status = get_suggestion(build_app(db), session_id, suggestion_id).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn suggestion_of_another_session_is_404() {
    let session_id = Id::new_v4();
    let other_session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let suggestion_id = Id::new_v4();
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
            .append_query_results(vec![vec![test_suggestion(suggestion_id, other_session_id)]])
            .into_connection(),
    );

    let status = get_suggestion(build_app(db), session_id, suggestion_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_suggestion_is_404() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
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
            .append_query_results(vec![Vec::<domain::transcript_suggestion::Model>::new()])
            .into_connection(),
    );

    let status = get_suggestion(build_app(db), session_id, Id::new_v4()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn non_participant_is_404() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let suggestion_id = Id::new_v4();
    let user = create_test_user();
    let role = test_role(user.id);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results(vec![vec![(
                test_session(session_id, relationship_id),
                test_relationship_with_coach(relationship_id, Id::new_v4(), Id::new_v4()),
            )]])
            .append_query_results(vec![vec![test_suggestion(suggestion_id, session_id)]])
            .into_connection(),
    );

    let status = get_suggestion(build_app(db), session_id, suggestion_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
