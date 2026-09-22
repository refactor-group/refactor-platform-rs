use std::sync::Arc;

use axum::http::StatusCode;
use axum::{body::Body, extract::Request, middleware::from_fn, routing::get, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::{NaiveDate, Utc};
use domain::user::Backend;
use domain::{
    coaching_relationships, coaching_sessions, transcript_segment, transcription, user_roles,
    users, Id,
};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, MockDatabase};
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use super::read;
use crate::middleware::auth::require_auth;
use crate::AppState;

const ROUTE: &str = "/coaching_sessions/:coaching_session_id/transcriptions/:transcription_id";

fn coach() -> users::Model {
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

fn coachee() -> users::Model {
    users::Model {
        id: Id::new_v4(),
        email: "caleb@example.com".to_string(),
        first_name: "Caleb".to_string(),
        last_name: "Bourg".to_string(),
        display_name: None,
        ..coach()
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

fn relationship(
    organization_id: Id,
    coach_id: Id,
    coachee_id: Id,
) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id: Id::new_v4(),
        coach_id,
        coachee_id,
        organization_id,
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn session(relationship_id: Id) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: relationship_id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: NaiveDate::from_ymd_opt(2026, 9, 21)
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
    }
}

fn transcription(
    session_id: Id,
    status: transcription::TranscriptionStatus,
) -> transcription::Model {
    let now = Utc::now();
    transcription::Model {
        id: Id::new_v4(),
        coaching_session_id: session_id,
        meeting_recording_id: Id::new_v4(),
        external_id: "ext-1".to_string(),
        recall_recording_id: Some("recall-1".to_string()),
        status,
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

fn segment(
    transcription_id: Id,
    label: &str,
    text: &str,
    start_ms: i32,
) -> transcript_segment::Model {
    transcript_segment::Model {
        id: Id::new_v4(),
        transcription_id,
        speaker_label: label.to_string(),
        text: text.to_string(),
        start_ms,
        end_ms: start_ms + 2000,
        confidence: None,
        sentiment: None,
        created_at: Utc::now().into(),
    }
}

fn segments(transcription_id: Id) -> Vec<transcript_segment::Model> {
    vec![
        segment(transcription_id, "Test User", "Good morning.", 0),
        segment(transcription_id, "Caleb Bourg", "Morning.", 4000),
        segment(transcription_id, "Guest", "Hi both.", 9000),
    ]
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
                .route(ROUTE, get(read))
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
        .and_then(|cookie| cookie.to_str().ok())
        .expect("login should return a session cookie")
        .to_string()
}

/// The auth and access rows every protected request consumes before the handler runs.
fn authorized(
    user: &users::Model,
    role: &user_roles::Model,
    session: &coaching_sessions::Model,
    relationship: &coaching_relationships::Model,
) -> MockDatabase {
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![(session.clone(), relationship.clone())]])
}

async fn get_transcript(
    app: &Router,
    cookie: &str,
    session_id: Id,
    transcription_id: Id,
    accept: Option<&str>,
    query: &str,
) -> axum::response::Response {
    let request = Request::builder()
        .uri(format!(
            "/coaching_sessions/{session_id}/transcriptions/{transcription_id}{query}"
        ))
        .header("cookie", cookie)
        .header("x-version", "1.0.0-beta1");
    let request = match accept {
        Some(accept) => request.header("accept", accept),
        None => request,
    };
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).expect("the body must be valid UTF-8")
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_str(&body_string(response).await).expect("the body must be JSON")
}

