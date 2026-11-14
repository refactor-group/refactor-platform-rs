//! Which status the batch session-goals check gives a mix of refused and unknown session ids,
//! through the production routes against a real database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use crate::test_utils::sqlite::TestApp;

/// The status of the coach's batch read of `ids`.
async fn coach_batch_status(app: &mut TestApp, ids: [Id; 2]) -> StatusCode {
    let [first, second] = ids;
    app.send(
        Caller::Coach,
        Method::GET,
        &format!("/coaching_sessions/goals?coaching_session_ids={first},{second}"),
        None,
    )
    .await
    .status
}

#[tokio::test]
async fn the_first_refused_or_unknown_session_decides_the_status() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s2, unknown) = (app.fixture.s2, Id::new_v4());

        assert_eq!(
            coach_batch_status(&mut app, [s2, unknown]).await,
            StatusCode::FORBIDDEN,
            "S2 first"
        );
        assert_eq!(
            coach_batch_status(&mut app, [unknown, s2]).await,
            StatusCode::NOT_FOUND,
            "unknown first"
        );
    })
    .await;
}
