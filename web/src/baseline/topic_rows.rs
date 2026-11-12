//! Topics (`TOP`).

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use super::harness::{ids, Baseline};

/// TOP-01
#[tokio::test]
async fn top_01_participants_list_the_sessions_topics() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/coaching_sessions/{}/topics", world.s1);
        let expected = ids([world.t1, world.t2]);

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                baseline.expect(caller, Method::GET, &path, None).await,
                (StatusCode::OK, expected.clone()),
                "{caller:?}"
            );
        }
    })
    .await;
}
