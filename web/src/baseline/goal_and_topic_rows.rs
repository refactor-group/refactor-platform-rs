//! Goals (`GOAL`) and topics (`TOP`).

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use super::harness::{ids, ids_in, Baseline};

/// GOAL-15
#[tokio::test]
async fn goal_15_participants_read_goals_per_session_of_the_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!(
            "/coaching_sessions/goals?coaching_relationship_id={}",
            world.r1
        );
        let expected = BTreeMap::from([
            (world.s1, ids([world.g1])),
            (world.sr1_sessions[0], BTreeSet::new()),
            (world.sr1_sessions[1], BTreeSet::new()),
        ]);

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline.send(caller, Method::GET, &path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            let by_session: BTreeMap<Id, BTreeSet<Id>> = reply.body["data"]["session_goals"]
                .as_object()
                .expect("goals keyed by session")
                .iter()
                .map(|(id, goals)| (id.parse().expect("a session id key"), ids_in(goals)))
                .collect();
            assert_eq!(by_session, expected, "{caller:?}");
        }
    })
    .await;
}

/// TOP-01
#[tokio::test]
async fn top_01_participants_list_the_sessions_topics() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/coaching_sessions/{}/topics", world.s1);
        let expected = ids([world.t1, world.t2]);

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
