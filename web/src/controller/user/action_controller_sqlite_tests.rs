//! Which actions appear in a person's action lists, and who may ask for them.
//!
//! Each test signs in as one kind of person, asks for a list against a real database, and
//! checks that it holds exactly the right records, or that the request is refused.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::remove_roles;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use crate::test_utils::sqlite::{ids, seed_super_coaching, TestApp};

/// ACT-18 and ACT-19
async fn assert_own_actions(scope: &str, expected: impl Fn(&TestApp) -> BTreeSet<Id>) {
    let mut app = TestApp::new().await;
    let coachee = app.fixture.user(Caller::Coachee);
    let expected = expected(&app);

    assert_eq!(
        app.expect(
            Caller::Coachee,
            Method::GET,
            &format!("/users/{coachee}/actions{scope}"),
            None
        )
        .await,
        (StatusCode::OK, expected)
    );
}

/// ACT-22, ACT-23 and ACT-24: `caller` lists the coachee's actions with `scope`.
async fn assert_coachee_actions_as(
    caller: Caller,
    scope: &str,
    expected: impl Fn(&TestApp) -> BTreeSet<Id>,
) {
    let mut app = TestApp::new().await;
    let coachee = app.fixture.user(Caller::Coachee);
    let expected = expected(&app);

    assert_eq!(
        app.expect(
            caller,
            Method::GET,
            &format!("/users/{coachee}/actions{scope}"),
            None
        )
        .await,
        (StatusCode::OK, expected)
    );
}

/// ACT-18
#[tokio::test]
async fn a_coachee_lists_every_action_of_their_sessions() {
    within_time_limit(assert_own_actions("", |b| {
        let w = &b.fixture;
        ids([w.a1, w.a2, w.a3, w.a4, w.a5])
    }))
    .await;
}

/// ACT-19
#[tokio::test]
async fn a_coachee_lists_the_actions_assigned_to_them() {
    within_time_limit(assert_own_actions("?scope=assigned", |b| {
        ids([b.fixture.a2, b.fixture.a5])
    }))
    .await;
}

/// ACT-20
#[tokio::test]
async fn a_coach_lists_their_own_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let coach = fixture.user(Caller::Coach);
        let sessions = ids([fixture.a1, fixture.a2, fixture.a3, fixture.a6, fixture.a7]);
        let assigned = ids([fixture.a1]);

        for (query, expected) in [("", sessions), ("?scope=assigned", assigned)] {
            assert_eq!(
                app.expect(
                    Caller::Coach,
                    Method::GET,
                    &format!("/users/{coach}/actions{query}"),
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

/// ACT-21
#[tokio::test]
async fn a_removed_coachee_lists_none_of_their_own_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let removed = app.fixture.user(Caller::Removed);

        assert_eq!(
            app.expect(
                Caller::Removed,
                Method::GET,
                &format!("/users/{removed}/actions"),
                None
            )
            .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// ACT-22
#[tokio::test]
async fn a_coach_lists_only_their_relationships_actions_for_the_coachee() {
    within_time_limit(assert_coachee_actions_as(Caller::Coach, "", |b| {
        ids([b.fixture.a1, b.fixture.a2, b.fixture.a3])
    }))
    .await;
}

/// ACT-23
#[tokio::test]
async fn a_coach_lists_only_their_relationships_assignments_of_the_coachee() {
    within_time_limit(assert_coachee_actions_as(
        Caller::Coach,
        "?scope=assigned",
        |b| ids([b.fixture.a2]),
    ))
    .await;
}

/// ACT-24
#[tokio::test]
async fn another_coach_lists_only_their_relationships_actions_for_the_coachee() {
    within_time_limit(assert_coachee_actions_as(Caller::OtherCoach, "", |b| {
        ids([b.fixture.a4, b.fixture.a5])
    }))
    .await;
}

/// ACT-25
#[tokio::test]
async fn members_who_do_not_coach_the_user_cannot_list_their_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/users/{}/actions", app.fixture.user(Caller::Coachee));

        for caller in [Caller::Member, Caller::Admin, Caller::Super] {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ACT-26
#[tokio::test]
async fn a_coachee_cannot_list_their_coachs_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/users/{}/actions", app.fixture.user(Caller::Coach));

        let (status, _) = app.expect(Caller::Coachee, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-26b
#[tokio::test]
async fn a_coach_removed_from_the_org_cannot_list_their_coachees_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        remove_roles(app.db(), app.fixture.user(Caller::Coach), app.fixture.o1).await;
        let path = format!("/users/{}/actions", app.fixture.user(Caller::Coachee));

        let (status, _) = app.expect(Caller::Coach, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-43
#[tokio::test]
async fn a_super_admin_lists_their_own_actions_in_an_org_they_hold_no_role_in() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let super_admin = app.fixture.user(Caller::Super);
        let seeded = seed_super_coaching(&app).await;
        let path = format!("/users/{super_admin}/actions");

        for (scope, expected) in [
            ("", ids(seeded.actions)),
            ("?scope=assigned", ids([seeded.own_action])),
        ] {
            assert_eq!(
                app.expect(Caller::Super, Method::GET, &format!("{path}{scope}"), None)
                    .await,
                (StatusCode::OK, expected),
                "{scope}"
            );
        }
    })
    .await;
}
