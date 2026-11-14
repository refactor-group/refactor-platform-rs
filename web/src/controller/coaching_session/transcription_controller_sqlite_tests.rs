//! Who can read a session's transcriptions.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in `transcription_controller_tests.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::body::Body;
use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::{
    assert_signed_out_refused, assert_transcription_reads_refused, ids, TestApp,
};

/// The coach's and the coachee's transcript lines, as seeded.
const COACH_LINE: &str = "What will you do next, kumquat?";

const COACHEE_LINE: &str = "I will send the kumquat plan.";

/// TRN-01
#[tokio::test]
async fn participants_read_the_latest_transcription() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_sessions/{}/transcriptions", app.fixture.s1);
        let expected = ids([app.fixture.tr1]);

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

/// TRN-02
#[tokio::test]
async fn non_participants_cannot_read_the_latest_transcription() {
    within_time_limit(assert_transcription_reads_refused(|_| String::new())).await;
}

/// TRN-03
#[tokio::test]
async fn participants_download_only_the_coachs_lines() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let path = format!(
            "/coaching_sessions/{}/transcriptions/{}?speaker=coach",
            fixture.s1, fixture.tr1
        );

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app
                .send_with(
                    caller,
                    Method::GET,
                    &path,
                    &[("accept", "text/plain")],
                    Body::empty(),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            let text = String::from_utf8_lossy(&reply.bytes);
            assert!(text.contains(COACH_LINE), "{caller:?}: {text}");
            assert!(!text.contains(COACHEE_LINE), "{caller:?}: {text}");
        }
    })
    .await;
}

/// TRN-04
#[tokio::test]
async fn non_participants_cannot_read_a_transcription() {
    within_time_limit(async {
        assert_transcription_reads_refused(|b| format!("/{}", b.fixture.tr1)).await;

        // The matrix's note: TR1 under a session it is not in is not found.
        let mut app = TestApp::new().await;
        let path = format!(
            "/coaching_sessions/{}/transcriptions/{}",
            app.fixture.s3, app.fixture.tr1
        );
        let reply = app.send(Caller::Coach, Method::GET, &path, None).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert_eq!(reply.body["error"], "transcription_not_found");
    })
    .await;
}

/// TRN-29
#[tokio::test]
async fn signed_out_callers_cannot_reach_transcripts_suggestions_recordings_or_images() {
    within_time_limit(assert_signed_out_refused(|app| {
        let fixture = &app.fixture;
        let session = format!("/coaching_sessions/{}", fixture.s1);
        let transcription = format!("{session}/transcriptions/{}", fixture.tr1);
        let suggestions = format!("{session}/transcript_suggestions");
        let suggestion = format!("{suggestions}/{}", fixture.sg1);
        let recording = format!("{session}/meeting_recording");
        let image = format!("/coaching_session_images/{}", fixture.img1);
        vec![
            (Method::GET, format!("{session}/transcriptions")),
            (Method::GET, transcription.clone()),
            (
                Method::GET,
                format!("{transcription}/transcription_segments"),
            ),
            (Method::GET, suggestions.clone()),
            (Method::GET, format!("{suggestions}/scan")),
            (Method::POST, format!("{suggestions}/scan")),
            (Method::POST, format!("{suggestion}/accept")),
            (Method::POST, format!("{suggestion}/dismiss")),
            (Method::POST, format!("{suggestion}/restore")),
            (Method::GET, recording.clone()),
            (Method::POST, recording.clone()),
            (Method::DELETE, recording),
            (Method::POST, format!("{session}/images")),
            (Method::GET, image.clone()),
            (Method::DELETE, image.clone()),
            (Method::POST, format!("{image}/restore")),
        ]
    }))
    .await;
}
