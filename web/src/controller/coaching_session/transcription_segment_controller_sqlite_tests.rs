//! Transcription segments (`TRN`), on SQLite as each access-matrix caller.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::transcription_segments;
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::{assert_transcription_reads_refused, TestApp};

/// TRN-05
#[tokio::test]
async fn participants_read_the_transcriptions_segments() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, tr1) = (app.world.s1, app.world.tr1);
        let path = format!("/coaching_sessions/{s1}/transcriptions/{tr1}/transcription_segments");
        let expected = transcription_segments(app.db(), tr1).await;
        assert_eq!(expected.len(), 2);

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

/// TRN-06
#[tokio::test]
async fn non_participants_cannot_read_the_transcriptions_segments() {
    within_time_limit(assert_transcription_reads_refused(|b| {
        format!("/{}/transcription_segments", b.world.tr1)
    }))
    .await;
}
