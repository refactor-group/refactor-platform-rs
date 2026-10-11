use std::sync::Arc;

use axum::http::StatusCode;
use axum::{body::Body, extract::Request, middleware::from_fn, routing::get, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::{NaiveDate, Utc};
use domain::transcript_suggestion::Target;
use domain::user::Backend;
use domain::{
    coaching_relationships, coaching_sessions, transcript_analysis, transcription, user_roles,
    users, Id,
};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase};
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use super::{scan, start_scan};
use crate::middleware::auth::require_auth;
use crate::AppState;

const SCAN_ROUTE: &str = "/coaching_sessions/:coaching_session_id/transcript_suggestions/scan";

fn user(email: &str) -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: email.to_string(),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        display_name: None,
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

fn role(user_id: Id, organization_id: Id) -> user_roles::Model {
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

/// A coach and coachee in one organization, with one session between them.
struct World {
    organization_id: Id,
    coach: users::Model,
    coachee: users::Model,
    relationship: coaching_relationships::Model,
    session: coaching_sessions::Model,
}

fn world() -> World {
    let now = Utc::now();
    let organization_id = Id::new_v4();
    let coach = user("coach@example.com");
    let coachee = user("coachee@example.com");
    let relationship = coaching_relationships::Model {
        id: Id::new_v4(),
        coach_id: coach.id,
        coachee_id: coachee.id,
        organization_id,
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    };
    let session = coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: relationship.id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: NaiveDate::from_ymd_opt(2026, 10, 12)
            .and_then(|date| date.and_hms_opt(10, 0, 0))
            .expect("the fixture date must be valid"),
        duration_minutes: domain::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: Some(now.into()),
        notice_given_at: now.into(),
    };
    World {
        organization_id,
        coach,
        coachee,
        relationship,
        session,
    }
}

fn completed_transcription(session_id: Id) -> transcription::Model {
    let now = Utc::now();
    transcription::Model {
        id: Id::new_v4(),
        coaching_session_id: session_id,
        meeting_recording_id: Id::new_v4(),
        external_id: "ext-1".to_string(),
        recall_recording_id: None,
        status: transcription::TranscriptionStatus::Completed,
        language_code: None,
        speaker_count: None,
        word_count: None,
        duration_seconds: None,
        confidence: None,
        error_message: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn completed_analysis(transcription_id: Id) -> transcript_analysis::Model {
    let now = Utc::now();
    transcript_analysis::Model {
        id: Id::new_v4(),
        transcription_id,
        target: Target::Action,
        status: transcript_analysis::Status::Completed,
        attempts: 1,
        claimed_at: Some(now.into()),
        error_message: None,
        model: Some("model".to_string()),
        input_tokens: Some(1),
        output_tokens: Some(1),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// The login and access rows for `user`, who holds `role`, reaching `world`'s session.
fn signed_in(user: &users::Model, role: &user_roles::Model, world: &World) -> MockDatabase {
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![(world.session.clone(), world.relationship.clone())]])
}

fn build_app(db: Arc<DatabaseConnection>) -> Router {
    let app_state = AppState::new(
        service::AppState::new(
            Config::from_args(["test", "--assembly-ai-api-key=test-key"]),
            &db,
        ),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
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
                .route(SCAN_ROUTE, get(scan).post(start_scan))
                .route_layer(from_fn(require_auth)),
        )
        .layer(auth_layer)
        .with_state(app_state)
}

async fn login_cookie(app: &Router, email: &str) -> String {
    let request = Request::builder()
        .uri("/login")
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("email={email}&password=password123")))
        .unwrap();
    app.clone()
        .oneshot(request)
        .await
        .unwrap()
        .headers()
        .get("set-cookie")
        .and_then(|cookie| cookie.to_str().ok())
        .expect("login should return a session cookie")
        .to_string()
}

async fn send(
    app: &Router,
    method: &str,
    cookie: Option<&str>,
    session_id: Id,
) -> axum::response::Response {
    let request = Request::builder()
        .method(method)
        .uri(format!(
            "/coaching_sessions/{session_id}/transcript_suggestions/scan"
        ))
        .header("x-version", "1.0.0");
    let request = match cookie {
        Some(cookie) => request.header("cookie", cookie),
        None => request,
    };
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Whether the database was asked to write anything.
fn wrote(db: &DatabaseConnection) -> bool {
    db.clone().into_transaction_log().iter().any(|t| {
        t.statements().iter().any(|s| {
            let sql = s.sql.trim_start().to_uppercase();
            sql.starts_with("INSERT") || sql.starts_with("UPDATE")
        })
    })
}

/// Sends `method` to the scan route as `outsider`, holding `outsider_role`; returns status and writes.
async fn as_outsider(
    method: &str,
    world: &World,
    outsider: &users::Model,
    outsider_role: &user_roles::Model,
) -> (StatusCode, bool) {
    let db = Arc::new(signed_in(outsider, outsider_role, world).into_connection());
    let app = build_app(db.clone());
    let cookie = login_cookie(&app, &outsider.email).await;

    let status = send(&app, method, Some(&cookie), world.session.id)
        .await
        .status();
    (status, wrote(&db))
}

#[tokio::test]
async fn a_user_outside_the_session_is_403_and_nothing_is_written() {
    for method in ["GET", "POST"] {
        let world = world();
        let outsider = user("outsider@example.com");
        let outsider_role = role(outsider.id, Id::new_v4());

        let (status, wrote) = as_outsider(method, &world, &outsider, &outsider_role).await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
        assert!(!wrote, "{method}");
    }
}

#[tokio::test]
async fn an_organization_member_outside_the_relationship_is_403_and_nothing_is_written() {
    for method in ["GET", "POST"] {
        let world = world();
        let member = user("member@example.com");
        let member_role = role(member.id, world.organization_id);

        let (status, wrote) = as_outsider(method, &world, &member, &member_role).await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
        assert!(!wrote, "{method}");
    }
}

#[tokio::test]
async fn an_unauthenticated_request_is_401() {
    for method in ["GET", "POST"] {
        let world = world();
        let app = build_app(Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres).into_connection(),
        ));

        let response = send(&app, method, None, world.session.id).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method}");
    }
}

#[tokio::test]
async fn the_coach_and_the_coachee_each_see_the_scan_state() {
    let world = world();
    for participant in [&world.coach, &world.coachee] {
        let participant_role = role(participant.id, world.organization_id);
        let transcription = completed_transcription(world.session.id);
        let db = signed_in(participant, &participant_role, &world)
            .append_query_results([vec![transcription.clone()]])
            .append_query_results([vec![completed_analysis(transcription.id)]])
            .into_connection();
        let app = build_app(Arc::new(db));
        let cookie = login_cookie(&app, &participant.email).await;

        let response = send(&app, "GET", Some(&cookie), world.session.id).await;

        assert_eq!(response.status(), StatusCode::OK, "{}", participant.email);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&bytes).expect("the body must be JSON");
        assert_eq!(body["data"], serde_json::json!({ "status": "completed" }));
    }
}
