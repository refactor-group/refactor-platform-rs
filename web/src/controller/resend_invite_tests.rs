//! `POST /coaching_session_series/:id/resend_invite` and
//! `POST /coaching_sessions/:id/resend_invite`, driven through the real routers with a
//! logged-in session, a mock database, and a mock Resend.

use std::sync::{Arc, Mutex};

use axum::{body::Body, extract::Request, http::StatusCode, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{NaiveDateTime, Utc};
use domain::user::Backend;
use domain::{
    actions, coaching_relationships, coaching_session_series, coaching_session_topics,
    coaching_sessions, goals, organizations, user_roles, users, Id,
};
use mockito::{Mock, Server, ServerGuard};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase};
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use crate::controller::user_session_controller;
use crate::router::{coaching_session_series_routes, coaching_sessions_routes};
use crate::AppState;

const COACH_EMAIL: &str = "coach@example.com";
const COACHEE_EMAIL: &str = "coachee@example.com";

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

/// A coach and coachee in one organization, and the relationship between them.
struct Fixture {
    coach: users::Model,
    coachee: users::Model,
    organization: organizations::Model,
    relationship: coaching_relationships::Model,
}

impl Fixture {
    fn new() -> Self {
        let now = Utc::now();
        let coach = user(COACH_EMAIL);
        let coachee = user(COACHEE_EMAIL);
        let organization = organizations::Model {
            id: Id::new_v4(),
            name: "Acme".to_string(),
            logo: None,
            slug: "acme".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
            archived_at: None,
            archived_by: None,
        };
        let relationship = coaching_relationships::Model {
            id: Id::new_v4(),
            organization_id: organization.id,
            coach_id: coach.id,
            coachee_id: coachee.id,
            slug: "test".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        };
        Self {
            coach,
            coachee,
            organization,
            relationship,
        }
    }

    fn role(&self, user: &users::Model) -> user_roles::Model {
        let now = Utc::now();
        user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(self.organization.id),
            user_id: user.id,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    /// The login and `require_auth` loads for `caller`.
    fn logged_in_as(&self, caller: &users::Model) -> MockDatabase {
        let row = (caller.clone(), self.role(caller));
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![row.clone()]])
            .append_query_results([vec![row]])
    }

    /// The loads `InviteParties::load` makes.
    fn with_parties(&self, db: MockDatabase) -> MockDatabase {
        db.append_query_results(vec![vec![self.relationship.clone()]])
            .append_query_results(vec![vec![self.coach.clone()]])
            .append_query_results(vec![vec![self.coachee.clone()]])
            .append_query_results(vec![vec![self.organization.clone()]])
    }

    fn series(
        &self,
        start_at: NaiveDateTime,
        ical_sequence: i32,
    ) -> coaching_session_series::Model {
        let now = Utc::now();
        coaching_session_series::Model {
            id: Id::new_v4(),
            coaching_relationship_id: self.relationship.id,
            rule: serde_json::json!({
                "start_at": start_at,
                "recurrence": { "frequency": "weekly", "interval": 1, "count": 3 },
                "duration_minutes": 60,
            }),
            ical_sequence,
            created_by_user_id: self.coach.id,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn session(&self, date: NaiveDateTime) -> coaching_sessions::Model {
        let now = Utc::now();
        coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: self.relationship.id,
            coaching_session_series_id: None,
            ical_sequence: 2,
            ical_recurrence_id: None,
            collab_document_name: None,
            date,
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
            notice_given_at: now.into(),
        }
    }

    /// One member per slot of `series`, each in the slot it was created for.
    fn members(&self, series: &coaching_session_series::Model) -> Vec<coaching_sessions::Model> {
        let start_at: NaiveDateTime =
            serde_json::from_value(series.rule["start_at"].clone()).unwrap();
        (0..3)
            .map(|week| start_at + chrono::Duration::weeks(week))
            .map(|slot| coaching_sessions::Model {
                coaching_session_series_id: Some(series.id),
                ical_recurrence_id: Some(slot),
                ..self.session(slot)
            })
            .collect()
    }
}

