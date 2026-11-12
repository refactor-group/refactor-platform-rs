//! A user's own actions (`ACT`) through `/users/{id}/actions`, on SQLite as each access-matrix
//! caller.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::remove_roles;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use crate::test_utils::sqlite::{ids, TestApp};

/// ACT-18 and ACT-19
async fn assert_own_actions(scope: &str, expected: impl Fn(&TestApp) -> BTreeSet<Id>) {
    let mut app = TestApp::new().await;
    let coachee = app.world.user(Caller::Coachee);
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
    let coachee = app.world.user(Caller::Coachee);
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
        let w = &b.world;
        ids([w.a1, w.a2, w.a3, w.a4, w.a5])
    }))
    .await;
}

/// ACT-19
#[tokio::test]
async fn a_coachee_lists_the_actions_assigned_to_them() {
    within_time_limit(assert_own_actions("?scope=assigned", |b| {
        ids([b.world.a2, b.world.a5])
    }))
    .await;
}

/// ACT-20
#[tokio::test]
async fn a_coach_lists_their_own_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let world = &app.world;
        let coach = world.user(Caller::Coach);
        let sessions = ids([world.a1, world.a2, world.a3, world.a6, world.a7]);
        let assigned = ids([world.a1]);

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
        let removed = app.world.user(Caller::Removed);

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
        ids([b.world.a1, b.world.a2, b.world.a3])
    }))
    .await;
}

/// ACT-23
#[tokio::test]
async fn a_coach_lists_only_their_relationships_assignments_of_the_coachee() {
    within_time_limit(assert_coachee_actions_as(
        Caller::Coach,
        "?scope=assigned",
        |b| ids([b.world.a2]),
    ))
    .await;
}

/// ACT-24
#[tokio::test]
async fn another_coach_lists_only_their_relationships_actions_for_the_coachee() {
    within_time_limit(assert_coachee_actions_as(Caller::OtherCoach, "", |b| {
        ids([b.world.a4, b.world.a5])
    }))
    .await;
}

/// ACT-25
#[tokio::test]
async fn members_who_do_not_coach_the_user_cannot_list_their_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/users/{}/actions", app.world.user(Caller::Coachee));

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
        let path = format!("/users/{}/actions", app.world.user(Caller::Coach));

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
        remove_roles(app.db(), app.world.user(Caller::Coach), app.world.o1).await;
        let path = format!("/users/{}/actions", app.world.user(Caller::Coachee));

        let (status, _) = app.expect(Caller::Coach, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}
