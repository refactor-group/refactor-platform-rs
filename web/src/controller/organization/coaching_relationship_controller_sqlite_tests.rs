//! Who can list, read and create an organization's coaching relationships, and read their goal
//! progress.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use chrono::TimeDelta;
use domain::coaching_relationship_kind::Kind;
use domain::test_utils::sqlite::fixture::{
    archive_organization, make_colleagues, remove_roles, seed_action_assigned_to,
    seed_content_free_session, seed_session_starting_in, set_role,
};
use domain::test_utils::sqlite::read_back::{
    pending_invites, relationship_row, relationships_between, reminders_sent, session_schedule,
    verified_audit_trail, AuditEntry,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::users::Role;
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{
    assert_emails_follow_seats, assert_signed_out_refused, created_id, field_ids, ids, Listener,
    Reply, TestApp, NON_PARTICIPANTS,
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
            assert_eq!(data["participants"][0]["first_name"], "Coach", "{caller:?}");
            assert_eq!(
                data["participants"][1]["first_name"], "Coachee",
                "{caller:?}"
            );
        }
    })
    .await;
}

/// The keys `participants` replaced.
const SEAT_NAMED_KEYS: [&str; 8] = [
    "coach_first_name",
    "coach_last_name",
    "coachee_first_name",
    "coachee_last_name",
    "coach_email",
    "coachee_email",
    "coach_is_member",
    "coachee_is_member",
];

/// KIND-48
#[tokio::test]
async fn a_coaching_relationship_lists_its_coach_then_its_coachee() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let (coach, coachee) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );

        let data = read_as(&mut app, Caller::Coach, r1).await;

        assert_eq!(
            data["participants"],
            json!([
                {
                    "id": coach,
                    "first_name": "Coach",
                    "last_name": "Access",
                    "email": Caller::Coach.email(),
                    "role": "coach",
                    "is_member": true,
                },
                {
                    "id": coachee,
                    "first_name": "Coachee",
                    "last_name": "Access",
                    "email": Caller::Coachee.email(),
                    "role": "coachee",
                    "is_member": true,
                },
            ]),
            "{data}"
        );
        for key in SEAT_NAMED_KEYS {
            assert!(data.get(key).is_none(), "{key}: {data}");
        }
    })
    .await;
}

/// KIND-48
#[tokio::test]
async fn a_colleague_relationship_lists_both_as_colleagues_coach_seat_first() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        make_colleagues(app.db(), r1).await;
        let (coach, coachee) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );

        let data = read_as(&mut app, Caller::Coachee, r1).await;

        let participants = &data["participants"];
        assert_eq!(
            (&participants[0]["id"], &participants[0]["role"]),
            (&json!(coach), &json!("colleague")),
            "{data}"
        );
        assert_eq!(
            (&participants[1]["id"], &participants[1]["role"]),
            (&json!(coachee), &json!("colleague")),
            "{data}"
        );
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

/// KIND-45
#[tokio::test]
async fn a_relationship_read_carries_each_participants_email() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;

        let data = read_as(&mut app, Caller::Coach, r1).await;

        assert_eq!(
            data["participants"][0]["email"],
            Caller::Coach.email(),
            "{data}"
        );
        assert_eq!(
            data["participants"][1]["email"],
            Caller::Coachee.email(),
            "{data}"
        );
    })
    .await;
}

/// KIND-45
#[tokio::test]
async fn an_admins_list_carries_each_relationships_participants_emails() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/organizations/{}/coaching_relationships", app.fixture.o1);

        let reply = app.send(Caller::Admin, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_emails_follow_seats(&app.fixture, &reply.body["data"]);
    })
    .await;
}

