//! A user's own goals (`GOAL`) through `/users/{id}/goals`, on SQLite as each access-matrix
//! caller.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::{ids, TestApp};

/// GOAL-22
#[tokio::test]
async fn a_coach_lists_the_goals_they_created() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coach = app.world.user(Caller::Coach);
        let expected = ids([app.world.g1]);

        assert_eq!(
            app.expect(
                Caller::Coach,
                Method::GET,
                &format!("/users/{coach}/goals"),
                None
            )
            .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// GOAL-23
#[tokio::test]
async fn a_coachee_who_created_no_goal_lists_none() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.world.user(Caller::Coachee);

        assert_eq!(
            app.expect(
                Caller::Coachee,
                Method::GET,
                &format!("/users/{coachee}/goals"),
                None
            )
            .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// GOAL-24
#[tokio::test]
async fn only_the_user_lists_their_own_goals() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/users/{}/goals", app.world.user(Caller::Coachee));

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}
