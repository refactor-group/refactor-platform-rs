//! Which goals appear in a person's goal list, and who may ask for it.
//!
//! Each test signs in as one kind of person, asks for a list against a real database, and
//! checks that it holds exactly the right records, or that the request is refused.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::seed_goal;
use domain::test_utils::sqlite::read_back::relationship_goals;
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::{ids, TestApp};

/// GOAL-22
#[tokio::test]
async fn a_coach_lists_the_goals_they_created() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coach = app.fixture.user(Caller::Coach);
        let expected = ids([app.fixture.g1]);

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
        let coachee = app.fixture.user(Caller::Coachee);

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
        let path = format!("/users/{}/goals", app.fixture.user(Caller::Coachee));

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-34
#[tokio::test]
async fn a_removed_coachee_lists_none_of_their_own_goals() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (removed, r3, s3) = (
            app.fixture.user(Caller::Removed),
            app.fixture.r3,
            app.fixture.s3,
        );
        let own_goal = seed_goal(app.db(), r3, s3, removed).await;
        assert_eq!(relationship_goals(app.db(), r3).await, ids([own_goal]));

        assert_eq!(
            app.expect(
                Caller::Removed,
                Method::GET,
                &format!("/users/{removed}/goals"),
                None
            )
            .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}
