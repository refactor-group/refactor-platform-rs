//! Who can see, create, change, reschedule and delete session series, and who a resent invite
//! reaches.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in the tests in
//! `coaching_session_series_controller.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use chrono::{TimeDelta, Utc};
use domain::test_utils::sqlite::fixture::{move_series_into_the_past, remove_roles};
use domain::test_utils::sqlite::read_back::{
    relationship_series, relationship_sessions, series_state,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{
    assert_coachee_not_a_member, assert_removed_coach_resend_refused, assert_resend_mails_both,
    assert_signed_out_refused, ids, ids_in, TestApp, NON_PARTICIPANTS,
};

/// SR1's invite resend path.
fn series_resend(app: &TestApp) -> String {
    format!("/coaching_session_series/{}/resend_invite", app.fixture.sr1)
}

/// Every non-participant of R1, plus the coachee.
const COACHEE_AND_NON_PARTICIPANTS: [Caller; 6] = [
    Caller::Coachee,
    Caller::Member,
    Caller::OtherCoach,
    Caller::Admin,
    Caller::Super,
    Caller::Outsider,
];

/// `days` from now, as the routes take a start.
fn start_in(days: i64) -> String {
    (Utc::now() + TimeDelta::days(days))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

/// A two-session weekly series create body for `relationship_id`.
fn create_body(relationship_id: Id) -> Value {
    json!({
        "coaching_relationship_id": relationship_id,
        "start_at": start_in(10),
        "recurrence": { "frequency": "weekly", "interval": 1, "count": 2 },
        "duration_minutes": 60,
    })
}

/// A PUT body moving the series to a new weekly rule.
fn reschedule_body() -> Value {
    json!({
        "start_at": start_in(12),
        "recurrence": { "frequency": "weekly", "interval": 1, "count": 2 },
        "duration_minutes": 45,
    })
}

/// A reschedule-following body moving `session` and those after it.
fn reschedule_following_body(session: Id) -> Value {
    json!({ "coaching_session_id": session, "start_at": start_in(12) })
}

/// Asserts each caller's series create in `relationship` is answered `expected`, with no
/// series or session written, no document created and no email sent.
async fn assert_create_refused(
    app: &mut TestApp,
    callers: &[Caller],
    relationship: Id,
    expected: StatusCode,
) {
    let series = relationship_series(app.db(), relationship).await;
    let sessions = relationship_sessions(app.db(), relationship).await;
    let documents = app.expect_tiptap("POST", 0).await;
    let emails = app.expect_emails(0).await;

    for &caller in callers {
        let reply = app
            .send(
                caller,
                Method::POST,
                "/coaching_session_series",
                Some(create_body(relationship)),
            )
            .await;
        assert_eq!(reply.status, expected, "{caller:?}");
        if expected == StatusCode::CONFLICT {
            assert_coachee_not_a_member(app, &reply);
        }
        assert_eq!(
            relationship_series(app.db(), relationship).await,
            series,
            "{caller:?}"
        );
        assert_eq!(
            relationship_sessions(app.db(), relationship).await,
            sessions,
            "{caller:?}"
        );
    }
    documents.assert_async().await;
    emails.assert_async().await;
}

/// Asserts each caller's `method` on a series at `suffix` is answered `expected`, leaving
/// the series, its sessions and its documents as they were and sending no email.
async fn assert_series_write_refused(
    callers: &[Caller],
    series: impl Fn(&TestApp) -> (Id, Id),
    method: Method,
    suffix: &str,
    body: impl Fn(Id) -> Option<Value>,
    expected: StatusCode,
) {
    let mut app = TestApp::new().await;
    let (series, first_session) = series(&app);
    let path = format!("/coaching_session_series/{series}{suffix}");
    let before = series_state(app.db(), series).await;
    let documents_created = app.expect_tiptap("POST", 0).await;
    let documents_deleted = app.expect_tiptap("DELETE", 0).await;
    let emails = app.expect_emails(0).await;

    for &caller in callers {
        let reply = app
            .send(caller, method.clone(), &path, body(first_session))
            .await;
        assert_eq!(reply.status, expected, "{caller:?}");
        if expected == StatusCode::CONFLICT {
            assert_coachee_not_a_member(&app, &reply);
        }
        assert!(before.is_some(), "{caller:?}");
        assert_eq!(series_state(app.db(), series).await, before, "{caller:?}");
    }
    documents_created.assert_async().await;
    documents_deleted.assert_async().await;
    emails.assert_async().await;
}

/// SR1 and its first session.
fn sr1(app: &TestApp) -> (Id, Id) {
    (app.fixture.sr1, app.fixture.sr1_sessions[0])
}

/// SR3 and its first session.
fn sr3(app: &TestApp) -> (Id, Id) {
    (app.fixture.sr3, app.fixture.sr3_sessions[0])
}

/// MAIL-09
#[tokio::test]
async fn a_series_resend_mails_both_members() {
    within_time_limit(assert_resend_mails_both(series_resend)).await;
}

/// MAIL-09
#[tokio::test]
async fn a_coach_who_left_cannot_resend_a_series_invite() {
    within_time_limit(assert_removed_coach_resend_refused(series_resend)).await;
}

/// SER-01
#[tokio::test]
async fn a_coach_creates_a_series_with_its_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let series_before = relationship_series(app.db(), r1).await;
        let sessions_before = relationship_sessions(app.db(), r1).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                "/coaching_session_series",
                Some(create_body(r1)),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let series = ids_in(&reply.body["data"]["series"]);
        let sessions = ids_in(&reply.body["data"]["sessions"]);
        assert_eq!(series.len(), 1);
        assert_eq!(sessions.len(), 2);
        let series_after = relationship_series(app.db(), r1).await;
        assert_eq!(
            series_after
                .difference(&series_before)
                .copied()
                .collect::<BTreeSet<_>>(),
            series
        );
        let sessions_after = relationship_sessions(app.db(), r1).await;
        assert_eq!(
            sessions_after
                .difference(&sessions_before)
                .copied()
                .collect::<BTreeSet<_>>(),
            sessions
        );
    })
    .await;
}

