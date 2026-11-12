//! Relationships (`REL`) and organizations (`ORG`).

use std::collections::BTreeMap;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::holds_role;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use super::harness::{ids, Baseline};

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

/// ORG-05
#[tokio::test]
async fn org_05_non_admins_cannot_remove_a_members_role() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, member) = (baseline.world.o1, baseline.world.user(Caller::Member));
        let path = format!("/organizations/{o1}/users/{member}/role");

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = baseline.expect(caller, Method::DELETE, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert!(holds_role(baseline.db(), member, o1).await, "{caller:?}");
        }
    })
    .await;
}