/// KIND-45
#[tokio::test]
async fn a_members_list_carries_each_relationships_participants_emails() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/organizations/{}/coaching_relationships", app.fixture.o1);

        let reply = app.send(Caller::Coach, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_emails_follow_seats(&app.fixture, &reply.body["data"]);
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
        let participants = &reply.body["data"]["participants"];
        assert_eq!(
            participants[0]["id"],
            json!(app.fixture.user(Caller::Coach)),
            "{participants}"
        );
        assert_eq!(participants[0]["is_member"], true, "{participants}");
        assert_eq!(
            participants[1]["id"],
            json!(app.fixture.user(Caller::Removed)),
            "{participants}"
        );
        assert_eq!(participants[1]["is_member"], false, "{participants}");
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
                let flag = |seat: usize| {
                    relationship["participants"][seat]["is_member"]
                        .as_bool()
                        .expect("a membership flag")
                };
                (id, (flag(0), flag(1)))
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
async fn a_colleague_relationship_tells_each_colleague_they_may_do_everything_and_suggests_actions()
{
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
            assert_eq!(data["suggests_actions"], true, "{caller:?}");
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

/// PUTs `body` to `relationship`'s kind in `organization` as `caller`.
async fn switch_kind_in(
    app: &mut TestApp,
    caller: Caller,
    organization: Id,
    relationship: Id,
    body: serde_json::Value,
) -> Reply {
    let path = format!("/organizations/{organization}/coaching_relationships/{relationship}/kind");
    app.send(caller, Method::PUT, &path, Some(body)).await
}

/// PUTs `body` to `relationship`'s kind in O1 as O1's admin.
async fn admin_switches(app: &mut TestApp, relationship: Id, body: serde_json::Value) -> Reply {
    let o1 = app.fixture.o1;
    switch_kind_in(app, Caller::Admin, o1, relationship, body).await
}

/// Asserts `reply` is a 409 or 422 with `reason` and `relationship` is stored as `before`.
async fn assert_refused_unchanged(
    app: &TestApp,
    reply: &Reply,
    (status, reason): (StatusCode, &str),
    relationship: Id,
    before: &Option<domain::coaching_relationships::Model>,
) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(reply.body["details"]["reason"], reason, "{}", reply.body);
    assert_eq!(&relationship_row(app.db(), relationship).await, before);
}

/// KIND-28
#[tokio::test]
async fn an_unconfirmed_switch_to_colleague_lists_its_consequences_and_changes_nothing() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let (coach, coachee) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );
        seed_action_assigned_to(app.db(), r1, &[coach]).await;
        let before = relationship_row(app.db(), r1).await;

        let reply = admin_switches(&mut app, r1, json!({ "kind": "colleague" })).await;

        assert_refused_unchanged(
            &app,
            &reply,
            (StatusCode::CONFLICT, "kind_switch_needs_confirmation"),
            r1,
            &before,
        )
        .await;
        let details = &reply.body["details"];
        assert_eq!(
            details["actions_becoming_visible"],
            json!([{ "user_id": coachee, "by_assignee": [{ "assignee_id": coach, "count": 2 }] }]),
            "A1 and the seeded action, both the coach's: {details}"
        );
        assert_eq!(details["scheduling_gained"], json!([coachee]), "{details}");
        assert_eq!(details["scheduling_lost"], json!([]), "{details}");
    })
    .await;
}

/// KIND-29
#[tokio::test]
async fn a_confirmed_switch_to_colleague_gives_the_former_coachee_everything() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;

        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "colleague", "confirmed": true }),
        )
        .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.ids(), ids([r1]));
        assert_eq!(reply.body["data"]["kind"], "colleague");
        assert!(reply.body["data"]["viewer"].is_null(), "{}", reply.body);
        assert_eq!(
            relationship_row(app.db(), r1).await.map(|row| row.kind),
            Some(Kind::Colleague)
        );
        let data = read_as(&mut app, Caller::Coachee, r1).await;
        assert_eq!(
            data["viewer"],
            json!({ "role": "colleague", "can_schedule_sessions": true, "sees_all_actions": true })
        );
    })
    .await;
}

/// KIND-30
#[tokio::test]
async fn a_confirmed_switch_to_coaching_seats_the_chosen_coach() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let (coach, coachee) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );
        make_colleagues(app.db(), r1).await;

        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "coaching", "coach_id": coachee, "confirmed": true }),
        )
        .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.body["data"]["coach_id"], coachee.to_string());
        assert_eq!(
            relationship_row(app.db(), r1).await.map(|row| (
                row.kind,
                row.coach_id,
                row.coachee_id
            )),
            Some((Kind::Coaching, coachee, coach))
        );
    })
    .await;
}

