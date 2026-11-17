//! Who can list, read and create an organization's coaching relationships, and read their goal
//! progress.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::coaching_relationship_kind::Kind;
use domain::test_utils::sqlite::fixture::{
    archive_organization, make_colleagues, remove_roles, set_role,
};
use domain::test_utils::sqlite::read_back::relationships_between;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::users::Role;
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{
    assert_signed_out_refused, created_id, field_ids, ids, TestApp, NON_PARTICIPANTS,
};

/// The goal progress path for R1.
fn goal_progress(app: &TestApp, query: &str) -> String {
    format!(
        "/organizations/{}/coaching_relationships/{}/goal_progress{query}",
        app.fixture.o1, app.fixture.r1
    )
}

/// Asserts one caller's list of O1's relationships.
async fn assert_org_list(caller: Caller, expected: impl Fn(&TestApp) -> BTreeSet<Id>) {
    let mut app = TestApp::new().await;
    let path = format!("/organizations/{}/coaching_relationships", app.fixture.o1);
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
        let g1 = app.fixture.g1;
        let coachee = app.fixture.user(Caller::Coachee);
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
        let member = app.fixture.user(Caller::Member);
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

/// GOAL-36
#[tokio::test]
async fn colleagues_cannot_scope_goal_progress_to_a_seat() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        make_colleagues(app.db(), app.fixture.r1).await;
        let g1 = app.fixture.g1;
        let coachee = app.fixture.user(Caller::Coachee);

        for scope in ["coach", "coachee"] {
            let path = goal_progress(&app, &format!("?assignee={scope}"));
            let reply = app.send(Caller::Coach, Method::GET, &path, None).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{scope}");
            assert_eq!(
                reply.body["details"]["reason"], "assignee_scope_not_applicable",
                "{scope}: {}",
                reply.body
            );
        }

        let path = goal_progress(&app, &format!("?assignee={coachee}"));
        let reply = app.send(Caller::Coach, Method::GET, &path, None).await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(
            field_ids(&reply.body["data"]["goal_progress"], "goal_id"),
            ids([g1])
        );
    })
    .await;
}

/// REL-01
#[tokio::test]
async fn a_coach_lists_only_their_own_relationships() {
    within_time_limit(assert_org_list(Caller::Coach, |b| {
        ids([b.fixture.r1, b.fixture.r3])
    }))
    .await;
}

/// REL-02
#[tokio::test]
async fn a_coachee_lists_only_their_own_relationships() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let path = format!("/organizations/{}/coaching_relationships", fixture.o1);
        let expected = ids([fixture.r1, fixture.r2]);

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
    within_time_limit(assert_org_list(Caller::OtherCoach, |b| ids([b.fixture.r2]))).await;
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
        ids([b.fixture.r1, b.fixture.r2, b.fixture.r3])
    }))
    .await;
}

/// REL-06
#[tokio::test]
async fn a_super_admin_lists_every_relationship_in_the_org() {
    within_time_limit(assert_org_list(Caller::Super, |b| {
        ids([b.fixture.r1, b.fixture.r2, b.fixture.r3])
    }))
    .await;
}

/// REL-07
#[tokio::test]
async fn non_members_cannot_list_the_orgs_relationships() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/organizations/{}/coaching_relationships", app.fixture.o1);

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
        let (o1, r1) = (app.fixture.o1, app.fixture.r1);
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

/// REL-08
#[tokio::test]
async fn a_relationship_read_carries_its_kind() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r1) = (app.fixture.o1, app.fixture.r1);
        let path = format!("/organizations/{o1}/coaching_relationships/{r1}");

        let reply = app.send(Caller::Coach, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["data"]["kind"], json!("coaching"));
    })
    .await;
}

/// REL-09
#[tokio::test]
async fn non_participants_cannot_read_a_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r1) = (app.fixture.o1, app.fixture.r1);
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
        let (o2, r1) = (app.fixture.o2, app.fixture.r1);
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
        let (o1, r3) = (app.fixture.o1, app.fixture.r3);
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
        let (o1, r3) = (app.fixture.o1, app.fixture.r3);
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
                app.fixture.user(Caller::Coach),
                app.fixture.user(Caller::Member),
            );
            let path = format!("/organizations/{}/coaching_relationships", app.fixture.o1);
            let body = json!({ "coach_id": coach, "coachee_id": member });

            let created = app
                .send(caller, Method::POST, &path, Some(body.clone()))
                .await;
            assert_eq!(created.status, StatusCode::OK, "{caller:?}");
            assert_eq!(created.body["status_code"], 201, "{caller:?}");
            let stored: BTreeSet<Id> = relationships_between(app.db(), coach, member)
                .await
                .into_iter()
                .map(|relationship| relationship.id)
                .collect();
            assert_eq!(stored.len(), 1, "{caller:?}");
            assert_eq!(created.ids(), stored, "{caller:?}");

            let again = app.send(caller, Method::POST, &path, Some(body)).await;
            assert_eq!(again.status, StatusCode::OK, "{caller:?} again");
            assert_eq!(again.ids(), stored, "{caller:?} again");
            assert_eq!(
                relationships_between(app.db(), coach, member)
                    .await
                    .into_iter()
                    .map(|relationship| relationship.id)
                    .collect::<BTreeSet<_>>(),
                stored,
                "{caller:?} again"
            );
        }
    })
    .await;
}