/// SER-02
#[tokio::test]
async fn a_coachee_cannot_create_a_series() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        assert_create_refused(&mut app, &[Caller::Coachee], r1, StatusCode::FORBIDDEN).await;
    })
    .await;
}

/// SER-03
#[tokio::test]
async fn non_participants_cannot_create_a_series() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        assert_create_refused(&mut app, &NON_PARTICIPANTS, r1, StatusCode::FORBIDDEN).await;
    })
    .await;
}

/// SER-04
#[tokio::test]
async fn participants_list_the_relationships_series() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, sr1) = (app.fixture.r1, app.fixture.sr1);
        let path = format!("/coaching_session_series?coaching_relationship_id={r1}");

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                app.expect(caller, Method::GET, &path, None).await,
                (StatusCode::OK, ids([sr1])),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SER-05
#[tokio::test]
async fn non_participants_cannot_list_the_relationships_series() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!(
            "/coaching_session_series?coaching_relationship_id={}",
            app.fixture.r1
        );

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// SER-06
#[tokio::test]
async fn participants_read_a_series_with_its_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let sr1 = fixture.sr1;
        let sessions = ids(fixture.sr1_sessions);
        let path = format!("/coaching_session_series/{sr1}");

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app.send(caller, Method::GET, &path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(
                ids_in(&reply.body["data"]["series"]),
                ids([sr1]),
                "{caller:?}"
            );
            assert_eq!(
                ids_in(&reply.body["data"]["sessions"]),
                sessions,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SER-07
#[tokio::test]
async fn non_participants_cannot_read_a_series() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_session_series/{}", app.fixture.sr1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// SER-09
#[tokio::test]
async fn only_the_coach_reschedules_a_series() {
    within_time_limit(assert_series_write_refused(
        &COACHEE_AND_NON_PARTICIPANTS,
        sr1,
        Method::PUT,
        "",
        |_| Some(reschedule_body()),
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SER-11
#[tokio::test]
async fn only_the_coach_reschedules_following_sessions() {
    within_time_limit(assert_series_write_refused(
        &COACHEE_AND_NON_PARTICIPANTS,
        sr1,
        Method::POST,
        "/reschedule",
        |session| Some(reschedule_following_body(session)),
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SER-13
#[tokio::test]
async fn only_the_coach_deletes_a_series() {
    within_time_limit(assert_series_write_refused(
        &COACHEE_AND_NON_PARTICIPANTS,
        sr1,
        Method::DELETE,
        "",
        |_| None,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SER-12
#[tokio::test]
async fn the_coach_deletes_a_series_with_no_upcoming_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let sr1 = app.fixture.sr1;
        // Deleting upcoming sessions takes Postgres advisory locks; past ones are kept.
        move_series_into_the_past(app.db(), sr1).await;

        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::DELETE,
                &format!("/coaching_session_series/{sr1}"),
                None,
            )
            .await;

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(series_state(app.db(), sr1).await, None);
    })
    .await;
}

/// SER-14
#[tokio::test]
async fn a_coach_resends_a_series_invite() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let sr1 = app.fixture.sr1;
        let (_, before, _) = series_state(app.db(), sr1).await.expect("SR1 is seeded");
        let invites = app.expect_emails(2).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_session_series/{sr1}/resend_invite"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        let (_, after, _) = series_state(app.db(), sr1).await.expect("SR1 is kept");
        assert_eq!(after, before + 1);
        invites.assert_async().await;
    })
    .await;
}

/// SER-15
#[tokio::test]
async fn only_the_coach_resends_a_series_invite() {
    within_time_limit(assert_series_write_refused(
        &COACHEE_AND_NON_PARTICIPANTS,
        sr1,
        Method::POST,
        "/resend_invite",
        |_| None,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SER-16
#[tokio::test]
async fn a_coach_removed_from_the_org_cannot_create_a_series() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, o1) = (app.fixture.r1, app.fixture.o1);
        remove_roles(app.db(), app.fixture.user(Caller::Coach), o1).await;
        assert_create_refused(&mut app, &[Caller::Coach], r1, StatusCode::FORBIDDEN).await;
    })
    .await;
}

/// SER-17
#[tokio::test]
async fn a_series_with_a_coachee_who_left_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r3 = app.fixture.r3;
        assert_create_refused(&mut app, &[Caller::Coach], r3, StatusCode::CONFLICT).await;
    })
    .await;
}

