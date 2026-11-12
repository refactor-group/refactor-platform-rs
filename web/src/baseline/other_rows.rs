//! Search (`SRCH`), the collaboration token (`JWT`) and users (`USR`).

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use super::harness::{ids_in, Baseline};

/// SRCH-04
#[tokio::test]
async fn srch_04_members_outside_every_relationship_find_nothing() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = "/search?q=kumquat&types=actions";

        for caller in [Caller::Member, Caller::Admin, Caller::Outsider] {
            let reply = baseline.send(caller, Method::GET, path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(
                ids_in(&reply.body["data"]["hits"]),
                BTreeSet::new(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// JWT-01
#[tokio::test]
async fn jwt_01_participants_get_a_token_for_the_sessions_document() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let path = format!("/jwt/generate_collab_token?coaching_session_id={s1}");

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline.send(caller, Method::GET, &path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            let document = reply.body["data"]["sub"].as_str().unwrap_or_default();
            assert!(document.contains(&s1.to_string()), "{caller:?}: {document}");
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
