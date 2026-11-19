//! Which relationships appear in a person's relationship list, and who may ask for it.
//!
//! Each test signs in as one kind of person, asks for a list against a real database, and
//! checks that it holds exactly the right records, or that the request is refused.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::make_colleagues;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use serde_json::json;

use crate::test_utils::sqlite::{assert_emails_follow_seats, ids, seed_super_coaching, TestApp};

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

/// KIND-50: `coach` and `coachee` name only coaching relationships; `colleague` names only
/// colleague ones, even where the user holds the coach seat.
#[tokio::test]
async fn a_coach_with_a_colleague_relationship_lists_each_under_its_own_role() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, r3) = (app.fixture.r1, app.fixture.r3);
        make_colleagues(app.db(), r3).await;
        let coach = app.fixture.user(Caller::Coach);
        let path = format!("/users/{coach}/coaching-relationships");

        for (query, expected) in [
            ("?role=coach", ids([r1])),
            ("?role=coachee", BTreeSet::new()),
            ("?role=colleague", ids([r3])),
            ("?role=all", ids([r1, r3])),
            ("", ids([r1, r3])),
        ] {
            assert_eq!(
                app.expect(Caller::Coach, Method::GET, &format!("{path}{query}"), None)
                    .await,
                (StatusCode::OK, expected),
                "{query}"
            );
        }
    })
    .await;
}

/// KIND-50
#[tokio::test]
async fn a_colleague_in_the_coachee_seat_lists_it_as_a_colleague() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, r2) = (app.fixture.r1, app.fixture.r2);
        make_colleagues(app.db(), r1).await;
        let coachee = app.fixture.user(Caller::Coachee);
        let path = format!("/users/{coachee}/coaching-relationships");

        for (query, expected) in [("?role=colleague", ids([r1])), ("?role=coachee", ids([r2]))] {
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

/// KIND-25
#[tokio::test]
async fn a_coachee_is_told_what_they_may_do_in_each_listed_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.fixture.user(Caller::Coachee);
        let path = format!("/users/{coachee}/coaching-relationships");

        let reply = app.send(Caller::Coachee, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        let listed = reply.body["data"].as_array().expect("a list of relationships");
        assert_eq!(listed.len(), 2, "{}", reply.body);
        for relationship in listed {
            assert_eq!(
                relationship["viewer"],
                json!({ "role": "coachee", "can_schedule_sessions": false, "sees_all_actions": false }),
                "{relationship}"
            );
        }
    })
    .await;
}

/// KIND-45
#[tokio::test]
async fn a_users_list_carries_each_relationships_participants_emails() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coach = app.fixture.user(Caller::Coach);
        let path = format!("/users/{coach}/coaching-relationships");

        let reply = app.send(Caller::Coach, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_emails_follow_seats(&app.fixture, &reply.body["data"]);
    })
    .await;
}