/// SER-18
#[tokio::test]
async fn rescheduling_a_series_whose_coachee_left_is_refused() {
    within_time_limit(assert_series_write_refused(
        &[Caller::Coach],
        sr3,
        Method::PUT,
        "",
        |_| Some(reschedule_body()),
        StatusCode::CONFLICT,
    ))
    .await;
}

/// SER-20
#[tokio::test]
async fn resending_a_series_to_a_coachee_who_left_is_refused() {
    within_time_limit(assert_series_write_refused(
        &[Caller::Coach],
        sr3,
        Method::POST,
        "/resend_invite",
        |_| None,
        StatusCode::CONFLICT,
    ))
    .await;
}

/// SER-22
#[tokio::test]
async fn an_unknown_series_is_not_found() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let unknown = Id::new_v4();
        let session = app.fixture.sr1_sessions[0];
        let emails = app.expect_emails(0).await;

        for (method, suffix, body) in [
            (Method::GET, "", None),
            (Method::POST, "/resend_invite", None),
            (
                Method::POST,
                "/reschedule",
                Some(reschedule_following_body(session)),
            ),
        ] {
            let path = format!("/coaching_session_series/{unknown}{suffix}");
            let (status, _) = app.expect(Caller::Coach, method.clone(), &path, body).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
        }
        emails.assert_async().await;
    })
    .await;
}

/// SER-23
#[tokio::test]
async fn signed_out_callers_cannot_reach_series() {
    within_time_limit(assert_signed_out_refused(|app| {
        let (r1, sr1) = (app.fixture.r1, app.fixture.sr1);
        vec![
            (
                Method::GET,
                format!("/coaching_session_series?coaching_relationship_id={r1}"),
            ),
            (Method::GET, format!("/coaching_session_series/{sr1}")),
            (Method::POST, "/coaching_session_series".to_string()),
            (Method::PUT, format!("/coaching_session_series/{sr1}")),
            (
                Method::POST,
                format!("/coaching_session_series/{sr1}/reschedule"),
            ),
            (Method::DELETE, format!("/coaching_session_series/{sr1}")),
            (
                Method::POST,
                format!("/coaching_session_series/{sr1}/resend_invite"),
            ),
        ]
    }))
    .await;
}