/// O1's relationship create path.
fn create_path(app: &TestApp) -> String {
    format!("/organizations/{}/coaching_relationships", app.fixture.o1)
}

/// Creates a relationship as O1's admin and returns its id.
async fn admin_creates(app: &mut TestApp, body: serde_json::Value) -> Id {
    let path = create_path(app);
    let reply = app
        .send(Caller::Admin, Method::POST, &path, Some(body))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    created_id(&reply)
}

/// Every relationship between `a` and `b`, in either direction: its coach and kind by id.
async fn stored_between(app: &TestApp, a: Id, b: Id) -> BTreeMap<Id, (Id, Kind)> {
    let mut stored = relationships_between(app.db(), a, b).await;
    stored.extend(relationships_between(app.db(), b, a).await);
    stored
        .into_iter()
        .map(|relationship| (relationship.id, (relationship.coach_id, relationship.kind)))
        .collect()
}

/// Asserts an admin's create is refused because the pair holds `existing`, and stores nothing.
async fn assert_pair_already_related(
    app: &mut TestApp,
    body: serde_json::Value,
    (a, b): (Id, Id),
    existing: Id,
) {
    let before = stored_between(app, a, b).await;
    let path = create_path(app);

    let reply = app
        .send(Caller::Admin, Method::POST, &path, Some(body.clone()))
        .await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        reply.body["details"]["reason"], "pair_already_related",
        "{body}"
    );
    assert_eq!(
        reply.body["details"]["relationship_id"],
        existing.to_string(),
        "{body}"
    );
    assert_eq!(stored_between(app, a, b).await, before, "{body}");
}

/// KIND-04
#[tokio::test]
async fn an_admin_creates_a_colleague_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (a, b) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );
        let path = create_path(&app);
        let body = json!({ "coach_id": a, "coachee_id": b, "kind": "colleague" });

        let created = app
            .send(Caller::Admin, Method::POST, &path, Some(body))
            .await;

        assert_eq!(created.status, StatusCode::OK);
        assert_eq!(created.body["status_code"], 201);
        assert_eq!(created.body["data"]["kind"], "colleague");
        assert_eq!(
            stored_between(&app, a, b).await,
            BTreeMap::from([(created_id(&created), (a, Kind::Colleague))])
        );
    })
    .await;
}

/// KIND-11
#[tokio::test]
async fn a_colleague_create_returns_the_pairs_colleague_relationship_either_way_round() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (a, b) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );
        let first = admin_creates(
            &mut app,
            json!({ "coach_id": a, "coachee_id": b, "kind": "colleague" }),
        )
        .await;

        let again = admin_creates(
            &mut app,
            json!({ "coach_id": b, "coachee_id": a, "kind": "colleague" }),
        )
        .await;

        assert_eq!(again, first);
        assert_eq!(
            stored_between(&app, a, b).await,
            BTreeMap::from([(first, (a, Kind::Colleague))])
        );
    })
    .await;
}

/// KIND-12
#[tokio::test]
async fn a_colleague_create_is_refused_for_a_coaching_pair() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (a, b) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );
        let coaching = admin_creates(&mut app, json!({ "coach_id": a, "coachee_id": b })).await;

        for (coach_id, coachee_id) in [(a, b), (b, a)] {
            assert_pair_already_related(
                &mut app,
                json!({ "coach_id": coach_id, "coachee_id": coachee_id, "kind": "colleague" }),
                (a, b),
                coaching,
            )
            .await;
        }
        assert_eq!(
            stored_between(&app, a, b).await,
            BTreeMap::from([(coaching, (a, Kind::Coaching))])
        );
    })
    .await;
}

