//! Who can list, read and create an organization's coaching relationships, and read their goal
//! progress.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::relationships_between;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{field_ids, ids, TestApp, NON_PARTICIPANTS};

/// The goal progress path for R1.
fn goal_progress(app: &TestApp, query: &str) -> String {
    format!(
        "/organizations/{}/coaching_relationships/{}/goal_progress{query}",
        app.world.o1, app.world.r1
    )
}

/// Asserts one caller's list of O1's relationships.
async fn assert_org_list(caller: Caller, expected: impl Fn(&TestApp) -> BTreeSet<Id>) {
    let mut app = TestApp::new().await;
    let path = format!("/organizations/{}/coaching_relationships", app.world.o1);
    let expected = expected(&app);

    assert_eq!(
        app.expect(caller, Method::GET, &path, None).await,
        (StatusCode::OK, expected),
        "{caller:?}"
    );
}

/// GOAL-19
#[tokio::test]
async fn participants_read_the_relationships_goal_progress() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let g1 = app.world.g1;
        let coachee = app.world.user(Caller::Coachee);
        let queries = [
            String::new(),
            "?assignee=coach".to_string(),
            "?assignee=coachee".to_string(),
            format!("?assignee={coachee}"),
        ];

        for caller in [Caller::Coach, Caller::Coachee] {
            for query in &queries {
                let path = goal_progress(&app, query);
                let reply = app.send(caller, Method::GET, &path, None).await;
                assert_eq!(reply.status, StatusCode::OK, "{caller:?} {query}");
                assert_eq!(
                    field_ids(&reply.body["data"]["goal_progress"], "goal_id"),
                    ids([g1]),
                    "{caller:?} {query}"
                );
            }
        }
    })
    .await;
}

/// GOAL-20
#[tokio::test]
async fn goal_progress_refuses_a_non_participant_assignee() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let member = app.world.user(Caller::Member);
        let path = goal_progress(&app, &format!("?assignee={member}"));

        let (status, _) = app.expect(Caller::Coach, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    })
    .await;
}

/// GOAL-21
#[tokio::test]
async fn non_participants_cannot_read_the_relationships_goal_progress() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = goal_progress(&app, "");

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// REL-01
#[tokio::test]
async fn a_coach_lists_only_their_own_relationships() {
    within_time_limit(assert_org_list(Caller::Coach, |b| {
        ids([b.world.r1, b.world.r3])
    }))
    .await;
}

/// REL-02
#[tokio::test]
async fn a_coachee_lists_only_their_own_relationships() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let world = &app.world;
        let path = format!("/organizations/{}/coaching_relationships", world.o1);
        let expected = ids([world.r1, world.r2]);

        assert_eq!(
            app.expect(Caller::Coachee, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// REL-03
#[tokio::test]
async fn another_coach_lists_only_their_relationship() {
    within_time_limit(assert_org_list(Caller::OtherCoach, |b| ids([b.world.r2]))).await;
}

/// REL-04
#[tokio::test]
async fn a_member_in_no_relationship_lists_none() {
    within_time_limit(assert_org_list(Caller::Member, |_| BTreeSet::new())).await;
}

/// REL-05
#[tokio::test]
async fn an_admin_lists_every_relationship_in_the_org() {
    within_time_limit(assert_org_list(Caller::Admin, |b| {
        ids([b.world.r1, b.world.r2, b.world.r3])
    }))
    .await;
}

/// REL-06
#[tokio::test]
async fn a_super_admin_lists_every_relationship_in_the_org() {
    within_time_limit(assert_org_list(Caller::Super, |b| {
        ids([b.world.r1, b.world.r2, b.world.r3])
    }))
    .await;
}

/// REL-07
#[tokio::test]
async fn non_members_cannot_list_the_orgs_relationships() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/organizations/{}/coaching_relationships", app.world.o1);

        for caller in [Caller::Outsider, Caller::Removed] {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// REL-08
#[tokio::test]
async fn participants_read_their_relationship_with_names() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r1) = (app.world.o1, app.world.r1);
        let path = format!("/organizations/{o1}/coaching_relationships/{r1}");

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app.send(caller, Method::GET, &path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.ids(), ids([r1]), "{caller:?}");
            let data = &reply.body["data"];
            assert_eq!(data["coach_first_name"], "Coach", "{caller:?}");
            assert_eq!(data["coachee_first_name"], "Coachee", "{caller:?}");
        }
    })
    .await;
}