/// A whole-hour start a week out, so it is upcoming and minute-aligned.
fn upcoming_start() -> NaiveDateTime {
    (Utc::now() + chrono::Duration::days(7))
        .date_naive()
        .and_hms_opt(15, 0, 0)
        .unwrap()
}

fn past_start() -> NaiveDateTime {
    (Utc::now() - chrono::Duration::days(30))
        .date_naive()
        .and_hms_opt(15, 0, 0)
        .unwrap()
}

fn config(resend_url: &str) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        "--session-scheduled-email-template-id=session_template",
        "--recurring-sessions-scheduled-email-template-id=series_template",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={resend_url}"),
    ])
}

/// Logs `caller_email` in over the real routers, then sends the resend `POST`.
async fn post_resend(
    db: Arc<DatabaseConnection>,
    config: Config,
    caller_email: &str,
    uri: String,
) -> axum::response::Response {
    let app_state = AppState::new(
        service::AppState::new(config, &db),
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

    let app = Router::new()
        .route("/login", post(user_session_controller::login))
        .with_state(app_state.clone())
        .merge(coaching_sessions_routes(app_state.clone()))
        .merge(coaching_session_series_routes(app_state))
        .layer(auth_layer);

    let login = Request::builder()
        .uri("/login")
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!(
            "email={caller_email}&password=password123"
        )))
        .unwrap();
    let cookie = app
        .clone()
        .oneshot(login)
        .await
        .unwrap()
        .headers()
        .get("set-cookie")
        .and_then(|c| c.to_str().ok())
        .expect("login should set a session cookie")
        .to_string();

    let request = Request::builder()
        .uri(uri)
        .method("POST")
        .header("cookie", cookie)
        .header("x-version", "1.0.0")
        .body(Body::empty())
        .unwrap();
    app.oneshot(request).await.unwrap()
}

async fn conflict_reason(response: axum::response::Response) -> String {
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    body["details"]["reason"].as_str().unwrap().to_string()
}

/// Whether any statement issued updates `table`'s `ical_sequence`.
fn bumped_sequence_of(db: DatabaseConnection, table: &str) -> bool {
    let target = format!(r#"UPDATE "refactor_platform"."{table}""#);
    db.into_transaction_log()
        .iter()
        .flat_map(|txn| txn.statements())
        .any(|stmt| stmt.sql.contains(&target) && stmt.sql.contains("ical_sequence"))
}

/// The unfolded `.ics` attached to a Resend request, if any.
fn attached_ics(request: &mockito::Request) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_slice(request.body().ok()?).ok()?;
    let content = payload["attachments"][0]["content"].as_str()?;
    let ics = String::from_utf8(STANDARD.decode(content).ok()?).ok()?;
    Some(ics.replace("\r\n ", "").replace("\r\n\t", ""))
}

/// Every `.ics` the Resend mock received, in order.
#[derive(Clone, Default)]
struct Received(Arc<Mutex<Vec<String>>>);

impl Received {
    fn recorder(&self) -> impl Fn(&mockito::Request) -> bool + Send + Sync + 'static {
        let received = self.0.clone();
        move |request| {
            received
                .lock()
                .unwrap()
                .push(attached_ics(request).unwrap_or_default());
            true
        }
    }

    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// One recipient's send, told apart by the `to` address.
