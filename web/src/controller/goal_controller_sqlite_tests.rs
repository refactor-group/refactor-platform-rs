//! Who can read, create, change and delete goals.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::status::Status;
use domain::test_utils::sqlite::read_back::{goal_state, relationship_goals, session_goals};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{
    assert_signed_out_refused, field_ids, ids, TestApp, NON_PARTICIPANTS,
};

/// A goal body in `relationship`, titled `title`.
fn goal(relationship: Id, title: &str) -> Value {
    json!({
        "coaching_relationship_id": relationship,
        "title": title,
        "body": "Goal body kumquat",
        "status": "InProgress",
    })
}

/// GOAL-01
#[tokio::test]
async fn participants_list_the_relationships_goals() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/goals?coaching_relationship_id={}", app.fixture.r1);
        let expected = ids([app.fixture.g1]);

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

/// GOAL-02
#[tokio::test]
async fn non_participants_cannot_list_the_relationships_goals() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/goals?coaching_relationship_id={}", app.fixture.r1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-03
#[tokio::test]
async fn a_removed_coachee_cannot_list_their_relationships_goals() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/goals?coaching_relationship_id={}", app.fixture.r3);

        let (status, _) = app.expect(Caller::Removed, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// GOAL-04
#[tokio::test]
async fn participants_read_the_goal() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let g1 = app.fixture.g1;

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                app.expect(caller, Method::GET, &format!("/goals/{g1}"), None)
                    .await,
                (StatusCode::OK, ids([g1])),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-05
#[tokio::test]
async fn non_participants_cannot_read_the_goal() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let g1 = app.fixture.g1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = app
                .expect(caller, Method::GET, &format!("/goals/{g1}"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::GET,
                &format!("/goals/{}", Id::new_v4()),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "an unknown goal");
    })
    .await;
}

/// GOAL-06
#[tokio::test]
async fn participants_update_the_goal() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, g1) = (app.fixture.r1, app.fixture.g1);

        for (caller, title) in [(Caller::Coach, "By coach"), (Caller::Coachee, "By coachee")] {
            assert_eq!(
                app.expect(
                    caller,
                    Method::PUT,
                    &format!("/goals/{g1}"),
                    Some(goal(r1, title))
                )
                .await,
                (StatusCode::OK, ids([g1])),
                "{caller:?}"
            );
            let stored = goal_state(app.db(), g1).await.and_then(|(t, _, _)| t);
            assert_eq!(stored.as_deref(), Some(title), "{caller:?}");
        }
    })
    .await;
}

/// GOAL-07
#[tokio::test]
async fn participants_delete_the_goal() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let g1 = app.fixture.g1;

            let reply = app
                .send(caller, Method::DELETE, &format!("/goals/{g1}"), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["id"], json!(g1), "{caller:?}");
            assert_eq!(goal_state(app.db(), g1).await, None, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-08
#[tokio::test]
async fn non_participants_cannot_change_the_goal() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, g1) = (app.fixture.r1, app.fixture.g1);
        let path = format!("/goals/{g1}");
        let before = goal_state(app.db(), g1).await;

        for caller in NON_PARTICIPANTS {
            for (method, body) in [
                (Method::PUT, Some(goal(r1, "Refused"))),
                (Method::DELETE, None),
            ] {
                let (status, _) = app.expect(caller, method.clone(), &path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
                assert_eq!(
                    goal_state(app.db(), g1).await,
                    before,
                    "{caller:?} {method}"
                );
            }
        }
    })
    .await;
}

/// GOAL-09
#[tokio::test]
async fn participants_list_the_goals_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let g1 = app.fixture.g1;
        let expected = ids([app.fixture.s1]);

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app
                .send(caller, Method::GET, &format!("/goals/{g1}/sessions"), None)
                .await;
            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(
                field_ids(&reply.body["data"], "coaching_session_id"),
                expected,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-10
#[tokio::test]
async fn participants_create_a_goal_in_the_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = relationship_goals(app.db(), r1).await;
            let reply = app
                .send(caller, Method::POST, "/goals", Some(goal(r1, "New goal")))
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created = reply.ids();
            assert_eq!(created.len(), 1, "{caller:?}: one new goal");
            assert!(created.is_disjoint(&before), "{caller:?}");
            assert_eq!(
                relationship_goals(app.db(), r1).await,
                before.union(&created).copied().collect(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-25
#[tokio::test]
async fn non_participants_cannot_create_a_goal_in_the_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, r3) = (app.fixture.r1, app.fixture.r3);
        let r1_goals = relationship_goals(app.db(), r1).await;
        let r3_goals = relationship_goals(app.db(), r3).await;

        let refusals = NON_PARTICIPANTS
            .into_iter()
            .map(|caller| (caller, r1))
            .chain([(Caller::Removed, r3)]);
        for (caller, relationship) in refusals {
            let (status, _) = app
                .expect(
                    caller,
                    Method::POST,
                    "/goals",
                    Some(goal(relationship, "Refused")),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        assert_eq!(relationship_goals(app.db(), r1).await, r1_goals);
        assert_eq!(relationship_goals(app.db(), r3).await, r3_goals);

        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::POST,
                "/goals",
                Some(goal(Id::new_v4(), "Unknown")),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "an unknown relationship");
    })
    .await;
}