fn header(response: &axum::response::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// The full happy-path world: authorized rows plus the handler's five queries.
struct World {
    app: Router,
    session_id: Id,
    transcription_id: Id,
}

async fn completed_world() -> (World, String) {
    world(transcription::TranscriptionStatus::Completed, true, None).await
}

/// Builds an app for one request. `with_segments` controls whether the transcript has
/// lines; `coachee_override` replaces the coachee fixture.
async fn world(
    status: transcription::TranscriptionStatus,
    with_segments: bool,
    coachee_override: Option<users::Model>,
) -> (World, String) {
    let organization_id = Id::new_v4();
    let coach = coach();
    let coachee = coachee_override.unwrap_or_else(coachee);
    let role = role(coach.id, organization_id);
    let relationship = relationship(organization_id, coach.id, coachee.id);
    let session = session(relationship.id);
    let transcription = transcription(session.id, status);

    let lines = if with_segments {
        segments(transcription.id)
    } else {
        vec![]
    };

    let db = authorized(&coach, &role, &session, &relationship)
        .append_query_results([vec![transcription.clone()]])
        .append_query_results([lines])
        .append_query_results([vec![relationship.clone()]])
        .append_query_results([vec![coach.clone()]])
        .append_query_results([vec![coachee.clone()]])
        .into_connection();

    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    (
        World {
            app,
            session_id: session.id,
            transcription_id: transcription.id,
        },
        cookie,
    )
}

#[tokio::test]
async fn no_accept_returns_json_with_speakers() {
    let (world, cookie) = completed_world().await;

    let response = get_transcript(
        &world.app,
        &cookie,
        world.session_id,
        world.transcription_id,
        None,
        "",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(header(&response, "content-type").starts_with("application/json"));
    assert_eq!(header(&response, "vary"), "accept");

    let transcription_id = world.transcription_id;
    let body = body_json(response).await;
    assert_eq!(body["data"]["id"], serde_json::json!(transcription_id));
    assert_eq!(
        body["data"]["speakers"],
        serde_json::json!([
            {"label": "Test User", "role": "coach"},
            {"label": "Caleb Bourg", "role": "coachee"},
            {"label": "Guest", "role": null},
        ])
    );
    assert!(
        body["data"].get("recall_recording_id").is_none(),
        "the internal correlation id must not reach a client"
    );
}

#[tokio::test]
async fn text_plain_returns_the_file_with_both_headers() {
    let (world, cookie) = completed_world().await;

    let response = get_transcript(
        &world.app,
        &cookie,
        world.session_id,
        world.transcription_id,
        Some("text/plain"),
        "",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(&response, "content-type"),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        header(&response, "content-disposition"),
        "attachment; filename=\"transcript-2026-09-21.txt\""
    );
    assert_eq!(header(&response, "vary"), "accept");
    assert!(body_string(response).await.starts_with(
        "Coaching session transcript\nDate: 2026-09-21\nSpeakers: Test User, Caleb Bourg, Guest\n\n[0:00] Test User: Good morning.\n"
    ));
}

#[tokio::test]
async fn speaker_filter_narrows_the_file_and_marks_the_filename() {
    let (world, cookie) = completed_world().await;

    let response = get_transcript(
        &world.app,
        &cookie,
        world.session_id,
        world.transcription_id,
        Some("text/plain"),
        "?speaker=coach&speaker=coachee",
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(header(&response, "content-disposition").ends_with("-filtered.txt\""));

    let body = body_string(response).await;
    assert!(body.contains("Speakers: Test User, Caleb Bourg\n"));
    assert!(!body.contains("Guest"));
}

#[tokio::test]
async fn unsupported_accept_is_406() {
    let organization_id = Id::new_v4();
    let coach = coach();
    let role = role(coach.id, organization_id);
    let coachee = coachee();
    let relationship = relationship(organization_id, coach.id, coachee.id);
    let session = session(relationship.id);

    // Only the auth and access rows: a handler that queried anything would 500.
    let db = authorized(&coach, &role, &session, &relationship).into_connection();
    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    let response = get_transcript(
        &app,
        &cookie,
        session.id,
        Id::new_v4(),
        Some("image/png"),
        "",
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(header(&response, "vary"), "accept");
}

#[tokio::test]
async fn wildcard_and_json_accept_select_json() {
    for accept in [
        "*/*",
        "application/json",
        "text/html, application/json;q=0.9",
    ] {
        let (world, cookie) = completed_world().await;

        let response = get_transcript(
            &world.app,
            &cookie,
            world.session_id,
            world.transcription_id,
            Some(accept),
            "",
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK, "accept: {accept}");
        assert!(
            header(&response, "content-type").starts_with("application/json"),
            "accept: {accept}"
        );
    }
}

#[tokio::test]
async fn quality_values_pick_the_preferred_supported_type() {
    for (accept, content_type) in [
        ("application/json;q=0, text/plain", "text/plain"),
        (
            "text/plain;q=0.5, application/json;q=0.9",
            "application/json",
        ),
        ("text/plain; q=0.9, */*; q=0.8", "text/plain"),
        (
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "application/json",
        ),
    ] {
        let (world, cookie) = completed_world().await;

        let response = get_transcript(
            &world.app,
            &cookie,
            world.session_id,
            world.transcription_id,
            Some(accept),
            "",
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK, "accept: {accept}");
        assert!(
            header(&response, "content-type").starts_with(content_type),
            "accept: {accept}"
        );
    }
}

#[tokio::test]
async fn a_zero_quality_on_every_supported_type_is_406() {
    let organization_id = Id::new_v4();
    let coach = coach();
    let role = role(coach.id, organization_id);
    let coachee = coachee();
    let relationship = relationship(organization_id, coach.id, coachee.id);
    let session = session(relationship.id);

    let db = authorized(&coach, &role, &session, &relationship).into_connection();
    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    let response = get_transcript(
        &app,
        &cookie,
        session.id,
        Id::new_v4(),
        Some("text/plain;q=0, image/png"),
        "",
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn unknown_speaker_is_400_for_json_too() {
    let organization_id = Id::new_v4();
    let coach = coach();
    let role = role(coach.id, organization_id);
    let coachee = coachee();
    let relationship = relationship(organization_id, coach.id, coachee.id);
    let session = session(relationship.id);

    let db = authorized(&coach, &role, &session, &relationship).into_connection();
    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    let response = get_transcript(
        &app,
        &cookie,
        session.id,
        Id::new_v4(),
        None,
        "?speaker=bob",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = body_json(response).await;
    assert_eq!(body["error"], "invalid_speaker");
    assert!(body["message"].as_str().unwrap_or_default().contains("bob"));
}

#[tokio::test]
async fn invalid_speaker_message_names_only_the_speaker_values() {
    let organization_id = Id::new_v4();
    let coach = coach();
    let role = role(coach.id, organization_id);
    let coachee = coachee();
    let relationship = relationship(organization_id, coach.id, coachee.id);
    let session = session(relationship.id);

    let db = authorized(&coach, &role, &session, &relationship).into_connection();
    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    let response = get_transcript(
        &app,
        &cookie,
        session.id,
        Id::new_v4(),
        Some("text/plain"),
        "?speaker=Jim%20H&speaker=bogus&foo=1",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = body_json(response).await;
    let message = body["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("'Jim H, bogus'"), "{message}");
    assert!(!message.contains("%20"), "{message}");
    assert!(!message.contains("foo"), "{message}");
}

#[tokio::test]
async fn transcription_under_another_session_is_404() {
    let organization_id = Id::new_v4();
    let coach = coach();
    let role = role(coach.id, organization_id);
    let coachee = coachee();
    let relationship = relationship(organization_id, coach.id, coachee.id);
    let session = session(relationship.id);
    let elsewhere = transcription(Id::new_v4(), transcription::TranscriptionStatus::Completed);

    let db = authorized(&coach, &role, &session, &relationship)
        .append_query_results([vec![elsewhere.clone()]])
        .into_connection();
    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    let response = get_transcript(
        &app,
        &cookie,
        session.id,
        elsewhere.id,
        Some("text/plain"),
        "",
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(response).await["error"],
        "transcription_not_found"
    );
}

#[tokio::test]
async fn incomplete_transcription_is_409_for_text_but_200_for_json() {
    let organization_id = Id::new_v4();
    let coach = coach();
    let role = role(coach.id, organization_id);
    let coachee = coachee();
    let relationship = relationship(organization_id, coach.id, coachee.id);
    let session = session(relationship.id);
    let queued = transcription(session.id, transcription::TranscriptionStatus::Queued);

    let db = authorized(&coach, &role, &session, &relationship)
        .append_query_results([vec![queued.clone()]])
        .into_connection();
    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    let response =
        get_transcript(&app, &cookie, session.id, queued.id, Some("text/plain"), "").await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(response).await["error"],
        "transcription_not_completed"
    );

    // The same rows read as JSON: readable at any status, with no speakers yet.
    let db = authorized(&coach, &role, &session, &relationship)
        .append_query_results([vec![queued.clone()]])
        .append_query_results([Vec::<transcript_segment::Model>::new()])
        .append_query_results([vec![relationship.clone()]])
        .append_query_results([vec![coach.clone()]])
        .append_query_results([vec![coachee.clone()]])
        .into_connection();
    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    let response = get_transcript(&app, &cookie, session.id, queued.id, None, "").await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    assert_eq!(body["data"]["status"], "queued");
    assert_eq!(body["data"]["speakers"], serde_json::json!([]));
}

#[tokio::test]
async fn unmatched_participant_is_422() {
    let stranger = users::Model {
        first_name: "Nobody".to_string(),
        last_name: "Here".to_string(),
        ..coachee()
    };
    let (world, cookie) = world(
        transcription::TranscriptionStatus::Completed,
        true,
        Some(stranger),
    )
    .await;

    let response = get_transcript(
        &world.app,
        &cookie,
        world.session_id,
        world.transcription_id,
        Some("text/plain"),
        "?speaker=coachee",
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = body_json(response).await;
    assert_eq!(body["error"], "speaker_not_identified");

    let message = body["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("coachee"), "{message}");
    assert!(message.contains("Guest"), "{message}");
}

#[tokio::test]
async fn non_participant_is_403() {
    let organization_id = Id::new_v4();
    let coach = coach();
    let role = role(coach.id, organization_id);
    let others = relationship(organization_id, Id::new_v4(), Id::new_v4());
    let session = session(others.id);

    let db = authorized(&coach, &role, &session, &others).into_connection();
    let app = build_app(Arc::new(db));
    let cookie = login_cookie(&app).await;

    let response = get_transcript(
        &app,
        &cookie,
        session.id,
        Id::new_v4(),
        Some("text/plain"),
        "",
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_string(response).await, "FORBIDDEN");
}
