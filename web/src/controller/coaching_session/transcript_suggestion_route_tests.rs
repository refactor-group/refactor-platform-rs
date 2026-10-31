//! The suggestion routes end to end through the router, sign-in and access checks, against a
//! mock database: who may call them, what they reject, and that they succeed for a participant.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    body::Body,
    extract::Request,
    middleware::from_fn,
    routing::{get, post},
    Router,
};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::{NaiveDate, Utc};
use domain::action::ActionWithAssignees;
use domain::transcript_suggestion::{self as suggestion, ActionProposal, Target};
use domain::user::Backend;
use domain::{
    actions, coaching_relationships, coaching_sessions, transcript_analysis, transcription,
    user_roles, users, Id,
};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase};
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use super::{accept, dismiss, index, restore, scan, start_scan};
use crate::middleware::auth::require_auth;
use crate::AppState;

const BASE: &str = "/coaching_sessions/:coaching_session_id/transcript_suggestions";

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
                .route(BASE, get(index))
                .route(&format!("{BASE}/scan"), get(scan).post(start_scan))
                .route(&format!("{BASE}/:suggestion_id/accept"), post(accept))
                .route(&format!("{BASE}/:suggestion_id/dismiss"), post(dismiss))
                .route(&format!("{BASE}/:suggestion_id/restore"), post(restore))
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

/// Sends `method` to `/coaching_sessions/{session_id}/transcript_suggestions{suffix}`.
async fn send(
    app: &Router,
    method: &str,
    cookie: Option<&str>,
    session_id: Id,
    suffix: &str,
    body: &str,
) -> axum::response::Response {
    let request = Request::builder()
        .method(method)
        .uri(format!(
            "/coaching_sessions/{session_id}/transcript_suggestions{suffix}"
        ))
        .header("x-version", "1.0.0")
        .header("content-type", "application/json");
    let request = match cookie {
        Some(cookie) => request.header("cookie", cookie),
        None => request,
    };
    app.clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).expect("the body must be JSON")
}

/// A pending suggestion in `world`'s session, assigned to the coachee.
fn pending_suggestion(world: &World) -> suggestion::Model {
    let now = Utc::now();
    let proposal = ActionProposal {
        action: ActionWithAssignees {
            action: actions::Model {
                id: Id::nil(),
                coaching_session_id: world.session.id,
                goal_id: None,
                user_id: Id::nil(),
                body: Some("Send the hiring plan".to_string()),
                due_by: Some(now.into()),
                status: domain::status::Status::NotStarted,
                status_changed_at: now.into(),
                created_at: now.into(),
                updated_at: now.into(),
            },
            assignee_ids: vec![world.coachee.id],
        },
        due_phrase: None,
    };
    suggestion::Model {
        id: Id::new_v4(),
        coaching_session_id: world.session.id,
        transcript_analysis_id: Id::new_v4(),
        target: Target::Action,
        proposal: serde_json::to_value(proposal).expect("the proposal serializes"),
        segment_id: Id::new_v4(),
        quote: "I'll send the hiring plan".to_string(),
        duplicate_of_id: None,
        status: suggestion::Status::Pending,
        resolved_by: None,
        resolved_at: None,
        created_entity_id: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// The coach, signed in, with `db` queued past the session access check.
async fn as_coach(
    world: &World,
    then: impl FnOnce(MockDatabase) -> MockDatabase,
) -> (Router, String, Arc<DatabaseConnection>) {
    let coach_role = role(world.coach.id, world.organization_id);
    let db = Arc::new(then(signed_in(&world.coach, &coach_role, world)).into_connection());
    let app = build_app(db.clone());
    let cookie = login_cookie(&app, &world.coach.email).await;
    (app, cookie, db)
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

/// The one UPDATE the database ran, with its values inlined.
fn only_update(db: &DatabaseConnection) -> String {
    let updates: Vec<String> = DatabaseConnection::clone(db)
        .into_transaction_log()
        .iter()
        .flat_map(|t| t.statements().iter().map(|s| s.to_string()))
        .filter(|sql| sql.trim_start().to_uppercase().starts_with("UPDATE"))
        .collect();
    assert_eq!(updates.len(), 1, "{updates:?}");
    updates[0].clone()
}

/// Every suggestion route, as (method, path suffix after the session's suggestions path).
fn every_route(suggestion_id: Id) -> Vec<(&'static str, String)> {
    vec![
        ("GET", String::new()),
        ("GET", "/scan".to_string()),
        ("POST", "/scan".to_string()),
        ("POST", format!("/{suggestion_id}/accept")),
        ("POST", format!("/{suggestion_id}/dismiss")),
        ("POST", format!("/{suggestion_id}/restore")),
    ]
}

/// How a route refuses a non-participant: a suggestion route hides the session behind a 404.
fn refused(suffix: &str) -> StatusCode {
    let on_a_suggestion = ["/accept", "/dismiss", "/restore"]
        .iter()
        .any(|verb| suffix.ends_with(verb));
    if on_a_suggestion {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::FORBIDDEN
    }
}

/// Sends `method` to `suffix` as `outsider`, holding `outsider_role`; returns status and writes.
async fn as_outsider(
    method: &str,
    suffix: &str,
    world: &World,
    outsider: &users::Model,
    outsider_role: &user_roles::Model,
) -> (StatusCode, bool) {
    let db = Arc::new(signed_in(outsider, outsider_role, world).into_connection());
    let app = build_app(db.clone());
    let cookie = login_cookie(&app, &outsider.email).await;

    let status = send(&app, method, Some(&cookie), world.session.id, suffix, "")
        .await
        .status();
    (status, wrote(&db))
}

#[tokio::test]
async fn a_user_outside_the_session_is_403_on_every_route_and_nothing_is_written() {
    for (method, suffix) in every_route(Id::new_v4()) {
        let world = world();
        let outsider = user("outsider@example.com");
        let outsider_role = role(outsider.id, Id::new_v4());

        let (status, wrote) = as_outsider(method, &suffix, &world, &outsider, &outsider_role).await;

        assert_eq!(status, refused(&suffix), "{method} {suffix}");
        assert!(!wrote, "{method} {suffix}");
    }
}

#[tokio::test]
async fn an_organization_member_outside_the_relationship_is_refused_on_every_route() {
    for (method, suffix) in every_route(Id::new_v4()) {
        let world = world();
        let member = user("member@example.com");
        let member_role = role(member.id, world.organization_id);

        let (status, wrote) = as_outsider(method, &suffix, &world, &member, &member_role).await;

        assert_eq!(status, refused(&suffix), "{method} {suffix}");
        assert!(!wrote, "{method} {suffix}");
    }
}

#[tokio::test]
async fn an_unauthenticated_request_is_401_on_every_route() {
    for (method, suffix) in every_route(Id::new_v4()) {
        let world = world();
        let app = build_app(Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres).into_connection(),
        ));

        let response = send(&app, method, None, world.session.id, &suffix, "").await;

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {suffix}"
        );
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

        let response = send(&app, "GET", Some(&cookie), world.session.id, "/scan", "").await;

        assert_eq!(response.status(), StatusCode::OK, "{}", participant.email);
        assert_eq!(
            json_body(response).await["data"],
            serde_json::json!({ "status": "completed" })
        );
    }
}