/// GOAL-28
#[tokio::test]
async fn a_created_in_session_from_another_relationship_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, s2) = (app.fixture.r1, app.fixture.s2);
        let before = relationship_goals(app.db(), r1).await;

        for (session, expected) in [
            (s2, StatusCode::UNPROCESSABLE_ENTITY),
            (Id::new_v4(), StatusCode::NOT_FOUND),
        ] {
            let mut body = goal(r1, "Refused");
            body["created_in_session_id"] = json!(session);
            let (status, _) = app
                .expect(Caller::Coach, Method::POST, "/goals", Some(body))
                .await;
            assert_eq!(status, expected, "created in {session}");
            assert_eq!(relationship_goals(app.db(), r1).await, before);
        }
    })
    .await;
}

/// GOAL-29
#[tokio::test]
async fn participants_update_the_goal_status() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let g1 = app.fixture.g1;

            let reply = app
                .send(
                    caller,
                    Method::PUT,
                    &format!("/goals/{g1}/status?value=on_hold"),
                    None,
                )
                .await;

            assert_eq!(
                (reply.status, reply.ids()),
                (StatusCode::OK, ids([g1])),
                "{caller:?}"
            );
            assert_eq!(reply.body["data"]["status"], "OnHold", "{caller:?}");
            let status = goal_state(app.db(), g1).await.map(|(_, _, status)| status);
            assert_eq!(status, Some(Status::OnHold), "{caller:?}");
        }
    })
    .await;
}

/// GOAL-30
#[tokio::test]
async fn participants_read_the_goals_progress() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let g1 = app.fixture.g1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app
                .send(caller, Method::GET, &format!("/goals/{g1}/progress"), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            let progress = &reply.body["data"];
            assert_eq!(progress["linked_coaching_session_count"], 1, "{caller:?}");
            assert_eq!(progress["actions_total"], 0, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-31
#[tokio::test]
async fn non_participants_cannot_set_the_goal_status_or_read_its_progress() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let g1 = app.fixture.g1;
        let before = goal_state(app.db(), g1).await;
        let requests = |goal: Id| {
            [
                (Method::PUT, format!("/goals/{goal}/status?value=on_hold")),
                (Method::GET, format!("/goals/{goal}/progress")),
            ]
        };

        for caller in NON_PARTICIPANTS {
            for (method, path) in requests(g1) {
                let (status, _) = app.expect(caller, method.clone(), &path, None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method} {path}");
            }
        }
        assert_eq!(goal_state(app.db(), g1).await, before);

        for (method, path) in requests(Id::new_v4()) {
            let (status, _) = app.expect(Caller::Coach, method.clone(), &path, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
        }
    })
    .await;
}

/// GOAL-32
#[tokio::test]
async fn a_coach_creates_a_goal_from_a_session_of_the_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, s1) = (app.fixture.r1, app.fixture.s1);
        let before = relationship_goals(app.db(), r1).await;
        let mut body = goal(r1, "From the session");
        body["created_in_session_id"] = json!(s1);

        let reply = app
            .send(Caller::Coach, Method::POST, "/goals", Some(body))
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        assert_eq!(reply.body["data"]["created_in_session_id"], json!(s1));
        let created = reply.ids();
        assert_eq!(created.len(), 1, "one new goal");
        assert!(created.is_disjoint(&before));
        assert_eq!(
            relationship_goals(app.db(), r1).await,
            before.union(&created).copied().collect()
        );
        assert!(session_goals(app.db(), s1).await.is_superset(&created));
    })
    .await;
}

/// GOAL-33
#[tokio::test]
async fn signed_out_callers_cannot_reach_goals() {
    within_time_limit(assert_signed_out_refused(|app| {
        let fixture = &app.fixture;
        let (o1, r1, s1, g1) = (fixture.o1, fixture.r1, fixture.s1, fixture.g1);
        let coach = fixture.user(Caller::Coach);
        vec![
            (Method::GET, format!("/goals?coaching_relationship_id={r1}")),
            (Method::POST, "/goals".to_string()),
            (Method::GET, format!("/goals/{g1}")),
            (Method::PUT, format!("/goals/{g1}")),
            (Method::PUT, format!("/goals/{g1}/status?value=on_hold")),
            (Method::DELETE, format!("/goals/{g1}")),
            (Method::GET, format!("/goals/{g1}/sessions")),
            (Method::GET, format!("/goals/{g1}/progress")),
            (Method::GET, format!("/coaching_sessions/{s1}/goals")),
            (Method::POST, format!("/coaching_sessions/{s1}/goals")),
            (
                Method::DELETE,
                format!("/coaching_sessions/{s1}/goals/{g1}"),
            ),
            (
                Method::GET,
                format!("/coaching_sessions/goals?coaching_relationship_id={r1}"),
            ),
            (
                Method::GET,
                format!("/organizations/{o1}/coaching_relationships/{r1}/goal_progress"),
            ),
            (Method::GET, format!("/users/{coach}/goals")),
        ]
    }))
    .await;
}
