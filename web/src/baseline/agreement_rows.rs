//! Agreements (`AGR`).

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{agreement_body, seed_agreement, session_content};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use super::harness::{ids, Baseline, NON_PARTICIPANTS};

/// An agreement body for `session`.
fn agreement(session: Id) -> Value {
    json!({ "coaching_session_id": session, "body": "Revised agreement kumquat" })
}

/// AGR-01
#[tokio::test]
async fn agr_01_participants_list_the_sessions_agreements() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/agreements?coaching_session_id={}", world.s1);
        let expected = ids([world.ag1]);

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

/// AGR-03
#[tokio::test]
async fn agr_03_a_removed_coachee_cannot_list_their_sessions_agreements() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/agreements?coaching_session_id={}", baseline.world.s3);

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// AGR-04
#[tokio::test]
async fn agr_04_participants_read_the_agreement() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let ag1 = baseline.world.ag1;

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                baseline
                    .expect(caller, Method::GET, &format!("/agreements/{ag1}"), None)
                    .await,
                (StatusCode::OK, ids([ag1])),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// AGR-05
#[tokio::test]
async fn agr_05_participants_create_an_agreement_in_the_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = session_content(baseline.db(), s1).await;
            let reply = baseline
                .send(caller, Method::POST, "/agreements", Some(agreement(s1)))
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created = reply.ids();
            assert_eq!(created.len(), 1, "{caller:?}: one new agreement");
            assert!(created.is_disjoint(&before), "{caller:?}");
            assert_eq!(
                session_content(baseline.db(), s1).await,
                before.union(&created).copied().collect(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// AGR-06
#[tokio::test]
async fn agr_06_participants_update_the_agreement() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, ag1) = (baseline.world.s1, baseline.world.ag1);

        for (caller, text) in [(Caller::Coach, "By coach"), (Caller::Coachee, "By coachee")] {
            let body = json!({ "coaching_session_id": s1, "body": text });
            assert_eq!(
                baseline
                    .expect(
                        caller,
                        Method::PUT,
                        &format!("/agreements/{ag1}"),
                        Some(body)
                    )
                    .await,
                (StatusCode::OK, ids([ag1])),
                "{caller:?}"
            );
            assert_eq!(
                agreement_body(baseline.db(), ag1).await,
                Some(Some(text.to_string())),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// AGR-07
#[tokio::test]
async fn agr_07_participants_delete_the_agreement() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut baseline = Baseline::new().await;
            let ag1 = baseline.world.ag1;

            let reply = baseline
                .send(caller, Method::DELETE, &format!("/agreements/{ag1}"), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["id"], json!(ag1), "{caller:?}");
            assert_eq!(agreement_body(baseline.db(), ag1).await, None, "{caller:?}");
        }
    })
    .await;
}

/// AGR-08
#[tokio::test]
async fn agr_08_non_participants_cannot_read_the_agreement() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let ag1 = baseline.world.ag1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(caller, Method::GET, &format!("/agreements/{ag1}"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        let (status, _) = baseline
            .expect(
                Caller::Coach,
                Method::GET,
                &format!("/agreements/{}", Id::new_v4()),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "an unknown agreement");
    })
    .await;
}

/// AGR-09
#[tokio::test]
async fn agr_09_non_participants_cannot_create_an_agreement_in_the_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let before = session_content(baseline.db(), s1).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(caller, Method::POST, "/agreements", Some(agreement(s1)))
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                session_content(baseline.db(), s1).await,
                before,
                "{caller:?}"
            );
        }
        let (status, _) = baseline
            .expect(
                Caller::Coach,
                Method::POST,
                "/agreements",
                Some(agreement(Id::new_v4())),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "an unknown session");
    })
    .await;
}

/// AGR-10
#[tokio::test]
async fn agr_10_non_participants_cannot_change_the_agreement() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, ag1) = (baseline.world.s1, baseline.world.ag1);
        let path = format!("/agreements/{ag1}");
        let before = agreement_body(baseline.db(), ag1).await;

        for caller in NON_PARTICIPANTS {
            for (method, body) in [(Method::PUT, Some(agreement(s1))), (Method::DELETE, None)] {
                let (status, _) = baseline.expect(caller, method.clone(), &path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
                assert_eq!(
                    agreement_body(baseline.db(), ag1).await,
                    before,
                    "{caller:?} {method}"
                );
            }
        }
    })
    .await;
}

/// AGR-11
#[tokio::test]
async fn agr_11_a_removed_coachee_cannot_reach_their_sessions_agreements() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s3 = baseline.world.s3;
        let ag3 = seed_agreement(baseline.db(), s3, baseline.world.user(Caller::Coach)).await;
        let body = agreement_body(baseline.db(), ag3).await;
        let content = session_content(baseline.db(), s3).await;
        let path = format!("/agreements/{ag3}");

        for (method, path, request) in [
            (Method::GET, path.as_str(), None),
            (Method::PUT, path.as_str(), Some(agreement(s3))),
            (Method::DELETE, path.as_str(), None),
            (Method::POST, "/agreements", Some(agreement(s3))),
        ] {
            let (status, _) = baseline
                .expect(Caller::Removed, method.clone(), path, request)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
        assert_eq!(agreement_body(baseline.db(), ag3).await, body);
        assert_eq!(session_content(baseline.db(), s3).await, content);
    })
    .await;
}
