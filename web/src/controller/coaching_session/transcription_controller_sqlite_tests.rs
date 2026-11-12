//! Transcriptions (`TRN`), on SQLite as each access-matrix caller.

use axum::body::Body;
use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::{assert_transcription_reads_refused, ids, TestApp};

/// The coach's and the coachee's transcript lines, as seeded.
const COACH_LINE: &str = "What will you do next, kumquat?";

const COACHEE_LINE: &str = "I will send the kumquat plan.";

/// TRN-01
#[tokio::test]
async fn participants_read_the_latest_transcription() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_sessions/{}/transcriptions", app.world.s1);
        let expected = ids([app.world.tr1]);

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
        let world = &app.world;
        let path = format!(
            "/coaching_sessions/{}/transcriptions/{}?speaker=coach",
            world.s1, world.tr1
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
        assert_transcription_reads_refused(|b| format!("/{}", b.world.tr1)).await;

        // The matrix's note: TR1 under a session it is not in is not found.
        let mut app = TestApp::new().await;
        let path = format!(
            "/coaching_sessions/{}/transcriptions/{}",
            app.world.s3, app.world.tr1
        );
        let (status, _) = app.expect(Caller::Coach, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}