/// KIND-13
#[tokio::test]
async fn a_coaching_create_is_refused_for_a_colleague_pair() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (a, b) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );
        let colleague = admin_creates(
            &mut app,
            json!({ "coach_id": a, "coachee_id": b, "kind": "colleague" }),
        )
        .await;

        for (coach_id, coachee_id) in [(a, b), (b, a)] {
            assert_pair_already_related(
                &mut app,
                json!({ "coach_id": coach_id, "coachee_id": coachee_id, "kind": "coaching" }),
                (a, b),
                colleague,
            )
            .await;
        }
        assert_eq!(
            stored_between(&app, a, b).await,
            BTreeMap::from([(colleague, (a, Kind::Colleague))])
        );
    })
    .await;
}

/// KIND-14
#[tokio::test]
async fn a_coaching_pair_may_also_coach_the_other_way_round() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (a, b) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );
        let forward = admin_creates(&mut app, json!({ "coach_id": a, "coachee_id": b })).await;

        let reverse = admin_creates(&mut app, json!({ "coach_id": b, "coachee_id": a })).await;
        let again = admin_creates(&mut app, json!({ "coach_id": a, "coachee_id": b })).await;

        assert_ne!(reverse, forward);
        assert_eq!(again, forward);
        assert_eq!(
            stored_between(&app, a, b).await,
            BTreeMap::from([
                (forward, (a, Kind::Coaching)),
                (reverse, (b, Kind::Coaching))
            ])
        );
    })
    .await;
}

/// REL-16
#[tokio::test]
async fn non_admins_cannot_create_a_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (coach, member) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );
        let path = format!("/organizations/{}/coaching_relationships", app.fixture.o1);
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
            assert!(
                relationships_between(app.db(), coach, member)
                    .await
                    .is_empty(),
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
        let fixture = &app.fixture;
        let path = format!("/organizations/{}/coaching_relationships", fixture.o1);
        let expected = BTreeMap::from([
            (fixture.r1, (true, true)),
            (fixture.r2, (true, true)),
            (fixture.r3, (true, false)),
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

/// REL-19
#[tokio::test]
async fn an_unknown_organizations_relationships_are_not_found() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/organizations/{}/coaching_relationships", Id::new_v4());

        for caller in [Caller::Coach, Caller::Admin, Caller::Super] {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{caller:?}");
        }
    })
    .await;
}

/// REL-20
#[tokio::test]
async fn an_admin_of_another_org_cannot_create_a_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (coach, member, outsider) = (
            fixture.user(Caller::Coach),
            fixture.user(Caller::Member),
            fixture.user(Caller::Outsider),
        );
        let (o1, o2) = (fixture.o1, fixture.o2);
        set_role(app.db(), outsider, Role::Admin, o2).await;

        let (status, _) = app
            .expect(
                Caller::Outsider,
                Method::POST,
                &format!("/organizations/{o1}/coaching_relationships"),
                Some(json!({ "coach_id": coach, "coachee_id": member })),
            )
            .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(relationships_between(app.db(), coach, member)
            .await
            .is_empty());
    })
    .await;
}

/// An admin's create is a conflict for a self pair, a participant outside the organization,
/// a pair already related by another kind, or an archived organization, and stores nothing.
#[tokio::test]
async fn an_unrelatable_pair_is_a_conflict() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (o1, admin) = (fixture.o1, fixture.user(Caller::Admin));
        let (coach, member, outsider) = (
            fixture.user(Caller::Coach),
            fixture.user(Caller::Member),
            fixture.user(Caller::Outsider),
        );
        let path = format!("/organizations/{o1}/coaching_relationships");
        admin_creates(
            &mut app,
            json!({ "coach_id": coach, "coachee_id": member, "kind": "colleague" }),
        )
        .await;

        for (case, coachee_id) in [
            ("self", coach),
            ("outsider", outsider),
            ("another kind", member),
        ] {
            let before = relationships_between(app.db(), coach, coachee_id).await;
            let (status, _) = app
                .expect(
                    Caller::Admin,
                    Method::POST,
                    &path,
                    Some(json!({ "coach_id": coach, "coachee_id": coachee_id })),
                )
                .await;
            assert_eq!(status, StatusCode::CONFLICT, "{case}");
            assert_eq!(
                relationships_between(app.db(), coach, coachee_id).await,
                before,
                "{case}"
            );
        }

        let other_coach = app.fixture.user(Caller::OtherCoach);
        archive_organization(app.db(), o1, admin).await;
        let (status, _) = app
            .expect(
                Caller::Admin,
                Method::POST,
                &path,
                Some(json!({ "coach_id": other_coach, "coachee_id": member })),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "archived");
        assert!(relationships_between(app.db(), other_coach, member)
            .await
            .is_empty());
    })
    .await;
}

