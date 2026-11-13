//! Who can see, start and stop a session's meeting recording.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in the tests in `meeting_recording_controller.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::meeting_recording::MeetingRecordingStatus;
use domain::test_utils::sqlite::fixture::seed_active_recording;
use domain::test_utils::sqlite::read_back::{recording_state, session_recordings};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{created_id, BotRequest, TestApp, NON_PARTICIPANTS};

const MEETING_URL: &str = "https://meet.google.com/abc-defg-hij";

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

/// TRN-19 and MEET-04
#[tokio::test]
async fn either_participant_starts_a_recording_bot() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            // A fresh app each: a second start on the same session answers 409.
            let mut app = TestApp::new().await;
            let s1 = app.fixture.s1;
            let path = format!("/coaching_sessions/{s1}/meeting_recording");
            let before = session_recordings(app.db(), s1).await;

            let reply = app
                .send(
                    caller,
                    Method::POST,
                    &path,
                    Some(json!({ "meeting_url": MEETING_URL })),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let recording = created_id(&reply);
            assert!(!before.contains(&recording), "{caller:?}");
            let requests = app.take_bot_requests();
            let [BotRequest::Create {
                bot_id,
                meeting_url,
                coaching_session_id,
            }] = requests.as_slice()
            else {
                panic!("{caller:?}: one bot sent, got {requests:?}");
            };
            assert_eq!(meeting_url, MEETING_URL, "{caller:?}");
            assert_eq!(coaching_session_id, &Some(s1.to_string()), "{caller:?}");
            assert_eq!(
                recording_state(app.db(), recording).await,
                Some((bot_id.clone(), MeetingRecordingStatus::Pending)),
                "{caller:?}"
            );
            assert_eq!(
                session_recordings(app.db(), s1).await.len(),
                before.len() + 1,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TRN-20
#[tokio::test]
async fn either_participant_stops_the_recording_bot() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let s1 = app.fixture.s1;
            let path = format!("/coaching_sessions/{s1}/meeting_recording");
            let bot_id = seed_active_recording(app.db(), s1).await;
            let before = session_recordings(app.db(), s1).await;

            let reply = app.send(caller, Method::DELETE, &path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(
                app.take_bot_requests(),
                vec![BotRequest::Stop {
                    bot_id: bot_id.clone()
                }],
                "{caller:?}"
            );
            assert_eq!(
                recording_state(app.db(), created_id(&reply)).await,
                Some((bot_id, MeetingRecordingStatus::Cancelled)),
                "{caller:?}"
            );
            assert_eq!(session_recordings(app.db(), s1).await, before, "{caller:?}");
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
