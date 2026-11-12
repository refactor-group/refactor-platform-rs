//! Who can see, start and stop a session's meeting recording.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in the tests in `meeting_recording_controller.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::read_back::session_recordings;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{TestApp, NON_PARTICIPANTS};

/// Asserts each caller is refused every recording route on `session`, with no recording
/// written. The POST is refused before any bot is sent.
async fn assert_recording_refused(callers: &[Caller], session: impl Fn(&TestApp) -> Id) {
    let mut app = TestApp::new().await;
    let session = session(&app);
    let path = format!("/coaching_sessions/{session}/meeting_recording");
    let before = session_recordings(app.db(), session).await;

    for &caller in callers {
        for (method, body) in [
            (Method::GET, None),
            (
                Method::POST,
                Some(json!({ "meeting_url": "https://meet.google.com/abc-defg-hij" })),
            ),
            (Method::DELETE, None),
        ] {
            let (status, _) = app.expect(caller, method.clone(), &path, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
        }
    }
    assert_eq!(session_recordings(app.db(), session).await, before);
}

/// TRN-18
#[tokio::test]
async fn participants_read_the_latest_recording() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let path = format!("/coaching_sessions/{s1}/meeting_recording");
        let expected = session_recordings(app.db(), s1).await;
        assert_eq!(expected.len(), 1);

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                app.expect(caller, Method::GET, &path, None).await,
                (StatusCode::OK, expected.clone()),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TRN-21
#[tokio::test]
async fn non_participants_cannot_reach_the_sessions_recording() {
    within_time_limit(assert_recording_refused(&NON_PARTICIPANTS, |b| {
        b.fixture.s1
    }))
    .await;
}

/// TRN-22
#[tokio::test]
async fn a_removed_coachee_cannot_reach_their_sessions_recording() {
    within_time_limit(assert_recording_refused(&[Caller::Removed], |b| {
        b.fixture.s3
    }))
    .await;
}
