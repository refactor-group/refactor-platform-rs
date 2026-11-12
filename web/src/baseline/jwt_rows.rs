//! The collaboration token (`JWT`).

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use super::harness::{Baseline, NON_PARTICIPANTS};

/// The token path for `session`.
fn token_path(session: Id) -> String {
    format!("/jwt/generate_collab_token?coaching_session_id={session}")
}

/// JWT-01
#[tokio::test]
async fn jwt_01_participants_get_a_token_for_the_sessions_document() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline
                .send(caller, Method::GET, &token_path(s1), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            let document = reply.body["data"]["sub"].as_str().unwrap_or_default();
            assert!(document.contains(&s1.to_string()), "{caller:?}: {document}");
        }
    })
    .await;
}

/// JWT-02
#[tokio::test]
async fn jwt_02_non_participants_get_no_token() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(caller, Method::GET, &token_path(s1), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }

        // The row's note: an unknown session is not found.
        let (status, _) = baseline
            .expect(Caller::Coach, Method::GET, &token_path(Id::new_v4()), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}

/// JWT-03
#[tokio::test]
async fn jwt_03_a_removed_coachee_gets_no_token() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s3 = baseline.world.s3;

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &token_path(s3), None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}
