//! Who can read, create, change and delete agreements.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::read_back::{agreement_body, session_content};
use domain::test_utils::sqlite::{seed_agreement, within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{assert_signed_out_refused, ids, TestApp, NON_PARTICIPANTS};

/// An agreement body for `session`.
fn agreement(session: Id) -> Value {
    json!({ "coaching_session_id": session, "body": "Revised agreement kumquat" })
}

/// AGR-01
#[tokio::test]
async fn participants_list_the_sessions_agreements() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let path = format!("/agreements?coaching_session_id={}", fixture.s1);
        let expected = ids([fixture.ag1]);
        // R2's session holds one too, which S1's list must leave out.
        let other_coach = fixture.user(Caller::OtherCoach);
        seed_agreement(app.db(), fixture.s2, other_coach, "Other kumquat").await;

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                app.expect(caller, Method::GET, &path, None).await,
                (StatusCode::OK, expected.clone()),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// AGR-02
#[tokio::test]
async fn non_participants_cannot_list_the_sessions_agreements() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/agreements?coaching_session_id={}", app.fixture.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// AGR-03
#[tokio::test]
async fn a_removed_coachee_cannot_list_their_sessions_agreements() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/agreements?coaching_session_id={}", app.fixture.s3);

        let (status, _) = app.expect(Caller::Removed, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// AGR-04
#[tokio::test]
async fn participants_read_the_agreement() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let ag1 = app.fixture.ag1;

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                app.expect(caller, Method::GET, &format!("/agreements/{ag1}"), None)
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
async fn participants_create_an_agreement_in_the_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = session_content(app.db(), s1).await;
            let reply = app
                .send(caller, Method::POST, "/agreements", Some(agreement(s1)))
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created = reply.ids();
            assert_eq!(created.len(), 1, "{caller:?}: one new agreement");
            assert!(created.is_disjoint(&before), "{caller:?}");
            assert_eq!(
                session_content(app.db(), s1).await,
                before.union(&created).copied().collect(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// AGR-06
#[tokio::test]
async fn participants_update_the_agreement() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, ag1) = (app.fixture.s1, app.fixture.ag1);

        for (caller, text) in [(Caller::Coach, "By coach"), (Caller::Coachee, "By coachee")] {
            let body = json!({ "coaching_session_id": s1, "body": text });
            assert_eq!(
                app.expect(
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
                agreement_body(app.db(), ag1).await,
                Some(Some(text.to_string())),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// AGR-07
#[tokio::test]
async fn participants_delete_the_agreement() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let ag1 = app.fixture.ag1;

            let reply = app
                .send(caller, Method::DELETE, &format!("/agreements/{ag1}"), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["id"], json!(ag1), "{caller:?}");
            assert_eq!(agreement_body(app.db(), ag1).await, None, "{caller:?}");
        }
    })
    .await;
}

/// AGR-08
#[tokio::test]
async fn non_participants_cannot_read_the_agreement() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let ag1 = app.fixture.ag1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = app
                .expect(caller, Method::GET, &format!("/agreements/{ag1}"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        let (status, _) = app
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
async fn non_participants_cannot_create_an_agreement_in_the_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let before = session_content(app.db(), s1).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = app
                .expect(caller, Method::POST, "/agreements", Some(agreement(s1)))
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(session_content(app.db(), s1).await, before, "{caller:?}");
        }
        let (status, _) = app
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
async fn non_participants_cannot_change_the_agreement() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, ag1) = (app.fixture.s1, app.fixture.ag1);
        let path = format!("/agreements/{ag1}");
        let before = agreement_body(app.db(), ag1).await;

        for caller in NON_PARTICIPANTS {
            for (method, body) in [(Method::PUT, Some(agreement(s1))), (Method::DELETE, None)] {
                let (status, _) = app.expect(caller, method.clone(), &path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
                assert_eq!(
                    agreement_body(app.db(), ag1).await,
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
async fn a_removed_coachee_cannot_reach_their_sessions_agreements() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s3 = app.fixture.s3;
        let ag3 = seed_agreement(
            app.db(),
            s3,
            app.fixture.user(Caller::Coach),
            "Agreement kumquat",
        )
        .await;
        let body = agreement_body(app.db(), ag3).await;
        let content = session_content(app.db(), s3).await;
        let path = format!("/agreements/{ag3}");

        for (method, path, request) in [
            (Method::GET, path.as_str(), None),
            (Method::PUT, path.as_str(), Some(agreement(s3))),
            (Method::DELETE, path.as_str(), None),
            (Method::POST, "/agreements", Some(agreement(s3))),
        ] {
            let (status, _) = app
                .expect(Caller::Removed, method.clone(), path, request)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
        assert_eq!(agreement_body(app.db(), ag3).await, body);
        assert_eq!(session_content(app.db(), s3).await, content);
    })
    .await;
}

/// AGR-12
#[tokio::test]
async fn signed_out_callers_cannot_reach_agreements() {
    within_time_limit(assert_signed_out_refused(|app| {
        let (s1, ag1) = (app.fixture.s1, app.fixture.ag1);
        vec![
            (Method::GET, format!("/agreements?coaching_session_id={s1}")),
            (Method::POST, "/agreements".to_string()),
            (Method::GET, format!("/agreements/{ag1}")),
            (Method::PUT, format!("/agreements/{ag1}")),
            (Method::DELETE, format!("/agreements/{ag1}")),
        ]
    }))
    .await;
}