#[tokio::test]
async fn starting_a_scan_of_a_never_scanned_transcript_is_202_pending() {
    let world = world();
    let transcription = completed_transcription(world.session.id);
    let mut queued = completed_analysis(transcription.id);
    queued.status = transcript_analysis::Status::Pending;
    queued.attempts = 0;
    let (app, cookie, db) = as_coach(&world, |db| {
        db.append_query_results([vec![transcription.clone()]])
            .append_query_results([Vec::<transcript_analysis::Model>::new()])
            .append_query_results([vec![queued.clone()]])
    })
    .await;

    let response = send(&app, "POST", Some(&cookie), world.session.id, "/scan", "").await;

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        json_body(response).await["data"],
        serde_json::json!({ "status": "pending" })
    );
    assert!(wrote(&db), "the pending analysis is inserted");
}

#[tokio::test]
async fn listing_returns_the_sessions_suggestions_and_rejects_an_unknown_filter() {
    let world = world();
    let (app, cookie, _) = as_coach(&world, |db| {
        db.append_query_results([Vec::<suggestion::Model>::new()])
    })
    .await;
    let response = send(&app, "GET", Some(&cookie), world.session.id, "", "").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["data"], serde_json::json!([]));

    for query in ["?status=accepted", "?target=goal"] {
        let (app, cookie, db) = as_coach(&world, |db| db).await;
        let response = send(&app, "GET", Some(&cookie), world.session.id, query, "").await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query}"
        );
        assert!(!wrote(&db), "{query}");
    }
}

#[tokio::test]
async fn a_suggestion_from_another_session_is_404_on_accept_dismiss_and_restore() {
    for verb in ["accept", "dismiss", "restore"] {
        let world = world();
        let mut elsewhere = pending_suggestion(&world);
        elsewhere.coaching_session_id = Id::new_v4();
        let (app, cookie, db) = as_coach(&world, |db| {
            db.append_query_results([vec![elsewhere.clone()]])
        })
        .await;

        let suffix = format!("/{}/{verb}", elsewhere.id);
        let response = send(&app, "POST", Some(&cookie), world.session.id, &suffix, "").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{verb}");
        assert!(!wrote(&db), "{verb}");
    }
}

#[tokio::test]
async fn accepting_with_a_malformed_body_is_400_and_nothing_is_written() {
    let world = world();
    let pending = pending_suggestion(&world);
    let (app, cookie, db) = as_coach(&world, |db| {
        db.append_query_results([vec![pending.clone()]])
    })
    .await;

    let suffix = format!("/{}/accept", pending.id);
    let response = send(
        &app,
        "POST",
        Some(&cookie),
        world.session.id,
        &suffix,
        "{\"body\": ",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(!wrote(&db));
}

#[tokio::test]
async fn the_coach_dismisses_a_pending_suggestion_with_204() {
    let world = world();
    let pending = pending_suggestion(&world);
    let mut dismissed = pending.clone();
    dismissed.status = suggestion::Status::Dismissed;
    dismissed.resolved_by = Some(world.coach.id);
    let (app, cookie, db) = as_coach(&world, |db| {
        db.append_query_results([vec![pending.clone()]])
            .append_query_results([vec![(world.session.clone(), world.relationship.clone())]])
            .append_query_results([vec![dismissed.clone()]])
    })
    .await;

    let suffix = format!("/{}/dismiss", pending.id);
    let response = send(&app, "POST", Some(&cookie), world.session.id, &suffix, "").await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let update = only_update(&db);
    let (set, condition) = update
        .split_once(" WHERE ")
        .expect("the update is conditional");
    assert!(set.contains("'dismissed'"), "{update}");
    assert!(condition.contains("'pending'"), "{update}");
}
