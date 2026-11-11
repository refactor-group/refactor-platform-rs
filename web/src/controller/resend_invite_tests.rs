//! `POST /coaching_session_series/:id/resend_invite` and
//! `POST /coaching_sessions/:id/resend_invite`, driven through the real routers with a
//! logged-in session, a mock database, and a mock Resend.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::{body::Body, extract::Request, http::StatusCode, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{NaiveDateTime, Utc};
use mockito::{Mock, Server, ServerGuard};
use password_auth::generate_hash;
use sea_orm::{prelude::DateTimeWithTimeZone, DatabaseBackend, MockDatabase};
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use domain::user::Backend;
use domain::{
    actions, coaching_relationships, coaching_session_series, coaching_session_topics,
    coaching_sessions, goals, organizations, user_roles, users, Id,
};
use service::config::Config;

use crate::controller::user_session_controller;
use crate::router::{coaching_session_series_routes, coaching_sessions_routes};
use crate::AppState;

pub(super) type Series = coaching_session_series::Model;
pub(super) type Session = coaching_sessions::Model;

fn now() -> DateTimeWithTimeZone {
    Utc::now().into()
}

/// A whole hour `days` from now, so it is minute-aligned.
pub(super) fn start_in(days: i64) -> NaiveDateTime {
    (Utc::now() + chrono::Duration::days(days))
        .date_naive()
        .and_hms_opt(15, 0, 0)
        .unwrap()
}

/// One `POST`: who logs in (`None` sends no cookie), where it goes, and the loads it will
/// make.
pub(super) struct Call {
    pub(super) caller: Option<String>,
    pub(super) path: String,
    pub(super) db: MockDatabase,
}

impl Call {
    pub(super) fn then(self, more: impl FnOnce(MockDatabase) -> MockDatabase) -> Self {
        Self {
            db: more(self.db),
            ..self
        }
    }
}

