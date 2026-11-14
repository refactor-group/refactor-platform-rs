//! Which actions appear in an organization's per-relationship action lists, and who may ask for
//! them.
//!
//! Each test signs in as one kind of person, asks for a list against a real database, and
//! checks that it holds exactly the right records, or that the request is refused.
//! How the routes behave once allowed is tested in the tests in `actions_controller.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::seed_relationship_with_actions;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use crate::test_utils::sqlite::{ids, ids_by_key, TestApp};

/// The batch action list's ids per relationship.
async fn batch_actions(
    app: &mut TestApp,
    caller: Caller,
    query: &str,
) -> (StatusCode, BTreeMap<Id, BTreeSet<Id>>) {
    let path = format!(
        "/organizations/{}/coaching_relationships/actions{query}",
        app.fixture.o1
    );
    let reply = app.send(caller, Method::GET, &path, None).await;
    let by_relationship = match reply.status {
        StatusCode::OK => ids_by_key(&reply.body["data"]["coachee_actions"]),
        _ => BTreeMap::new(),
    };
    (reply.status, by_relationship)
}

/// The per-relationship action list path.
fn relationship_actions(app: &TestApp, relationship: Id, query: &str) -> String {
    format!(
        "/organizations/{}/coaching_relationships/{relationship}/actions{query}",
        app.fixture.o1
    )
}

/// ACT-01
#[tokio::test]
async fn a_coach_lists_actions_per_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let expected = BTreeMap::from([
            (fixture.r1, ids([fixture.a1, fixture.a2, fixture.a3])),
            (fixture.r3, ids([fixture.a6, fixture.a7])),
        ]);

        assert_eq!(
            batch_actions(&mut app, Caller::Coach, "").await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-02
#[tokio::test]
async fn a_coachee_lists_only_own_and_unassigned_actions_per_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let expected = BTreeMap::from([
            (fixture.r1, ids([fixture.a2, fixture.a3])),
            (fixture.r2, ids([fixture.a5])),
        ]);

        assert_eq!(
            batch_actions(&mut app, Caller::Coachee, "").await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-03
#[tokio::test]
async fn another_coach_lists_only_their_relationships_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let expected = BTreeMap::from([(fixture.r2, ids([fixture.a4, fixture.a5]))]);

        assert_eq!(
            batch_actions(&mut app, Caller::OtherCoach, "").await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-04
#[tokio::test]
async fn members_in_no_relationship_list_no_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;

        for caller in [Caller::Member, Caller::Admin, Caller::Super] {
            assert_eq!(
                batch_actions(&mut app, caller, "").await,
                (StatusCode::OK, BTreeMap::new()),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ACT-05
#[tokio::test]
async fn non_members_cannot_list_the_orgs_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;

        for caller in [Caller::Outsider, Caller::Removed] {
            let (status, _) = batch_actions(&mut app, caller, "").await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ACT-06
#[tokio::test]
async fn a_coachee_who_coaches_no_one_cannot_scope_to_the_coach() {
    within_time_limit(async {
        let mut app = TestApp::new().await;

        let (status, _) = batch_actions(&mut app, Caller::Coachee, "?assignee=coach").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-07
#[tokio::test]
async fn a_coach_lists_the_relationships_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = relationship_actions(&app, app.fixture.r1, "");
        let fixture = &app.fixture;
        let expected = ids([fixture.a1, fixture.a2, fixture.a3]);

        assert_eq!(
            app.expect(Caller::Coach, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-08
#[tokio::test]
async fn a_coachee_lists_only_own_and_unassigned_relationship_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = relationship_actions(&app, app.fixture.r1, "");
        let expected = ids([app.fixture.a2, app.fixture.a3]);

        assert_eq!(
            app.expect(Caller::Coachee, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-09
#[tokio::test]
async fn a_coachee_scopes_relationship_actions_only_to_themselves() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let coachee = app.fixture.user(Caller::Coachee);
        let a2 = app.fixture.a2;

        let path = relationship_actions(&app, r1, "?assignee=coach");
        let (status, _) = app.expect(Caller::Coachee, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "assignee=coach");

        for scope in ["coachee".to_string(), coachee.to_string()] {
            let path = relationship_actions(&app, r1, &format!("?assignee={scope}"));
            assert_eq!(
                app.expect(Caller::Coachee, Method::GET, &path, None).await,
                (StatusCode::OK, ids([a2])),
                "assignee={scope}"
            );
        }
    })
    .await;
}

/// ACT-10
#[tokio::test]
async fn non_participants_cannot_list_the_relationships_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = relationship_actions(&app, app.fixture.r1, "");

        for caller in [
            Caller::Member,
            Caller::OtherCoach,
            Caller::Admin,
            Caller::Super,
        ] {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ACT-11
#[tokio::test]
async fn an_outsider_cannot_list_the_relationships_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = relationship_actions(&app, app.fixture.r1, "");

        let (status, _) = app.expect(Caller::Outsider, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-12
#[tokio::test]
async fn a_removed_coachee_cannot_list_their_relationships_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = relationship_actions(&app, app.fixture.r3, "");

        let (status, _) = app.expect(Caller::Removed, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-13
#[tokio::test]
async fn a_coach_lists_actions_of_a_relationship_whose_coachee_left() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = relationship_actions(&app, app.fixture.r3, "");
        let expected = ids([app.fixture.a6, app.fixture.a7]);

        assert_eq!(
            app.expect(Caller::Coach, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-41
#[tokio::test]
async fn a_coachee_who_also_coaches_lists_each_relationship_by_their_role_in_it() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (coachee, member) = (
            app.fixture.user(Caller::Coachee),
            app.fixture.user(Caller::Member),
        );
        let (r4, r4_actions) =
            seed_relationship_with_actions(app.db(), app.fixture.o1, coachee, member).await;
        let fixture = &app.fixture;
        let expected = BTreeMap::from([
            (fixture.r1, ids([fixture.a2, fixture.a3])),
            (fixture.r2, ids([fixture.a5])),
            (r4, ids(r4_actions)),
        ]);

        assert_eq!(
            batch_actions(&mut app, Caller::Coachee, "").await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}