async fn mock_send_to(
    server: &mut ServerGuard,
    email: &str,
    status: usize,
    received: &Received,
) -> Mock {
    server
        .mock("POST", "/emails")
        .match_body(mockito::Matcher::Regex(format!(r#""to":\[[^\]]*{email}"#)))
        .match_request(received.recorder())
        .with_status(status)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await
}

/// The VEVENT carrying the RRULE.
fn master_vevent(ics: &str) -> &str {
    ics.split("BEGIN:VEVENT")
        .skip(1)
        .find(|event| event.contains("RRULE"))
        .expect("the snapshot has a master VEVENT")
}

// ── Series ─────────────────────────────────────────────────────────

#[tokio::test]
async fn series_resend_forbids_the_coachee_and_sends_nothing() {
    let mut server = Server::new_async().await;
    let no_send = server
        .mock("POST", "/emails")
        .expect(0)
        .create_async()
        .await;
    let fixture = Fixture::new();
    let series = fixture.series(upcoming_start(), 3);

    let db = Arc::new(
        fixture
            .logged_in_as(&fixture.coachee)
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![vec![fixture.relationship.clone()]])
            .into_connection(),
    );

    let response = post_resend(
        db,
        config(&server.url()),
        COACHEE_EMAIL,
        format!("/coaching_session_series/{}/resend_invite", series.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    no_send.assert_async().await;
}

#[tokio::test]
async fn series_resend_refuses_a_series_with_nothing_upcoming() {
    let server = Server::new_async().await;
    let fixture = Fixture::new();
    let series = fixture.series(past_start(), 3);
    let rows = fixture.members(&series);

    let db = Arc::new(
        fixture
            .logged_in_as(&fixture.coach)
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![vec![fixture.relationship.clone()]])
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![rows])
            .into_connection(),
    );

    let response = post_resend(
        db.clone(),
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_session_series/{}/resend_invite", series.id),
    )
    .await;

    assert_eq!(conflict_reason(response).await, "no_upcoming_sessions");
    let db = Arc::try_unwrap(db).expect("the app has been dropped");
    assert!(!bumped_sequence_of(db, "coaching_session_series"));
}

#[tokio::test]
async fn series_resend_refuses_a_legacy_series() {
    let server = Server::new_async().await;
    let fixture = Fixture::new();
    let series = fixture.series(upcoming_start(), 3);
    let mut rows = fixture.members(&series);
    rows[1].ical_recurrence_id = None;

    let db = Arc::new(
        fixture
            .logged_in_as(&fixture.coach)
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![vec![fixture.relationship.clone()]])
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![rows])
            .into_connection(),
    );

    let response = post_resend(
        db,
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_session_series/{}/resend_invite", series.id),
    )
    .await;

    assert_eq!(conflict_reason(response).await, "legacy_series");
}

/// The series loads up to the send: the series twice (gate, then domain), its rows, the
/// bump, the rows raised to the bump, the parties, then the in-progress goals the
/// description lists.
fn series_send_db(
    fixture: &Fixture,
    series: &coaching_session_series::Model,
    rows: Vec<coaching_sessions::Model>,
    bumped: &coaching_session_series::Model,
) -> Arc<DatabaseConnection> {
    let raised: Vec<coaching_sessions::Model> = rows
        .iter()
        .map(|row| coaching_sessions::Model {
            ical_sequence: row.ical_sequence.max(bumped.ical_sequence),
            ..row.clone()
        })
        .collect();
    let db = fixture
        .logged_in_as(&fixture.coach)
        .append_query_results(vec![vec![series.clone()]])
        .append_query_results(vec![vec![fixture.relationship.clone()]])
        .append_query_results(vec![vec![series.clone()]])
        .append_query_results(vec![rows])
        .append_query_results(vec![vec![bumped.clone()]])
        .append_query_results(vec![raised]);
    Arc::new(
        fixture
            .with_parties(db)
            .append_query_results::<goals::Model, _, _>(vec![vec![]])
            .into_connection(),
    )
}