/// A coach and coachee in one organization and the relationship between them, building
/// the calls each route makes.
pub(super) struct Fixture {
    pub(super) coach: users::Model,
    pub(super) coachee: users::Model,
    pub(super) organization: organizations::Model,
    pub(super) relationship: coaching_relationships::Model,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let user = |email: &str| users::Model {
            id: Id::new_v4(),
            email: email.to_string(),
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            display_name: None,
            password: Some(generate_hash("password123")),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            default_coaching_session_duration_minutes: 60,
            working_hours_start: domain::working_hours::default_start(),
            working_hours_end: domain::working_hours::default_end(),
            roles: vec![],
            invite_status: None,
            created_at: now(),
            updated_at: now(),
        };
        let (coach, coachee) = (user("coach@example.com"), user("coachee@example.com"));
        let organization = organizations::Model {
            id: Id::new_v4(),
            name: "Acme".to_string(),
            logo: None,
            slug: "acme".to_string(),
            created_at: now(),
            updated_at: now(),
            archived_at: None,
            archived_by: None,
        };
        let relationship = coaching_relationships::Model {
            id: Id::new_v4(),
            organization_id: organization.id,
            coach_id: coach.id,
            coachee_id: coachee.id,
            slug: "test".to_string(),
            created_at: now(),
            updated_at: now(),
        };
        Self {
            coach,
            coachee,
            organization,
            relationship,
        }
    }

    pub(super) fn series(&self, start_at: NaiveDateTime) -> Series {
        Series {
            id: Id::new_v4(),
            coaching_relationship_id: self.relationship.id,
            rule: serde_json::json!({
                "start_at": start_at,
                "recurrence": { "frequency": "weekly", "interval": 1, "count": 3 },
                "duration_minutes": 60,
            }),
            ical_sequence: 3,
            created_by_user_id: self.coach.id,
            created_at: now(),
            updated_at: now(),
        }
    }

    pub(super) fn session(&self, date: NaiveDateTime) -> Session {
        Session {
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
            created_at: now(),
            updated_at: now(),
            hydrated_at: None,
            notice_given_at: now(),
        }
    }

    /// One member per slot of `series`, each in the slot it was created for.
    pub(super) fn members(&self, series: &Series) -> Vec<Session> {
        let start_at: NaiveDateTime =
            serde_json::from_value(series.rule["start_at"].clone()).unwrap();
        (0..3)
            .map(|week| start_at + chrono::Duration::weeks(week))
            .map(|slot| Session {
                coaching_session_series_id: Some(series.id),
                ical_recurrence_id: Some(slot),
                ..self.session(slot)
            })
            .collect()
    }

    /// The login and `require_auth` loads for `caller`.
    pub(super) fn login(&self, caller: &users::Model, path: String) -> Call {
        let role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(self.organization.id),
            user_id: caller.id,
            created_at: now(),
            updated_at: now(),
        };
        let row = (caller.clone(), role);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![row.clone()]])
            .append_query_results([vec![row]]);
        Call {
            caller: Some(caller.email.clone()),
            path,
            db,
        }
    }

    /// No login, so no loads.
    pub(super) fn anonymous(path: String) -> Call {
        Call {
            caller: None,
            path,
            db: MockDatabase::new(DatabaseBackend::Postgres),
        }
    }

    /// The loads `InviteParties::load` makes.
    fn parties(&self, db: MockDatabase) -> MockDatabase {
        db.append_query_results([vec![self.coach.clone()]])
            .append_query_results([vec![self.coachee.clone()]])
            .append_query_results([vec![self.organization.clone()]])
    }

    /// The relationship, then which of `members` still hold a role in its organization.
    pub(super) fn membership(&self, db: MockDatabase, members: &[Id]) -> MockDatabase {
        let rows: Vec<_> = members
            .iter()
            .map(|&id| BTreeMap::from([("user_id".to_string(), sea_orm::Value::from(id))]))
            .collect();
        db.append_query_results([vec![self.relationship.clone()]])
            .append_query_results([rows])
    }

    /// Both participants.
    pub(super) fn both(&self) -> Vec<Id> {
        vec![self.coach.id, self.coachee.id]
    }

    fn series_gate(&self, caller: &users::Model, series: &Series) -> Call {
        let path = format!("/coaching_session_series/{}/resend_invite", series.id);
        self.gate(caller, series, path)
    }

    /// The login and the series access check's loads, for a series route at `path`.
    pub(super) fn gate(&self, caller: &users::Model, series: &Series, path: String) -> Call {
        self.login(caller, path).then(|db| {
            db.append_query_results([vec![series.clone()]])
                .append_query_results([vec![self.relationship.clone()]])
        })
    }

    /// The coach past the gate, then the domain's series and member reads.
    fn series_loaded(&self, series: &Series, rows: Vec<Session>) -> Call {
        self.series_gate(&self.coach, series).then(|db| {
            db.append_query_results([vec![series.clone()]])
                .append_query_results([rows])
        })
    }

    /// Up to the send with both participants members.
    fn series_sent(&self, series: &Series, rows: Vec<Session>) -> Call {
        self.series_sent_to(series, rows, &self.both())
    }

    /// Up to the send: the membership check, the bump, the members raised to it, the
    /// parties, then the goals the description lists.
    fn series_sent_to(&self, series: &Series, rows: Vec<Session>, members: &[Id]) -> Call {
        let bumped = series.ical_sequence + 1;
        let raised: Vec<_> = rows
            .iter()
            .map(|row| Session {
                ical_sequence: row.ical_sequence.max(bumped),
                ..row.clone()
            })
            .collect();
        let bumped = Series {
            ical_sequence: bumped,
            ..series.clone()
        };
        self.series_loaded(series, rows).then(|db| {
            self.parties(
                self.membership(db, members)
                    .append_query_results([vec![bumped]])
                    .append_query_results([raised]),
            )
            .append_query_results::<goals::Model, _, _>([vec![]])
        })
    }

    fn session_gate(&self, caller: &users::Model, session: &Session) -> Call {
        let path = format!("/coaching_sessions/{}/resend_invite", session.id);
        self.login(caller, path).then(|db| {
            db.append_query_results([vec![(session.clone(), self.relationship.clone())]])
        })
    }

    /// The coach past the gate, then the domain's read.
    fn session_loaded(&self, session: &Session) -> Call {
        self.session_gate(&self.coach, session)
            .then(|db| db.append_query_results([vec![session.clone()]]))
    }

    /// Up to the send with both participants members.
    fn session_sent(&self, session: &Session) -> Call {
        self.session_sent_to(session, &self.both())
    }

    /// Up to the send: the membership check, the bump, the parties, then the topics,
    /// goals and open actions the description lists.
    fn session_sent_to(&self, session: &Session, members: &[Id]) -> Call {
        let bumped = Session {
            ical_sequence: session.ical_sequence + 1,
            ..session.clone()
        };
        self.session_loaded(session).then(|db| {
            self.parties(
                self.membership(db, members)
                    .append_query_results([vec![bumped]]),
            )
            .append_query_results::<coaching_session_topics::Model, _, _>([vec![]])
            .append_query_results::<goals::Model, _, _>([vec![]])
            .append_query_results::<actions::Model, _, _>([vec![]])
        })
    }
}