/// KIND-31
#[tokio::test]
async fn a_switch_naming_the_coach_wrongly_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let (coach, member) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );
        let before = relationship_row(app.db(), r1).await;

        for (body, reason) in [
            (
                json!({ "kind": "coaching", "coach_id": member, "confirmed": true }),
                "coach_not_a_participant",
            ),
            (
                json!({ "kind": "coaching", "confirmed": true }),
                "coach_required",
            ),
            (
                json!({ "kind": "colleague", "coach_id": coach, "confirmed": true }),
                "coach_not_applicable",
            ),
        ] {
            let reply = admin_switches(&mut app, r1, body).await;
            assert_refused_unchanged(
                &app,
                &reply,
                (StatusCode::UNPROCESSABLE_ENTITY, reason),
                r1,
                &before,
            )
            .await;
        }
    })
    .await;
}

/// KIND-32
#[tokio::test]
async fn reversing_a_coaching_relationship_needs_confirmation_then_swaps_the_seats() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let (coach, coachee) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );
        let before = relationship_row(app.db(), r1).await;
        let reversal = json!({ "kind": "coaching", "coach_id": coachee });

        let reply = admin_switches(&mut app, r1, reversal.clone()).await;

        assert_refused_unchanged(
            &app,
            &reply,
            (StatusCode::CONFLICT, "kind_switch_needs_confirmation"),
            r1,
            &before,
        )
        .await;
        let details = &reply.body["details"];
        assert_eq!(details["scheduling_gained"], json!([coachee]), "{details}");
        assert_eq!(details["scheduling_lost"], json!([coach]), "{details}");
        assert_eq!(
            details["actions_becoming_visible"],
            json!([{ "user_id": coachee, "by_assignee": [{ "assignee_id": coach, "count": 1 }] }]),
            "{details}"
        );

        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "coaching", "coach_id": coachee, "confirmed": true }),
        )
        .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(
            relationship_row(app.db(), r1).await.map(|row| (
                row.kind,
                row.coach_id,
                row.coachee_id
            )),
            Some((Kind::Coaching, coachee, coach))
        );
    })
    .await;
}

/// KIND-45
#[tokio::test]
async fn a_confirmed_reversal_returns_each_swapped_seats_email() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, coachee) = (app.fixture.r1, app.fixture.user(Caller::Coachee));

        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "coaching", "coach_id": coachee, "confirmed": true }),
        )
        .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        let data = &reply.body["data"];
        assert_eq!(
            data["participants"][0]["email"],
            Caller::Coachee.email(),
            "{data}"
        );
        assert_eq!(
            data["participants"][1]["email"],
            Caller::Coach.email(),
            "{data}"
        );
    })
    .await;
}

/// KIND-33
#[tokio::test]
async fn a_pair_coaching_each_other_cannot_switch_either_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let (coach, coachee) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );
        let reverse = admin_creates(
            &mut app,
            json!({ "coach_id": coachee, "coachee_id": coach }),
        )
        .await;
        let before = stored_between(&app, coach, coachee).await;

        for (relationship, body, other) in [
            (
                r1,
                json!({ "kind": "colleague", "confirmed": true }),
                reverse,
            ),
            (
                reverse,
                json!({ "kind": "colleague", "confirmed": true }),
                r1,
            ),
            (
                r1,
                json!({ "kind": "coaching", "coach_id": coachee, "confirmed": true }),
                reverse,
            ),
        ] {
            let reply = admin_switches(&mut app, relationship, body.clone()).await;

            assert_eq!(reply.status, StatusCode::CONFLICT, "{body}: {}", reply.body);
            assert_eq!(reply.body["details"]["reason"], "pair_already_related");
            assert_eq!(
                reply.body["details"]["relationship_id"],
                other.to_string(),
                "{body}"
            );
            assert_eq!(stored_between(&app, coach, coachee).await, before, "{body}");
        }
    })
    .await;
}

/// KIND-34
#[tokio::test]
async fn only_the_organizations_admins_switch_a_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, o2, r1) = (app.fixture.o1, app.fixture.o2, app.fixture.r1);
        let before = relationship_row(app.db(), r1).await;
        let body = json!({ "kind": "colleague", "confirmed": true });

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::OtherCoach,
            Caller::Member,
            Caller::Outsider,
            Caller::Removed,
        ] {
            let reply = switch_kind_in(&mut app, caller, o1, r1, body.clone()).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        let reply = switch_kind_in(&mut app, Caller::Super, o2, r1, body.clone()).await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "another org's relationship"
        );
        let reply = admin_switches(&mut app, Id::new_v4(), body).await;
        assert_eq!(
            reply.status,
            StatusCode::NOT_FOUND,
            "an unknown relationship"
        );
        assert_eq!(relationship_row(app.db(), r1).await, before);
    })
    .await;
}

