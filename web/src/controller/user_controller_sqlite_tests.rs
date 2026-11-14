//! Who can look up users by email and change a user's settings.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! Which users a lookup finds once allowed is tested in `domain/src/user_role_sqlite_tests.rs`;
//! the lookup's rate limit takes a Postgres advisory lock.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::read_back::working_hours;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use serde_json::json;

use crate::test_utils::sqlite::{assert_signed_out_refused, ids, TestApp};

/// USR-04
#[tokio::test]
async fn signed_out_callers_cannot_reach_user_routes() {
    within_time_limit(assert_signed_out_refused(|app| {
        let coachee = app.fixture.user(Caller::Coachee);
        let r1 = app.fixture.r1;
        vec![
            (
                Method::GET,
                format!("/users?email={}", Caller::Member.email()),
            ),
            (Method::GET, format!("/users/{coachee}")),
            (Method::PUT, format!("/users/{coachee}")),
            (Method::GET, format!("/users/{coachee}/coaching_sessions")),
            (
                Method::GET,
                format!(
                    "/users/{coachee}/coaching_sessions/nearest?coaching_relationship_ids={r1}"
                ),
            ),
        ]
    }))
    .await;
}

/// USR-05
#[tokio::test]
async fn only_the_user_changes_their_own_settings() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.fixture.user(Caller::Coachee);
        let before = working_hours(app.db(), coachee).await;

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let (status, _) = app
                .expect(
                    caller,
                    Method::PUT,
                    &format!("/users/{coachee}"),
                    Some(json!({ "working_hours_start": "09:30" })),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(working_hours(app.db(), coachee).await, before, "{caller:?}");
        }
    })
    .await;
}

/// USR-06
#[tokio::test]
async fn only_admins_look_up_users_by_email() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/users?email={}", Caller::Member.email());

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// Only the user reads their own record.
#[tokio::test]
async fn only_the_user_reads_their_own_record() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.fixture.user(Caller::Coachee);
        let path = format!("/users/{coachee}");

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let reply = app.send(caller, Method::GET, &path, None).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(&reply.bytes[..], b"FORBIDDEN", "{caller:?}");
        }

        let (status, read) = app.expect(Caller::Coachee, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(read, ids([coachee]));
    })
    .await;
}

/// A user path id that is not a UUID is a bad request.
#[tokio::test]
async fn a_malformed_user_id_is_a_bad_request() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let reply = app
            .send(Caller::Coachee, Method::GET, "/users/not-a-uuid", None)
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert_eq!(&reply.bytes[..], b"Invalid user_id");
    })
    .await;
}
