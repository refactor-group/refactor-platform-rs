//! Actions (`ACT`), notes (`NOTE`) and agreements (`AGR`).

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use super::harness::{ids, ids_in, Baseline, NON_PARTICIPANTS};

/// ACT-02
#[tokio::test]
async fn act_02_a_coachee_lists_only_own_and_unassigned_actions_per_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/organizations/{}/coaching_relationships/actions", world.o1);
        let expected = BTreeMap::from([
            (world.r1, ids([world.a2, world.a3])),
            (world.r2, ids([world.a5])),
        ]);

        let reply = baseline
            .send(Caller::Coachee, Method::GET, &path, None)
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        let by_relationship: BTreeMap<Id, BTreeSet<Id>> = reply.body["data"]["coachee_actions"]
            .as_object()
            .expect("actions keyed by relationship")
            .iter()
            .map(|(id, actions)| (id.parse().expect("a relationship id key"), ids_in(actions)))
            .collect();
        assert_eq!(by_relationship, expected);
    })
    .await;
}

/// ACT-14
#[tokio::test]
async fn act_14_a_coach_lists_the_sessions_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/actions?coaching_session_id={}", world.s1);
        let expected = ids([world.a1, world.a2, world.a3]);

        assert_eq!(
            baseline
                .expect(Caller::Coach, Method::GET, &path, None)
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// NOTE-01
#[tokio::test]
async fn note_01_participants_list_the_sessions_notes() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/notes?coaching_session_id={}", world.s1);
        let expected = ids([world.n1]);

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                baseline.expect(caller, Method::GET, &path, None).await,
                (StatusCode::OK, expected.clone()),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// AGR-02
#[tokio::test]
async fn agr_02_non_participants_cannot_list_the_sessions_agreements() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/agreements?coaching_session_id={}", baseline.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}