/// An admin's create naming an unknown user is not found.
#[tokio::test]
async fn a_create_naming_an_unknown_user_is_not_found() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coach = app.fixture.user(Caller::Coach);
        let path = format!("/organizations/{}/coaching_relationships", app.fixture.o1);

        let (status, _) = app
            .expect(
                Caller::Admin,
                Method::POST,
                &path,
                Some(json!({ "coach_id": coach, "coachee_id": Id::new_v4() })),
            )
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}

/// REL-21
#[tokio::test]
async fn signed_out_callers_cannot_reach_relationships() {
    within_time_limit(assert_signed_out_refused(|app| {
        let (o1, r1) = (app.fixture.o1, app.fixture.r1);
        let relationships = format!("/organizations/{o1}/coaching_relationships");
        vec![
            (Method::GET, relationships.clone()),
            (Method::POST, relationships.clone()),
            (Method::GET, format!("{relationships}/{r1}")),
            (
                Method::GET,
                format!(
                    "/users/{}/coaching-relationships",
                    app.fixture.user(Caller::Coach)
                ),
            ),
        ]
    }))
    .await;
}

/// GETs `relationship` in O1 as `caller` and returns its data.
async fn read_as(app: &mut TestApp, caller: Caller, relationship: Id) -> serde_json::Value {
    let path = format!(
        "/organizations/{}/coaching_relationships/{relationship}",
        app.fixture.o1
    );
    let reply = app.send(caller, Method::GET, &path, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{caller:?}: {}", reply.body);
    reply.body["data"].clone()
}

/// KIND-19
#[tokio::test]
async fn a_coaching_relationship_tells_each_participant_what_they_may_do() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;

        for (caller, viewer) in [
            (
                Caller::Coach,
                json!({ "role": "coach", "can_schedule_sessions": true, "sees_all_actions": true }),
            ),
            (
                Caller::Coachee,
                json!({ "role": "coachee", "can_schedule_sessions": false, "sees_all_actions": false }),
            ),
        ] {
            let data = read_as(&mut app, caller, r1).await;
            assert_eq!(data["viewer"], viewer, "{caller:?}");
            assert_eq!(data["suggests_actions"], true, "{caller:?}");
        }
    })
    .await;
}

/// KIND-20
#[tokio::test]
async fn a_colleague_relationship_tells_each_colleague_they_may_do_everything() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        make_colleagues(app.db(), r1).await;

        for caller in [Caller::Coach, Caller::Coachee] {
            let data = read_as(&mut app, caller, r1).await;
            assert_eq!(
                data["viewer"],
                json!({ "role": "colleague", "can_schedule_sessions": true, "sees_all_actions": true }),
                "{caller:?}"
            );
            assert_eq!(data["suggests_actions"], false, "{caller:?}");
        }
    })
    .await;
}

/// KIND-21
#[tokio::test]
async fn an_admin_is_no_viewer_of_the_relationships_they_list() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/organizations/{}/coaching_relationships", app.fixture.o1);

        let reply = app.send(Caller::Admin, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        let listed = reply.body["data"]
            .as_array()
            .expect("a list of relationships");
        assert_eq!(listed.len(), 3, "{}", reply.body);
        for relationship in listed {
            assert!(relationship["viewer"].is_null(), "{relationship}");
        }
    })
    .await;
}

/// KIND-24
#[tokio::test]
async fn scheduling_is_blocked_by_the_participant_who_left() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r1, r3) = (app.fixture.o1, app.fixture.r1, app.fixture.r3);
        let (coach, removed) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Removed),
        );

        let data = read_as(&mut app, Caller::Coach, r1).await;
        assert!(data["scheduling_blocked_by"].is_null(), "{data}");

        let data = read_as(&mut app, Caller::Coach, r3).await;
        assert_eq!(data["scheduling_blocked_by"], removed.to_string(), "{data}");

        make_colleagues(app.db(), r1).await;
        remove_roles(app.db(), coach, o1).await;
        let data = read_as(&mut app, Caller::Coachee, r1).await;
        assert_eq!(data["scheduling_blocked_by"], coach.to_string(), "{data}");
    })
    .await;
}

/// KIND-26
#[tokio::test]
async fn a_created_relationship_names_no_viewer_for_the_admin() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let body = json!({
            "coach_id": app.fixture.user(Caller::Coach),
            "coachee_id": app.fixture.user(Caller::Member),
        });
        let path = create_path(&app);

        let created = app
            .send(Caller::Admin, Method::POST, &path, Some(body))
            .await;

        assert_eq!(created.status, StatusCode::OK, "{}", created.body);
        assert!(created.body["data"]["viewer"].is_null(), "{}", created.body);
        assert_eq!(created.body["data"]["suggests_actions"], true);
        assert!(created.body["data"]["scheduling_blocked_by"].is_null());
    })
    .await;
}
