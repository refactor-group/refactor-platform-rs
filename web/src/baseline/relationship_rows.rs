//! Relationships (`REL`).

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::relationships_between;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::json;

use super::harness::{ids, Baseline, NON_PARTICIPANTS};

/// Asserts one caller's list of O1's relationships.
async fn assert_org_list(caller: Caller, expected: impl Fn(&Baseline) -> BTreeSet<Id>) {
    let mut baseline = Baseline::new().await;
    let path = format!(
        "/organizations/{}/coaching_relationships",
        baseline.world.o1
    );
    let expected = expected(&baseline);

    assert_eq!(
        baseline.expect(caller, Method::GET, &path, None).await,
        (StatusCode::OK, expected),
        "{caller:?}"
    );
}

/// REL-01
#[tokio::test]
async fn rel_01_a_coach_lists_only_their_own_relationships() {
    within_time_limit(assert_org_list(Caller::Coach, |b| {
        ids([b.world.r1, b.world.r3])
    }))
    .await;
}

/// REL-02
#[tokio::test]
async fn rel_02_a_coachee_lists_only_their_own_relationships() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/organizations/{}/coaching_relationships", world.o1);
        let expected = ids([world.r1, world.r2]);

        assert_eq!(
            baseline
                .expect(Caller::Coachee, Method::GET, &path, None)
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// REL-03
#[tokio::test]
async fn rel_03_another_coach_lists_only_their_relationship() {
    within_time_limit(assert_org_list(Caller::OtherCoach, |b| ids([b.world.r2]))).await;
}

/// REL-04
#[tokio::test]
async fn rel_04_a_member_in_no_relationship_lists_none() {
    within_time_limit(assert_org_list(Caller::Member, |_| BTreeSet::new())).await;
}

/// REL-05
#[tokio::test]
async fn rel_05_an_admin_lists_every_relationship_in_the_org() {
    within_time_limit(assert_org_list(Caller::Admin, |b| {
        ids([b.world.r1, b.world.r2, b.world.r3])
    }))
    .await;
}

/// REL-06
#[tokio::test]
async fn rel_06_a_super_admin_lists_every_relationship_in_the_org() {
    within_time_limit(assert_org_list(Caller::Super, |b| {
        ids([b.world.r1, b.world.r2, b.world.r3])
    }))
    .await;
}

/// REL-07
#[tokio::test]
async fn rel_07_non_members_cannot_list_the_orgs_relationships() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!(
            "/organizations/{}/coaching_relationships",
            baseline.world.o1
        );

        for caller in [Caller::Outsider, Caller::Removed] {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// REL-08
#[tokio::test]
async fn rel_08_participants_read_their_relationship_with_names() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, r1) = (baseline.world.o1, baseline.world.r1);
        let path = format!("/organizations/{o1}/coaching_relationships/{r1}");

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline.send(caller, Method::GET, &path, None).await;

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
async fn rel_09_non_participants_cannot_read_a_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, r1) = (baseline.world.o1, baseline.world.r1);
        let path = format!("/organizations/{o1}/coaching_relationships/{r1}");

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// REL-10
#[tokio::test]
async fn rel_10_a_relationship_under_another_org_is_not_found() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o2, r1) = (baseline.world.o2, baseline.world.r1);
        let path = format!("/organizations/{o2}/coaching_relationships/{r1}");

        let (status, _) = baseline
            .expect(Caller::Coach, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}

/// REL-11
#[tokio::test]
async fn rel_11_a_removed_coachee_cannot_read_their_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, r3) = (baseline.world.o1, baseline.world.r3);
        let path = format!("/organizations/{o1}/coaching_relationships/{r3}");

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// REL-12
#[tokio::test]
async fn rel_12_a_coach_keeps_a_relationship_whose_coachee_left() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, r3) = (baseline.world.o1, baseline.world.r3);
        let path = format!("/organizations/{o1}/coaching_relationships/{r3}");

        let reply = baseline.send(Caller::Coach, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.ids(), ids([r3]));
        assert_eq!(reply.body["data"]["coach_is_member"], true);
        assert_eq!(reply.body["data"]["coachee_is_member"], false);
    })
    .await;
}

/// REL-13
#[tokio::test]
async fn rel_13_a_coachee_lists_their_own_relationships_by_role() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let coachee = world.user(Caller::Coachee);
        let both = ids([world.r1, world.r2]);
        let path = format!("/users/{coachee}/coaching-relationships");

        for (query, expected) in [
            ("", both.clone()),
            ("?role=coach", BTreeSet::new()),
            ("?role=coachee", both),
        ] {
            assert_eq!(
                baseline
                    .expect(
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
async fn rel_14_only_the_user_lists_their_own_relationships() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let coachee = baseline.world.user(Caller::Coachee);
        let path = format!("/users/{coachee}/coaching-relationships");

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// REL-15
#[tokio::test]
async fn rel_15_admins_create_a_relationship_once_per_pair() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut baseline = Baseline::new().await;
            let (coach, member) = (
                baseline.world.user(Caller::Coach),
                baseline.world.user(Caller::Member),
            );
            let path = format!(
                "/organizations/{}/coaching_relationships",
                baseline.world.o1
            );
            let body = json!({ "coach_id": coach, "coachee_id": member });

            let created = baseline
                .send(caller, Method::POST, &path, Some(body.clone()))
                .await;
            assert_eq!(created.status, StatusCode::OK, "{caller:?}");
            assert_eq!(created.body["status_code"], 201, "{caller:?}");
            let stored = relationships_between(baseline.db(), coach, member).await;
            assert_eq!(stored.len(), 1, "{caller:?}");
            assert_eq!(created.ids(), stored, "{caller:?}");

            let again = baseline.send(caller, Method::POST, &path, Some(body)).await;
            assert_eq!(again.status, StatusCode::OK, "{caller:?} again");
            assert_eq!(again.ids(), stored, "{caller:?} again");
            assert_eq!(
                relationships_between(baseline.db(), coach, member).await,
                stored,
                "{caller:?} again"
            );
        }
    })
    .await;
}

/// REL-16
#[tokio::test]
async fn rel_16_non_admins_cannot_create_a_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (coach, member) = (
            baseline.world.user(Caller::Coach),
            baseline.world.user(Caller::Member),
        );
        let path = format!(
            "/organizations/{}/coaching_relationships",
            baseline.world.o1
        );
        let body = json!({ "coach_id": coach, "coachee_id": member });

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = baseline
                .expect(caller, Method::POST, &path, Some(body.clone()))
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                relationships_between(baseline.db(), coach, member).await,
                BTreeSet::new(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// REL-17
#[tokio::test]
async fn rel_17_a_removed_coachee_lists_none_of_their_own_relationships() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let removed = baseline.world.user(Caller::Removed);

        assert_eq!(
            baseline
                .expect(
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

/// REL-18
#[tokio::test]
async fn rel_18_an_admin_sees_who_is_still_a_member() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/organizations/{}/coaching_relationships", world.o1);
        let expected = BTreeMap::from([
            (world.r1, (true, true)),
            (world.r2, (true, true)),
            (world.r3, (true, false)),
        ]);

        let reply = baseline.send(Caller::Admin, Method::GET, &path, None).await;

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
