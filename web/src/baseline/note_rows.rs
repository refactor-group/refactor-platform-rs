//! Notes (`NOTE`).

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{note_body, seed_note, session_content};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use super::harness::{ids, Baseline, NON_PARTICIPANTS};

/// A note body for `session`.
fn note(session: Id) -> Value {
    json!({ "coaching_session_id": session, "body": "Revised note kumquat" })
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

/// NOTE-02
#[tokio::test]
async fn note_02_non_participants_cannot_list_the_sessions_notes() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/notes?coaching_session_id={}", baseline.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// NOTE-03
#[tokio::test]
async fn note_03_a_removed_coachee_cannot_list_their_sessions_notes() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/notes?coaching_session_id={}", baseline.world.s3);

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// NOTE-04
#[tokio::test]
async fn note_04_participants_read_the_note() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let n1 = baseline.world.n1;

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                baseline
                    .expect(caller, Method::GET, &format!("/notes/{n1}"), None)
                    .await,
                (StatusCode::OK, ids([n1])),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// NOTE-05
#[tokio::test]
async fn note_05_participants_create_a_note_in_the_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = session_content(baseline.db(), s1).await;
            let reply = baseline
                .send(caller, Method::POST, "/notes", Some(note(s1)))
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created = reply.ids();
            assert_eq!(created.len(), 1, "{caller:?}: one new note");
            assert!(created.is_disjoint(&before), "{caller:?}");
            let after = session_content(baseline.db(), s1).await;
            assert_eq!(
                after,
                before.union(&created).copied().collect(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// NOTE-06
#[tokio::test]
async fn note_06_participants_update_the_note() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, n1) = (baseline.world.s1, baseline.world.n1);

        for (caller, text) in [(Caller::Coach, "By coach"), (Caller::Coachee, "By coachee")] {
            let body = json!({ "coaching_session_id": s1, "body": text });
            assert_eq!(
                baseline
                    .expect(caller, Method::PUT, &format!("/notes/{n1}"), Some(body))
                    .await,
                (StatusCode::OK, ids([n1])),
                "{caller:?}"
            );
            assert_eq!(
                note_body(baseline.db(), n1).await,
                Some(Some(text.to_string())),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// NOTE-07
#[tokio::test]
async fn note_07_non_participants_cannot_read_the_note() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let n1 = baseline.world.n1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(caller, Method::GET, &format!("/notes/{n1}"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        let (status, _) = baseline
            .expect(
                Caller::Coach,
                Method::GET,
                &format!("/notes/{}", Id::new_v4()),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "an unknown note");
    })
    .await;
}

/// NOTE-08
#[tokio::test]
async fn note_08_non_participants_cannot_create_a_note_in_the_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let before = session_content(baseline.db(), s1).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(caller, Method::POST, "/notes", Some(note(s1)))
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
                "/notes",
                Some(note(Id::new_v4())),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "an unknown session");
    })
    .await;
}

/// NOTE-09
#[tokio::test]
async fn note_09_non_participants_cannot_update_the_note() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, n1) = (baseline.world.s1, baseline.world.n1);
        let before = note_body(baseline.db(), n1).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(caller, Method::PUT, &format!("/notes/{n1}"), Some(note(s1)))
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(note_body(baseline.db(), n1).await, before, "{caller:?}");
        }
    })
    .await;
}

/// NOTE-10
#[tokio::test]
async fn note_10_a_removed_coachee_cannot_reach_their_sessions_notes() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s3 = baseline.world.s3;
        let n3 = seed_note(baseline.db(), s3, baseline.world.user(Caller::Coach)).await;
        let body = note_body(baseline.db(), n3).await;
        let content = session_content(baseline.db(), s3).await;
        let path = format!("/notes/{n3}");

        for (method, path, request) in [
            (Method::GET, path.as_str(), None),
            (Method::PUT, path.as_str(), Some(note(s3))),
            (Method::POST, "/notes", Some(note(s3))),
        ] {
            let (status, _) = baseline
                .expect(Caller::Removed, method.clone(), path, request)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
        assert_eq!(note_body(baseline.db(), n3).await, body);
        assert_eq!(session_content(baseline.db(), s3).await, content);
    })
    .await;
}