/// KIND-35
#[tokio::test]
async fn switching_to_what_the_relationship_already_is_changes_nothing() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let coach = app.fixture.user(Caller::Coach);
        let before = relationship_row(app.db(), r1).await;

        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "coaching", "coach_id": coach }),
        )
        .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.ids(), ids([r1]));
        assert_eq!(relationship_row(app.db(), r1).await, before);
    })
    .await;
}

/// KIND-36
#[tokio::test]
async fn a_relationship_whose_participant_left_cannot_switch() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r3 = app.fixture.r3;
        let before = relationship_row(app.db(), r3).await;

        for body in [
            json!({ "kind": "colleague" }),
            json!({ "kind": "colleague", "confirmed": true }),
        ] {
            let reply = admin_switches(&mut app, r3, body.clone()).await;

            assert_refused_unchanged(
                &app,
                &reply,
                (StatusCode::CONFLICT, "participant_not_a_member"),
                r3,
                &before,
            )
            .await;
            let details = &reply.body["details"];
            assert_eq!(
                details["user_id"],
                app.fixture.user(Caller::Removed).to_string(),
                "{body}"
            );
            assert_eq!(
                details["organization_id"],
                app.fixture.o1.to_string(),
                "{body}"
            );
        }
    })
    .await;
}

/// KIND-37
#[tokio::test]
async fn a_switch_marks_the_newly_reminded_participants_imminent_reminders_sent() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, s1) = (app.fixture.r1, app.fixture.s1);
        let coach = app.fixture.user(Caller::Coach);
        let imminent = seed_session_starting_in(app.db(), r1, TimeDelta::hours(2)).await;
        let (start, _) = session_schedule(app.db(), imminent)
            .await
            .expect("the session reads");

        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "colleague", "confirmed": true }),
        )
        .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(
            reminders_sent(app.db(), imminent).await,
            BTreeMap::from([(coach, start)]),
            "the former coach is newly reminded; the coachee always was"
        );
        assert_eq!(
            reminders_sent(app.db(), s1).await,
            BTreeMap::new(),
            "S1 is days away, outside the window"
        );
    })
    .await;
}

/// The confirmed switch of R1 to colleague, answered 200.
async fn admin_switches_r1_to_colleague(app: &mut TestApp) {
    let r1 = app.fixture.r1;
    let reply = admin_switches(app, r1, json!({ "kind": "colleague", "confirmed": true })).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
}

/// Each of `targets` owed to both of R1's participants.
fn owed_to_both(app: &TestApp, targets: &[Id]) -> BTreeSet<(Id, Id)> {
    let participants = [
        app.fixture.user(Caller::Coach),
        app.fixture.user(Caller::Coachee),
    ];
    targets
        .iter()
        .flat_map(|&target| participants.map(|user| (target, user)))
        .collect()
}

/// `count` copies of each caller's address, sorted as `take_emailed` returns them.
fn mailed(callers: &[Caller], count: usize) -> Vec<String> {
    let mut mailed: Vec<String> = callers
        .iter()
        .flat_map(|caller| std::iter::repeat_n(caller.email(), count))
        .collect();
    mailed.sort();
    mailed
}

/// The data of each `coaching_relationship_kind_changed` event `listener` received.
async fn kind_changes(listener: &mut Listener) -> Vec<serde_json::Value> {
    listener
        .received()
        .await
        .into_iter()
        .filter(|(event, _)| event == "coaching_relationship_kind_changed")
        .map(|(_, data)| data["data"].clone())
        .collect()
}

/// KIND-38
#[tokio::test]
async fn a_confirmed_switch_resends_every_upcoming_invite_to_both_once() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, sr1) = (app.fixture.s1, app.fixture.sr1);
        let s4 = seed_content_free_session(app.db(), app.fixture.r1).await;

        admin_switches_r1_to_colleague(&mut app).await;

        assert_eq!(
            pending_invites(app.db()).await,
            owed_to_both(&app, &[s1, s4, sr1]),
            "S1 and S4 their own, SR1's sessions their series'"
        );
        app.take_emailed();
        app.run_pending_reschedule_sweep().await;
        assert_eq!(
            app.take_emailed(),
            mailed(&[Caller::Coach, Caller::Coachee], 3),
            "one copy of each invite to each participant"
        );
        assert_eq!(pending_invites(app.db()).await, BTreeSet::new());

        app.run_pending_reschedule_sweep().await;
        assert_eq!(app.take_emailed(), Vec::<String>::new(), "a second run");
    })
    .await;
}

