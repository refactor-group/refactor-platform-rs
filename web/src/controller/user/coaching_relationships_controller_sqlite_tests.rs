//! Which relationships appear in a person's relationship list, and who may ask for it.
//!
//! Each test signs in as one kind of person, asks for a list against a real database, and
//! checks that it holds exactly the right records, or that the request is refused.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::{ids, seed_super_coaching, TestApp};

/// REL-13
#[tokio::test]
async fn a_coachee_lists_their_own_relationships_by_role() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let coachee = fixture.user(Caller::Coachee);
        let both = ids([fixture.r1, fixture.r2]);
        let path = format!("/users/{coachee}/coaching-relationships");

        for (query, expected) in [
            ("", both.clone()),
            ("?role=coach", BTreeSet::new()),
            ("?role=coachee", both),
        ] {
            assert_eq!(
                app.expect(
                    Caller::Coachee,
                    Method::GET,
                    &format!("{path}{query}"),
                    None
                )
                .await,
                (StatusCode::OK, expected),
                "{query}"
            );
        }
    })
    .await;
}

/// REL-14
#[tokio::test]
async fn only_the_user_lists_their_own_relationships() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.fixture.user(Caller::Coachee);
        let path = format!("/users/{coachee}/coaching-relationships");

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// REL-17
#[tokio::test]
async fn a_removed_coachee_lists_none_of_their_own_relationships() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let removed = app.fixture.user(Caller::Removed);

        assert_eq!(
            app.expect(
                Caller::Removed,
                Method::GET,
                &format!("/users/{removed}/coaching-relationships"),
                None
            )
            .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// USR-02
#[tokio::test]
async fn a_coachee_coaches_no_one() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.fixture.user(Caller::Coachee);
        let path = format!("/users/{coachee}/coaching-relationships?role=coach");

        assert_eq!(
            app.expect(Caller::Coachee, Method::GET, &path, None).await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// USR-03
#[tokio::test]
async fn a_coach_lists_the_relationships_they_coach() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let coach = fixture.user(Caller::Coach);
        let expected = ids([fixture.r1, fixture.r3]);
        let path = format!("/users/{coach}/coaching-relationships?role=coach");

        assert_eq!(
            app.expect(Caller::Coach, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// REL-22
#[tokio::test]
async fn a_super_admin_lists_their_own_relationship_in_an_org_they_hold_no_role_in() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let super_admin = app.fixture.user(Caller::Super);
        let expected = ids([seed_super_coaching(&app).await.relationship]);
        let path = format!("/users/{super_admin}/coaching-relationships");

        assert_eq!(
            app.expect(Caller::Super, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}
