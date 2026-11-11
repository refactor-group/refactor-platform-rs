//! The note routes through the router, sign-in and access checks, against a mock database:
//! only a participant of the note's relationship may read, create or change it.

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
use chrono::Utc;
use domain::user::Backend;
use domain::{coaching_relationships, coaching_sessions, notes, user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase};
use serde_json::json;
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use crate::controller::note_controller::{create, read, update};
use crate::middleware::auth::require_auth;
use crate::AppState;

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
        working_hours_start: domain::working_hours::default_start(),
        working_hours_end: domain::working_hours::default_end(),
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

/// A relationship with one session holding one note, plus callers outside it.
struct World {
    organization_id: Id,
    coach: users::Model,
    coachee: users::Model,
    member: users::Model,
    outsider: users::Model,
    relationship: coaching_relationships::Model,
    session: coaching_sessions::Model,
    note: notes::Model,
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
        date: now.naive_utc(),
        duration_minutes: domain::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: Some(now.into()),
        notice_given_at: now.into(),
    };
    let note = notes::Model {
        id: Id::new_v4(),
        coaching_session_id: session.id,
        body: Some("Talked through the plan".to_string()),
        user_id: coach.id,
        created_at: now.into(),
        updated_at: now.into(),
    };
    World {
        organization_id,
        coach,
        coachee,
        member: user("member@example.com"),
        outsider: user("outsider@example.com"),
        relationship,
        session,
        note,
    }
}

#[derive(Clone, Copy, Debug)]
enum Caller {
    Coach,
    Coachee,
    /// In the relationship's organization, not in the relationship.
    Member,
    /// In another organization only.
    Outsider,
    /// The coach, with no role in the relationship's organization.
    Removed,
}

impl World {
    fn caller(&self, caller: Caller) -> &users::Model {
        match caller {
            Caller::Coach | Caller::Removed => &self.coach,
            Caller::Coachee => &self.coachee,
            Caller::Member => &self.member,
            Caller::Outsider => &self.outsider,
        }
    }

    /// The login row and the per-request user row for `caller`.
    fn signed_in(&self, caller: Caller) -> MockDatabase {
        let user = self.caller(caller).clone();
        let db = MockDatabase::new(DatabaseBackend::Postgres);
        match caller {
            Caller::Removed => db
                .append_query_results([vec![(user.clone(), None::<user_roles::Model>)]])
                .append_query_results([vec![(user, None::<user_roles::Model>)]]),
            Caller::Outsider => {
                let role = role(user.id, Id::new_v4());
                db.append_query_results([vec![(user.clone(), role.clone())]])
                    .append_query_results([vec![(user, role)]])
            }
            _ => {
                let role = role(user.id, self.organization_id);
                db.append_query_results([vec![(user.clone(), role.clone())]])
                    .append_query_results([vec![(user, role)]])
            }
        }
    }

    /// `caller` signed in, with the note and its session queued for `NoteAccess`.
    fn reaching_note(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![self.note.clone()]])
            .append_query_results([vec![(self.session.clone(), self.relationship.clone())]])
    }

    /// `caller` signed in, with the session queued for the create's session check.
    fn reaching_session(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![(self.session.clone(), self.relationship.clone())]])
    }

    fn body(&self, coaching_session_id: Id) -> serde_json::Value {
        json!({
            "coaching_session_id": coaching_session_id,
            "body": "Revised the plan",
        })
    }
}

fn build_app(db: Arc<DatabaseConnection>) -> Router {
    let app_state = AppState::new(
        service::AppState::new(Config::default(), &db),
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
                .route("/notes", post(create))
                .route("/notes/:id", get(read).put(update))
                .route_layer(from_fn(require_auth)),
        )
        .layer(auth_layer)
        .with_state(app_state)
}

/// Signs `email` in, then sends `method uri` with `body` as JSON.
async fn send(
    db: &Arc<DatabaseConnection>,
    email: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let app = build_app(db.clone());
    let login = Request::builder()
        .uri("/login")
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("email={email}&password=password123")))
        .unwrap();
    let cookie = app
        .clone()
        .oneshot(login)
        .await
        .unwrap()
        .headers()
        .get("set-cookie")
        .and_then(|cookie| cookie.to_str().ok())
        .expect("login should return a session cookie")
        .to_string();

    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .header("x-version", "1.0.0")
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .unwrap();
    app.oneshot(request).await.unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).expect("the body must be JSON")
}