/// KIND-39
#[tokio::test]
async fn switches_before_the_sweep_owe_each_invite_once() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, s1, sr1) = (app.fixture.r1, app.fixture.s1, app.fixture.sr1);
        let coach = app.fixture.user(Caller::Coach);

        admin_switches_r1_to_colleague(&mut app).await;
        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "coaching", "coach_id": coach, "confirmed": true }),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);

        assert_eq!(
            pending_invites(app.db()).await,
            owed_to_both(&app, &[s1, sr1])
        );
        app.take_emailed();
        app.run_pending_reschedule_sweep().await;
        assert_eq!(
            app.take_emailed(),
            mailed(&[Caller::Coach, Caller::Coachee], 2)
        );
    })
    .await;
}

/// KIND-40
#[tokio::test]
async fn a_participant_who_left_before_the_sweep_gets_no_invite() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (coachee, o1) = (app.fixture.user(Caller::Coachee), app.fixture.o1);

        admin_switches_r1_to_colleague(&mut app).await;
        remove_roles(app.db(), coachee, o1).await;
        app.take_emailed();
        app.run_pending_reschedule_sweep().await;

        assert_eq!(
            app.take_emailed(),
            mailed(&[Caller::Coach], 2),
            "S1 and SR1 to the coach only"
        );
        assert_eq!(
            pending_invites(app.db()).await,
            BTreeSet::new(),
            "nothing is owed to someone who left"
        );
    })
    .await;
}

/// KIND-41
#[tokio::test]
async fn a_past_session_is_owed_no_invite() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, sr1) = (app.fixture.s1, app.fixture.sr1);
        let past = seed_session_starting_in(app.db(), app.fixture.r1, TimeDelta::days(-2)).await;

        admin_switches_r1_to_colleague(&mut app).await;

        let owed = pending_invites(app.db()).await;
        assert_eq!(owed, owed_to_both(&app, &[s1, sr1]));
        assert!(owed.iter().all(|&(target, _)| target != past));
    })
    .await;
}

/// KIND-42
#[tokio::test]
async fn an_unconfirmed_or_refused_switch_owes_nothing_and_tells_no_one() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let member = app.fixture.user(Caller::Member);
        let mut listeners = [
            app.listen(Caller::Coach).await,
            app.listen(Caller::Coachee).await,
        ];

        for (body, status) in [
            (json!({ "kind": "colleague" }), StatusCode::CONFLICT),
            (
                json!({ "kind": "coaching", "coach_id": member, "confirmed": true }),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ] {
            let reply = admin_switches(&mut app, r1, body.clone()).await;
            assert_eq!(reply.status, status, "{body}: {}", reply.body);
        }

        assert_eq!(pending_invites(app.db()).await, BTreeSet::new());
        for listener in &mut listeners {
            assert_eq!(
                kind_changes(listener).await,
                Vec::<serde_json::Value>::new()
            );
        }
    })
    .await;
}

/// KIND-43
#[tokio::test]
async fn a_switch_tells_both_participants_once() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r1) = (app.fixture.o1, app.fixture.r1);
        let coachee = app.fixture.user(Caller::Coachee);
        let change = |from: &str, to: &str| {
            json!({
                "coaching_relationship_id": r1,
                "organization_id": o1,
                "from": from,
                "to": to,
            })
        };
        let mut listeners = [
            app.listen(Caller::Coach).await,
            app.listen(Caller::Coachee).await,
        ];
        let mut member = app.listen(Caller::Member).await;

        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "coaching", "coach_id": coachee, "confirmed": true }),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        for listener in &mut listeners {
            assert_eq!(
                kind_changes(listener).await,
                vec![change("coaching", "coaching")],
                "a reversal keeps the kind"
            );
        }

        admin_switches_r1_to_colleague(&mut app).await;
        for listener in &mut listeners {
            assert_eq!(
                kind_changes(listener).await,
                vec![change("coaching", "colleague")]
            );
        }
        assert_eq!(
            kind_changes(&mut member).await,
            Vec::<serde_json::Value>::new(),
            "not a participant"
        );
    })
    .await;
}