#[tokio::test]
async fn series_resend_sends_the_snapshot_under_the_next_sequence() {
    let mut server = Server::new_async().await;
    let fixture = Fixture::new();
    let series = fixture.series(upcoming_start(), 3);
    let bumped = coaching_session_series::Model {
        ical_sequence: 4,
        ..series.clone()
    };
    let rows = fixture.members(&series);
    let received = Received::default();
    let to_coachee = mock_send_to(&mut server, COACHEE_EMAIL, 200, &received).await;
    let to_coach = mock_send_to(&mut server, COACH_EMAIL, 200, &received).await;

    let db = series_send_db(&fixture, &series, rows, &bumped);
    let response = post_resend(
        db.clone(),
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_session_series/{}/resend_invite", series.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    to_coachee.assert_async().await;
    to_coach.assert_async().await;
    let received = received.all();
    assert_eq!(received.len(), 2);
    for ics in &received {
        assert!(master_vevent(ics).contains("SEQUENCE:4"), "{ics}");
    }
    let db = Arc::try_unwrap(db).expect("the app has been dropped");
    assert!(bumped_sequence_of(db, "coaching_session_series"));
}

/// Google drops an override whose SEQUENCE trails its master, so members rise with the bump.
#[tokio::test]
async fn series_resend_raises_moved_members_to_the_master_sequence() {
    let mut server = Server::new_async().await;
    let fixture = Fixture::new();
    let series = fixture.series(upcoming_start(), 3);
    let bumped = coaching_session_series::Model {
        ical_sequence: 4,
        ..series.clone()
    };
    let mut rows = fixture.members(&series);
    rows[1].date += chrono::Duration::days(1);
    rows[1].ical_sequence = 1;
    let received = Received::default();
    let to_coachee = mock_send_to(&mut server, COACHEE_EMAIL, 200, &received).await;
    let to_coach = mock_send_to(&mut server, COACH_EMAIL, 200, &received).await;

    let db = series_send_db(&fixture, &series, rows, &bumped);
    let response = post_resend(
        db.clone(),
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_session_series/{}/resend_invite", series.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    to_coachee.assert_async().await;
    to_coach.assert_async().await;
    for ics in &received.all() {
        let master = master_vevent(ics);
        let moved = ics
            .split("BEGIN:VEVENT")
            .skip(1)
            .find(|event| event.contains("RECURRENCE-ID"))
            .expect("the snapshot has an override VEVENT");
        assert!(master.contains("SEQUENCE:4"), "{ics}");
        assert!(moved.contains("SEQUENCE:4"), "{ics}");
    }
    let db = Arc::try_unwrap(db).expect("the app has been dropped");
    let raised_members = db
        .into_transaction_log()
        .iter()
        .flat_map(|txn| txn.statements())
        .any(|stmt| {
            stmt.sql
                .contains(r#"UPDATE "refactor_platform"."coaching_sessions""#)
                && stmt.sql.contains("GREATEST")
        });
    assert!(
        raised_members,
        "members must be raised to the series SEQUENCE"
    );
}

#[tokio::test]
async fn series_resend_reports_an_unreached_coachee() {
    let mut server = Server::new_async().await;
    let fixture = Fixture::new();
    let series = fixture.series(upcoming_start(), 3);
    let bumped = coaching_session_series::Model {
        ical_sequence: 4,
        ..series.clone()
    };
    let rows = fixture.members(&series);
    let received = Received::default();
    let to_coachee = mock_send_to(&mut server, COACHEE_EMAIL, 400, &received).await;
    let to_coach = mock_send_to(&mut server, COACH_EMAIL, 200, &received).await;

    let response = post_resend(
        series_send_db(&fixture, &series, rows, &bumped),
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_session_series/{}/resend_invite", series.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    to_coachee.assert_async().await;
    to_coach.assert_async().await;
}

// ── Session ────────────────────────────────────────────────────────

#[tokio::test]
async fn session_resend_forbids_the_coachee() {
    let mut server = Server::new_async().await;
    let no_send = server
        .mock("POST", "/emails")
        .expect(0)
        .create_async()
        .await;
    let fixture = Fixture::new();
    let session = fixture.session(upcoming_start());

    let db = Arc::new(
        fixture
            .logged_in_as(&fixture.coachee)
            .append_query_results(vec![vec![(session.clone(), fixture.relationship.clone())]])
            .into_connection(),
    );

    let response = post_resend(
        db,
        config(&server.url()),
        COACHEE_EMAIL,
        format!("/coaching_sessions/{}/resend_invite", session.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    no_send.assert_async().await;
}

/// The gate's load, then the domain's.
fn session_refusal_db(
    fixture: &Fixture,
    session: &coaching_sessions::Model,
) -> Arc<DatabaseConnection> {
    Arc::new(
        fixture
            .logged_in_as(&fixture.coach)
            .append_query_results(vec![vec![(session.clone(), fixture.relationship.clone())]])
            .append_query_results(vec![vec![session.clone()]])
            .into_connection(),
    )
}

#[tokio::test]
async fn session_resend_refuses_a_current_series_member() {
    let server = Server::new_async().await;
    let fixture = Fixture::new();
    let session = coaching_sessions::Model {
        coaching_session_series_id: Some(Id::new_v4()),
        ical_recurrence_id: Some(upcoming_start()),
        ..fixture.session(upcoming_start())
    };

    let response = post_resend(
        session_refusal_db(&fixture, &session),
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_sessions/{}/resend_invite", session.id),
    )
    .await;

    assert_eq!(conflict_reason(response).await, "series_member");
}

#[tokio::test]
async fn session_resend_refuses_a_past_one_off() {
    let server = Server::new_async().await;
    let fixture = Fixture::new();
    let session = fixture.session(past_start());

    let response = post_resend(
        session_refusal_db(&fixture, &session),
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_sessions/{}/resend_invite", session.id),
    )
    .await;

    assert_eq!(conflict_reason(response).await, "no_upcoming_sessions");
}

/// The session loads up to the send: the gate, the domain read, the bump, the parties,
/// then the topics, goals and open actions the description lists.
fn session_send_db(
    fixture: &Fixture,
    session: &coaching_sessions::Model,
) -> Arc<DatabaseConnection> {
    let bumped = coaching_sessions::Model {
        ical_sequence: session.ical_sequence + 1,
        ..session.clone()
    };
    let db = fixture
        .logged_in_as(&fixture.coach)
        .append_query_results(vec![vec![(session.clone(), fixture.relationship.clone())]])
        .append_query_results(vec![vec![session.clone()]])
        .append_query_results(vec![vec![bumped]]);
    Arc::new(
        fixture
            .with_parties(db)
            .append_query_results::<coaching_session_topics::Model, _, _>(vec![vec![]])
            .append_query_results::<goals::Model, _, _>(vec![vec![]])
            .append_query_results::<actions::Model, _, _>(vec![vec![]])
            .into_connection(),
    )
}

#[tokio::test]
async fn session_resend_sends_a_legacy_member_standalone() {
    let mut server = Server::new_async().await;
    let fixture = Fixture::new();
    let session = coaching_sessions::Model {
        coaching_session_series_id: Some(Id::new_v4()),
        ical_recurrence_id: None,
        ..fixture.session(upcoming_start())
    };
    let received = Received::default();
    let to_coachee = mock_send_to(&mut server, COACHEE_EMAIL, 200, &received).await;
    let to_coach = mock_send_to(&mut server, COACH_EMAIL, 200, &received).await;

    let response = post_resend(
        session_send_db(&fixture, &session),
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_sessions/{}/resend_invite", session.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    to_coachee.assert_async().await;
    to_coach.assert_async().await;
    let received = received.all();
    assert_eq!(received.len(), 2);
    for ics in &received {
        assert!(
            ics.contains(&format!("UID:{}@myrefactor.com", session.id)),
            "{ics}"
        );
        assert!(!ics.contains("RECURRENCE-ID"), "{ics}");
    }
}

#[tokio::test]
async fn session_resend_sends_a_one_off_under_the_next_sequence() {
    let mut server = Server::new_async().await;
    let fixture = Fixture::new();
    let session = fixture.session(upcoming_start());
    let received = Received::default();
    let to_coachee = mock_send_to(&mut server, COACHEE_EMAIL, 200, &received).await;
    let to_coach = mock_send_to(&mut server, COACH_EMAIL, 200, &received).await;

    let db = session_send_db(&fixture, &session);
    let response = post_resend(
        db.clone(),
        config(&server.url()),
        COACH_EMAIL,
        format!("/coaching_sessions/{}/resend_invite", session.id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    to_coachee.assert_async().await;
    to_coach.assert_async().await;
    let db = Arc::try_unwrap(db).expect("the app has been dropped");
    assert!(bumped_sequence_of(db, "coaching_sessions"));
}