/// REL-09
#[tokio::test]
async fn non_participants_cannot_read_a_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r1) = (app.world.o1, app.world.r1);
        let path = format!("/organizations/{o1}/coaching_relationships/{r1}");

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// REL-10
#[tokio::test]
async fn a_relationship_under_another_org_is_not_found() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o2, r1) = (app.world.o2, app.world.r1);
        let path = format!("/organizations/{o2}/coaching_relationships/{r1}");

        let (status, _) = app.expect(Caller::Coach, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}

/// REL-11
#[tokio::test]
async fn a_removed_coachee_cannot_read_their_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r3) = (app.world.o1, app.world.r3);
        let path = format!("/organizations/{o1}/coaching_relationships/{r3}");

        let (status, _) = app.expect(Caller::Removed, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// REL-12
#[tokio::test]
async fn a_coach_keeps_a_relationship_whose_coachee_left() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r3) = (app.world.o1, app.world.r3);
        let path = format!("/organizations/{o1}/coaching_relationships/{r3}");

        let reply = app.send(Caller::Coach, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.ids(), ids([r3]));
        assert_eq!(reply.body["data"]["coach_is_member"], true);
        assert_eq!(reply.body["data"]["coachee_is_member"], false);
    })
    .await;
}

/// REL-15
#[tokio::test]
async fn admins_create_a_relationship_once_per_pair() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let (coach, member) = (
                app.world.user(Caller::Coach),
                app.world.user(Caller::Member),
            );
            let path = format!("/organizations/{}/coaching_relationships", app.world.o1);
            let body = json!({ "coach_id": coach, "coachee_id": member });

            let created = app
                .send(caller, Method::POST, &path, Some(body.clone()))
                .await;
            assert_eq!(created.status, StatusCode::OK, "{caller:?}");
            assert_eq!(created.body["status_code"], 201, "{caller:?}");
            let stored = relationships_between(app.db(), coach, member).await;
            assert_eq!(stored.len(), 1, "{caller:?}");
            assert_eq!(created.ids(), stored, "{caller:?}");

            let again = app.send(caller, Method::POST, &path, Some(body)).await;
            assert_eq!(again.status, StatusCode::OK, "{caller:?} again");
            assert_eq!(again.ids(), stored, "{caller:?} again");
            assert_eq!(
                relationships_between(app.db(), coach, member).await,
                stored,
                "{caller:?} again"
            );
        }
    })
    .await;
}

/// REL-16
#[tokio::test]
async fn non_admins_cannot_create_a_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (coach, member) = (
            app.world.user(Caller::Coach),
            app.world.user(Caller::Member),
        );
        let path = format!("/organizations/{}/coaching_relationships", app.world.o1);
        let body = json!({ "coach_id": coach, "coachee_id": member });

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = app
                .expect(caller, Method::POST, &path, Some(body.clone()))
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                relationships_between(app.db(), coach, member).await,
                BTreeSet::new(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// REL-18
#[tokio::test]
async fn an_admin_sees_who_is_still_a_member() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let world = &app.world;
        let path = format!("/organizations/{}/coaching_relationships", world.o1);
        let expected = BTreeMap::from([
            (world.r1, (true, true)),
            (world.r2, (true, true)),
            (world.r3, (true, false)),
        ]);

        let reply = app.send(Caller::Admin, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        let membership: BTreeMap<Id, (bool, bool)> = reply.body["data"]
            .as_array()
            .expect("a list of relationships")
            .iter()
            .map(|relationship| {
                let id = relationship["id"]
                    .as_str()
                    .and_then(|id| id.parse().ok())
                    .expect("each relationship has an id");
                let flag = |field: &str| relationship[field].as_bool().expect("a membership flag");
                (id, (flag("coach_is_member"), flag("coachee_is_member")))
            })
            .collect();
        assert_eq!(membership, expected);
    })
    .await;
}