/// KIND-44
#[tokio::test]
async fn a_relationship_in_an_archived_organization_cannot_switch() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r1) = (app.fixture.o1, app.fixture.r1);
        let admin = app.fixture.user(Caller::Admin);
        archive_organization(app.db(), o1, admin).await;
        let before = relationship_row(app.db(), r1).await;

        for body in [
            json!({ "kind": "colleague" }),
            json!({ "kind": "colleague", "confirmed": true }),
        ] {
            let reply = admin_switches(&mut app, r1, body.clone()).await;

            assert_eq!(reply.status, StatusCode::CONFLICT, "{body}: {}", reply.body);
            assert_eq!(reply.body["error"], "organization_archived", "{body}");
            assert_eq!(relationship_row(app.db(), r1).await, before, "{body}");
            assert_eq!(pending_invites(app.db()).await, BTreeSet::new(), "{body}");
        }
    })
    .await;
}

/// The audit entry for ADMIN creating colleague relationship `id` with `coach_id` in the coach seat.
fn colleagues_created(app: &TestApp, id: Id, coach_id: Id, coachee_id: Id) -> AuditEntry {
    AuditEntry {
        actor_user_id: Some(app.fixture.user(Caller::Admin)),
        action: "relationship_created".into(),
        subject_type: "coaching_relationship".into(),
        subject_id: id,
        details: json!({ "kind": "colleague", "coach_id": coach_id, "coachee_id": coachee_id }),
    }
}

/// REL-23
#[tokio::test]
async fn a_created_relationship_is_recorded() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (a, b) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );

        let created = admin_creates(
            &mut app,
            json!({ "coach_id": a, "coachee_id": b, "kind": "colleague" }),
        )
        .await;

        assert_eq!(
            verified_audit_trail(app.db(), app.fixture.o1).await,
            vec![colleagues_created(&app, created, a, b)]
        );
    })
    .await;
}

/// REL-24
#[tokio::test]
async fn a_returned_relationship_is_not_recorded_again() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (a, b) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );
        let created = admin_creates(
            &mut app,
            json!({ "coach_id": a, "coachee_id": b, "kind": "colleague" }),
        )
        .await;

        let returned = admin_creates(
            &mut app,
            json!({ "coach_id": b, "coachee_id": a, "kind": "colleague" }),
        )
        .await;

        assert_eq!(returned, created);
        assert_eq!(
            verified_audit_trail(app.db(), app.fixture.o1).await,
            vec![colleagues_created(&app, created, a, b)]
        );
    })
    .await;
}

/// KIND-46
#[tokio::test]
async fn a_confirmed_switch_is_recorded_with_the_seats_after_it() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let (coach, coachee) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );
        make_colleagues(app.db(), r1).await;

        let reply = admin_switches(
            &mut app,
            r1,
            json!({ "kind": "coaching", "coach_id": coachee, "confirmed": true }),
        )
        .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(
            verified_audit_trail(app.db(), app.fixture.o1).await,
            vec![AuditEntry {
                actor_user_id: Some(app.fixture.user(Caller::Admin)),
                action: "relationship_kind_changed".into(),
                subject_type: "coaching_relationship".into(),
                subject_id: r1,
                details: json!({
                    "from": "colleague",
                    "to": "coaching",
                    "coach_id": coachee,
                    "coachee_id": coach,
                }),
            }]
        );
    })
    .await;
}

/// KIND-46
#[tokio::test]
async fn an_unconfirmed_refused_or_unchanged_switch_records_nothing() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, r1) = (app.fixture.o1, app.fixture.r1);
        let (coach, member) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Member),
        );

        for (caller, body, status) in [
            (
                Caller::Admin,
                json!({ "kind": "colleague" }),
                StatusCode::CONFLICT,
            ),
            (
                Caller::Admin,
                json!({ "kind": "coaching", "coach_id": member, "confirmed": true }),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Caller::Coach,
                json!({ "kind": "colleague", "confirmed": true }),
                StatusCode::FORBIDDEN,
            ),
            (
                Caller::Admin,
                json!({ "kind": "coaching", "coach_id": coach, "confirmed": true }),
                StatusCode::OK,
            ),
        ] {
            let reply = switch_kind_in(&mut app, caller, o1, r1, body.clone()).await;
            assert_eq!(reply.status, status, "{body}: {}", reply.body);
        }

        assert_eq!(verified_audit_trail(app.db(), o1).await, vec![]);
    })
    .await;
}
