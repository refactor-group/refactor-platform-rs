//! Users (`USR`).

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use super::harness::{ids, Baseline};

/// USR-01
#[tokio::test]
async fn usr_01_a_coachees_sessions_include_their_relationships() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let coachee = world.user(Caller::Coachee);
        let expected = ids([
            world.s1,
            world.s2,
            world.sr1_sessions[0],
            world.sr1_sessions[1],
        ]);
        let path = format!("/users/{coachee}/coaching_sessions?include=relationship");

        let reply = baseline
            .send(Caller::Coachee, Method::GET, &path, None)
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.ids(), expected);
        for session in reply.body["data"].as_array().expect("a list of sessions") {
            for field in ["relationship", "coach", "coachee"] {
                assert!(session[field].is_object(), "{field} in {session}");
            }
            assert_eq!(session["coachee"]["id"], coachee.to_string(), "{session}");
        }
    })
    .await;
}

/// USR-02
#[tokio::test]
async fn usr_02_a_coachee_coaches_no_one() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let coachee = baseline.world.user(Caller::Coachee);
        let path = format!("/users/{coachee}/coaching-relationships?role=coach");

        assert_eq!(
            baseline
                .expect(Caller::Coachee, Method::GET, &path, None)
                .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// USR-03
#[tokio::test]
async fn usr_03_a_coach_lists_the_relationships_they_coach() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let coach = world.user(Caller::Coach);
        let expected = ids([world.r1, world.r3]);
        let path = format!("/users/{coach}/coaching-relationships?role=coach");

        assert_eq!(
            baseline
                .expect(Caller::Coach, Method::GET, &path, None)
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}