/// Whether the database was asked to write anything.
fn wrote(db: &DatabaseConnection) -> bool {
    db.clone().into_transaction_log().iter().any(|t| {
        t.statements().iter().any(|s| {
            let sql = s.sql.trim_start().to_uppercase();
            ["INSERT", "UPDATE", "DELETE"]
                .iter()
                .any(|verb| sql.starts_with(verb))
        })
    })
}

/// Sends the request and asserts the refusal `expected` with nothing written.
async fn assert_refused(
    row: &str,
    db: MockDatabase,
    email: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    expected: StatusCode,
) {
    let db = Arc::new(db.into_connection());
    let response = send(&db, email, method, uri, body).await;
    assert_eq!(response.status(), expected, "{row}");
    assert!(
        !wrote(&db),
        "{row}: the refused request wrote to the database"
    );
}

#[tokio::test]
async fn note_04_coach_reads_the_note() {
    let w = world();
    let db = Arc::new(w.reaching_note(Caller::Coach).into_connection());
    let response = send(
        &db,
        &w.coach.email,
        "GET",
        &format!("/notes/{}", w.note.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "NOTE-04");
    let body = json_body(response).await;
    assert_eq!(body["data"]["id"], json!(w.note.id), "NOTE-04");
}

#[tokio::test]
async fn note_07_member_cannot_read_the_note() {
    let w = world();
    assert_refused(
        "NOTE-07",
        w.reaching_note(Caller::Member),
        &w.member.email,
        "GET",
        &format!("/notes/{}", w.note.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn note_07b_unknown_note_is_not_found() {
    let w = world();
    assert_refused(
        "NOTE-07b",
        w.signed_in(Caller::Coach)
            .append_query_results([Vec::<notes::Model>::new()]),
        &w.coach.email,
        "GET",
        &format!("/notes/{}", Id::new_v4()),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn note_10_participant_removed_from_the_organization_cannot_read_the_note() {
    let w = world();
    assert_refused(
        "NOTE-10",
        w.reaching_note(Caller::Removed),
        &w.coach.email,
        "GET",
        &format!("/notes/{}", w.note.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn note_05_coachee_creates_a_note_in_the_session() {
    let w = world();
    let db = Arc::new(
        w.reaching_session(Caller::Coachee)
            // The INSERT ... RETURNING.
            .append_query_results([vec![w.note.clone()]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coachee.email,
        "POST",
        "/notes",
        Some(w.body(w.session.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "NOTE-05");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "NOTE-05");
}

#[tokio::test]
async fn note_08_outsider_cannot_create_a_note_in_the_session() {
    let w = world();
    assert_refused(
        "NOTE-08",
        w.reaching_session(Caller::Outsider),
        &w.outsider.email,
        "POST",
        "/notes",
        Some(w.body(w.session.id)),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn note_08b_unknown_session_is_not_found() {
    let w = world();
    assert_refused(
        "NOTE-08b",
        w.signed_in(Caller::Coach).append_query_results([Vec::<(
            coaching_sessions::Model,
            coaching_relationships::Model,
        )>::new()]),
        &w.coach.email,
        "POST",
        "/notes",
        Some(w.body(Id::new_v4())),
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn note_06_coach_updates_the_note() {
    let w = world();
    let db = Arc::new(
        w.reaching_note(Caller::Coach)
            // The read, then the UPDATE ... RETURNING.
            .append_query_results([vec![w.note.clone()]])
            .append_query_results([vec![w.note.clone()]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "PUT",
        &format!("/notes/{}", w.note.id),
        Some(w.body(w.session.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "NOTE-06");
}

#[tokio::test]
async fn note_09_member_cannot_update_the_note() {
    let w = world();
    assert_refused(
        "NOTE-09",
        w.reaching_note(Caller::Member),
        &w.member.email,
        "PUT",
        &format!("/notes/{}", w.note.id),
        Some(w.body(w.session.id)),
        StatusCode::FORBIDDEN,
    )
    .await;
}
