//! Who can list, link and unlink goals on a coaching session.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in `goal_controller_tests.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::session_goals;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{ids, ids_by_key, TestApp, NON_PARTICIPANTS};

/// The batch session-goals read's ids per session.
async fn batch_goals(
    app: &mut TestApp,
    caller: Caller,
    query: &str,
) -> (StatusCode, BTreeMap<Id, BTreeSet<Id>>) {
    let reply = app
        .send(
            caller,
            Method::GET,
            &format!("/coaching_sessions/goals?{query}"),
            None,
        )
        .await;
    let by_session = match reply.status {
        StatusCode::OK => ids_by_key(&reply.body["data"]["session_goals"]),
        _ => BTreeMap::new(),
    };
    (reply.status, by_session)
}

/// GOAL-11
#[tokio::test]
async fn participants_list_the_sessions_goals() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_sessions/{}/goals", app.world.s1);
        let expected = ids([app.world.g1]);

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

/// GOAL-12
#[tokio::test]
async fn non_participants_cannot_list_the_sessions_goals() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_sessions/{}/goals", app.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-13
#[tokio::test]
async fn participants_link_a_goal_to_another_session_of_the_relationship() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let (session, g1) = (app.world.sr1_sessions[0], app.world.g1);
            assert_eq!(session_goals(app.db(), session).await, BTreeSet::new());

            let reply = app
                .send(
                    caller,
                    Method::POST,
                    &format!("/coaching_sessions/{session}/goals"),
                    Some(json!({ "goal_id": g1 })),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            assert_eq!(
                session_goals(app.db(), session).await,
                ids([g1]),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-14
#[tokio::test]
async fn participants_unlink_a_goal_from_the_session() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let (s1, g1) = (app.world.s1, app.world.g1);

            let reply = app
                .send(
                    caller,
                    Method::DELETE,
                    &format!("/coaching_sessions/{s1}/goals/{g1}"),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["goal_id"], json!(g1), "{caller:?}");
            assert_eq!(
                session_goals(app.db(), s1).await,
                BTreeSet::new(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-15
#[tokio::test]
async fn participants_read_goals_per_session_of_the_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let world = &app.world;
        let query = format!("coaching_relationship_id={}", world.r1);
        let expected = BTreeMap::from([
            (world.s1, ids([world.g1])),
            (world.sr1_sessions[0], BTreeSet::new()),
            (world.sr1_sessions[1], BTreeSet::new()),
        ]);

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                batch_goals(&mut app, caller, &query).await,
                (StatusCode::OK, expected.clone()),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-16
#[tokio::test]
async fn non_participants_cannot_read_goals_per_session_of_the_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let query = format!("coaching_relationship_id={}", app.world.r1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = batch_goals(&mut app, caller, &query).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-17
#[tokio::test]
async fn every_batch_session_must_be_the_callers() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, s2, g1) = (app.world.s1, app.world.s2, app.world.g1);

        for query in [
            format!("coaching_session_ids={s1},{s2}"),
            format!("coaching_session_ids={s2},{s1}"),
        ] {
            let (status, _) = batch_goals(&mut app, Caller::Coach, &query).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{query}");
        }
        assert_eq!(
            batch_goals(
                &mut app,
                Caller::Coach,
                &format!("coaching_session_ids={s1}")
            )
            .await,
            (StatusCode::OK, BTreeMap::from([(s1, ids([g1]))])),
            "S1 alone"
        );
    })
    .await;
}

/// GOAL-18
#[tokio::test]
async fn non_participants_cannot_batch_read_the_sessions_goals() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let query = format!("coaching_session_ids={}", app.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = batch_goals(&mut app, caller, &query).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-26
#[tokio::test]
async fn non_participants_cannot_change_the_sessions_goal_links() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, s3, g1) = (app.world.s1, app.world.s3, app.world.g1);
        let s1_links = session_goals(app.db(), s1).await;
        let s3_links = session_goals(app.db(), s3).await;

        let refusals = NON_PARTICIPANTS
            .into_iter()
            .map(|caller| (caller, s1))
            .chain([(Caller::Removed, s3)]);
        for (caller, session) in refusals {
            for (method, path, body) in [
                (
                    Method::POST,
                    format!("/coaching_sessions/{session}/goals"),
                    Some(json!({ "goal_id": g1 })),
                ),
                (
                    Method::DELETE,
                    format!("/coaching_sessions/{session}/goals/{g1}"),
                    None,
                ),
            ] {
                let (status, _) = app.expect(caller, method.clone(), &path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
            }
        }
        assert_eq!(session_goals(app.db(), s1).await, s1_links);
        assert_eq!(session_goals(app.db(), s3).await, s3_links);
    })
    .await;
}

/// GOAL-27
#[tokio::test]
async fn linking_a_goal_from_another_relationship_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, g2) = (app.world.s1, app.world.g2);
        let links = session_goals(app.db(), s1).await;

        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_sessions/{s1}/goals"),
                Some(json!({ "goal_id": g2 })),
            )
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(session_goals(app.db(), s1).await, links);
    })
    .await;
}