struct Outcome {
    status: StatusCode,
    reason: Option<String>,
    /// The 409 body's `details`.
    details: serde_json::Value,
    /// The unfolded `.ics` of every Resend request.
    sent: Vec<String>,
    /// Every UPDATE that wrote an `ical_sequence`.
    sequence_updates: Vec<String>,
    /// Every INSERT, UPDATE and DELETE.
    writes: Vec<String>,
}

impl Outcome {
    fn updated(&self, table: &str, also: &str) -> bool {
        let target = format!(r#"UPDATE "refactor_platform"."{table}""#);
        let updated = |sql: &&String| sql.contains(&target) && sql.contains(also);
        self.sequence_updates.iter().any(|sql| updated(&sql))
    }
}

/// How Resend answers each participant: `Some(status)` expects one request, `None` none.
#[derive(Clone, Copy)]
struct Mail {
    coachee: Option<usize>,
    coach: Option<usize>,
}

impl Mail {
    const NONE: Mail = Mail {
        coachee: None,
        coach: None,
    };

    /// The coachee answered with `status`, the coach with 200.
    fn coachee(status: usize) -> Mail {
        Mail {
            coachee: Some(status),
            coach: Some(200),
        }
    }
}

/// Mocks Resend per participant as `mail` says.
async fn mock_resend(
    server: &mut ServerGuard,
    sent: &Arc<Mutex<Vec<String>>>,
    mail: Mail,
) -> Vec<Mock> {
    let mut mocks = Vec::new();
    for (email, status) in [("coachee@", mail.coachee), ("coach@", mail.coach)] {
        let sent = sent.clone();
        let record = move |request: &mockito::Request| {
            let ics = serde_json::from_slice::<serde_json::Value>(request.body().unwrap())
                .ok()
                .and_then(|body| body["attachments"][0]["content"].as_str().map(String::from))
                .and_then(|content| STANDARD.decode(content).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            let unfolded = ics.replace("\r\n ", "").replace("\r\n\t", "");
            sent.lock().unwrap().push(unfolded);
            true
        };
        let to = mockito::Matcher::Regex(format!(r#""to":\[[^\]]*{email}"#));
        let mock = server
            .mock("POST", "/emails")
            .match_body(to)
            .match_request(record);
        let mock = mock
            .with_status(status.unwrap_or(200))
            .with_body(r#"{"id":"e"}"#)
            .expect(usize::from(status.is_some()));
        mocks.push(mock.create_async().await);
    }
    mocks
}

/// Logs the caller in over the real routers, then sends the resend `POST`.
async fn resend(call: Call, mail: Mail) -> Outcome {
    let mut server = Server::new_async().await;
    let sent = Arc::new(Mutex::new(Vec::new()));
    let mocks = mock_resend(&mut server, &sent, mail).await;
    let config = Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        "--session-scheduled-email-template-id=session_template",
        "--recurring-sessions-scheduled-email-template-id=series_template",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
    ]);
    let db = Arc::new(call.db.into_connection());
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
    let app = Router::new()
        .route("/login", post(user_session_controller::login))
        .with_state(app_state.clone())
        .merge(coaching_sessions_routes(app_state.clone()))
        .merge(coaching_session_series_routes(app_state))
        .layer(AuthManagerLayerBuilder::new(Backend::new(&db), session_layer).build());

    let mut request = Request::post(call.path).header("x-version", "1.0.0");
    if let Some(caller) = call.caller {
        let form = format!("email={caller}&password=password123");
        let login = Request::post("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(form))
            .unwrap();
        let login = app.clone().oneshot(login).await.unwrap();
        request = request.header("cookie", login.headers()["set-cookie"].clone());
    }
    let request = request.body(Body::empty()).unwrap();
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await;
    let details = serde_json::from_slice::<serde_json::Value>(&body.unwrap())
        .map(|body| body["details"].clone())
        .unwrap_or_default();
    let reason = details["reason"].as_str().map(String::from);
    for mock in mocks {
        mock.assert_async().await;
    }
    let log = Arc::try_unwrap(db).expect("the app has been dropped");
    let writes: Vec<String> = log
        .into_transaction_log()
        .iter()
        .flat_map(|txn| txn.statements())
        .map(|stmt| stmt.sql.clone())
        .filter(|sql| {
            ["INSERT", "UPDATE", "DELETE"]
                .iter()
                .any(|w| sql.starts_with(w))
        })
        .collect();
    let sequence_updates = writes
        .iter()
        .filter(|sql| sql.starts_with("UPDATE") && sql.contains("ical_sequence"))
        .cloned()
        .collect();
    let sent = sent.lock().unwrap().clone();
    Outcome {
        status,
        reason,
        details,
        sent,
        sequence_updates,
        writes,
    }
}

/// Each refusal, by its 409 reason (`None` is a 403), sends nothing and bumps nothing.
#[tokio::test]
async fn refusals_send_nothing_and_bump_nothing() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let past_series = f.series(start_in(-30));
    let mut legacy_rows = f.members(&series);
    legacy_rows[1].ical_recurrence_id = None;
    let one_off = f.session(start_in(7));
    let member = f.members(&series).remove(0);

    let cases = [
        (f.series_gate(&f.coachee, &series), None),
        (
            f.series_loaded(&past_series, f.members(&past_series)),
            Some("no_upcoming_sessions"),
        ),
        (f.series_loaded(&series, legacy_rows), Some("legacy_series")),
        (f.session_gate(&f.coachee, &one_off), None),
        (f.session_loaded(&member), Some("series_member")),
        (
            f.session_loaded(&f.session(start_in(-30))),
            Some("no_upcoming_sessions"),
        ),
    ];

    for (case, (call, reason)) in cases.into_iter().enumerate() {
        let status = match reason {
            Some(_) => StatusCode::CONFLICT,
            None => StatusCode::FORBIDDEN,
        };
        let out = resend(call, Mail::NONE).await;
        assert_eq!(out.status, status, "case {case}");
        assert_eq!(out.reason.as_deref(), reason, "case {case}");
        assert!(out.sequence_updates.is_empty(), "case {case}");
    }
}

/// No cookie is a 401 and an unknown id a 404; neither sends nor bumps.
#[tokio::test]
async fn unauthenticated_and_unknown_send_nothing_and_bump_nothing() {
    let f = Fixture::new();
    let series_path = format!("/coaching_session_series/{}/resend_invite", Id::new_v4());
    let session_path = format!("/coaching_sessions/{}/resend_invite", Id::new_v4());

    let cases = [
        (
            Fixture::anonymous(series_path.clone()),
            StatusCode::UNAUTHORIZED,
        ),
        (
            Fixture::anonymous(session_path.clone()),
            StatusCode::UNAUTHORIZED,
        ),
        (
            f.login(&f.coach, series_path)
                .then(|db| db.append_query_results([Vec::<Series>::new()])),
            StatusCode::NOT_FOUND,
        ),
        (
            f.login(&f.coach, session_path).then(|db| {
                db.append_query_results([Vec::<(Session, coaching_relationships::Model)>::new()])
            }),
            StatusCode::NOT_FOUND,
        ),
    ];

    for (case, (call, status)) in cases.into_iter().enumerate() {
        let out = resend(call, Mail::NONE).await;
        assert_eq!(out.status, status, "case {case}");
        assert!(out.sequence_updates.is_empty(), "case {case}");
    }
}

#[tokio::test]
async fn series_resend_bumps_the_series_and_raises_its_members() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let mut rows = f.members(&series);
    rows[1].date += chrono::Duration::days(1);
    rows[1].ical_sequence = 1;

    let out = resend(f.series_sent(&series, rows), Mail::coachee(200)).await;

    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(out.sent.len(), 2);
    assert!(out.updated("coaching_session_series", "ical_sequence"));
    assert!(out.updated("coaching_sessions", "GREATEST"));
}

#[tokio::test]
async fn series_resend_reports_an_unreached_coachee() {
    let f = Fixture::new();
    let series = f.series(start_in(7));

    let out = resend(
        f.series_sent(&series, f.members(&series)),
        Mail::coachee(400),
    )
    .await;

    assert_eq!(out.status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn session_resend_sends_a_legacy_member_standalone() {
    let f = Fixture::new();
    let session = Session {
        coaching_session_series_id: Some(Id::new_v4()),
        ..f.session(start_in(7))
    };

    let out = resend(f.session_sent(&session), Mail::coachee(200)).await;

    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(out.sent.len(), 2);
    for ics in &out.sent {
        assert!(ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
        assert!(!ics.contains("RECURRENCE-ID"), "{ics}");
    }
}

#[tokio::test]
async fn session_resend_bumps_a_one_off() {
    let f = Fixture::new();
    let session = f.session(start_in(7));

    let out = resend(f.session_sent(&session), Mail::coachee(200)).await;

    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(out.sent.len(), 2);
    assert!(out.updated("coaching_sessions", "ical_sequence"));
}

/// A coachee no longer in the organization is refused before anything is bumped or sent,
/// with the coachee named so the coach can re-add them. Both resends, coachee alone or
/// both removed.
#[tokio::test]
async fn resend_to_a_removed_coachee_is_refused() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let session = f.session(start_in(7));
    let removed = [vec![f.coach.id], vec![]];

    let calls = removed.iter().flat_map(|members| {
        [
            f.series_loaded(&series, f.members(&series))
                .then(|db| f.membership(db, members)),
            f.session_loaded(&session)
                .then(|db| f.membership(db, members)),
        ]
    });

    for (case, call) in calls.enumerate() {
        let out = resend(call, Mail::NONE).await;
        assert_eq!(out.status, StatusCode::CONFLICT, "case {case}");
        assert_eq!(
            out.reason.as_deref(),
            Some("coachee_not_a_member"),
            "case {case}"
        );
        assert_eq!(
            out.details["user_id"],
            serde_json::json!(f.coachee.id),
            "case {case}"
        );
        assert_eq!(
            out.details["organization_id"],
            serde_json::json!(f.organization.id),
            "case {case}"
        );
        assert!(out.writes.is_empty(), "case {case}: {:?}", out.writes);
    }
}

/// A coach no longer in the organization still resends to a current coachee, who alone
/// is mailed.
#[tokio::test]
async fn resend_with_a_removed_coach_mails_only_the_coachee() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let session = f.session(start_in(7));
    let coachee_only = Mail {
        coachee: Some(200),
        coach: None,
    };

    let calls = [
        f.series_sent_to(&series, f.members(&series), &[f.coachee.id]),
        f.session_sent_to(&session, &[f.coachee.id]),
    ];

    for (case, call) in calls.into_iter().enumerate() {
        let out = resend(call, coachee_only).await;
        assert_eq!(out.status, StatusCode::NO_CONTENT, "case {case}");
        assert_eq!(out.sent.len(), 1, "case {case}");
    }
}
