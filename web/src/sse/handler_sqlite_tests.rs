//! Who receives each real-time update.
//!
//! Each test opens several people's event streams, makes a change through the API against a
//! real database, and checks which streams receive it.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::{
    seed_active_recording, seed_content_free_session, seed_unscanned_transcription,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{assert_signed_out_refused, created_id, Listener, TestApp};

/// A goal body in `relationship`.
fn goal(relationship: Id) -> Value {
    json!({
        "coaching_relationship_id": relationship,
        "title": "Goal kumquat",
        "body": "Goal body kumquat",
        "status": "InProgress",
    })
}

/// The `field` entity ids carried by the `event` events in `received`.
fn carried(received: &[(String, Value)], event: &str, field: &str) -> BTreeSet<Id> {
    received
        .iter()
        .filter(|(name, _)| name == event)
        .filter_map(|(_, data)| data["data"][field]["id"].as_str()?.parse().ok())
        .collect()
}

/// Opens COACH's and COACHEE's streams.
async fn participants_listening(app: &mut TestApp) -> [(Caller, Listener); 2] {
    [
        (Caller::Coach, app.listen(Caller::Coach).await),
        (Caller::Coachee, app.listen(Caller::Coachee).await),
    ]
}

/// SSE-01
#[tokio::test]
async fn every_caller_opens_an_event_stream() {
    within_time_limit(async {
        let mut app = TestApp::new().await;

        for caller in Caller::ALL {
            let (status, headers) = app.open(caller, "/sse").await;

            assert_eq!(status, StatusCode::OK, "{caller:?}");
            assert_eq!(
                headers
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok()),
                Some("text/event-stream"),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SSE-02
#[tokio::test]
async fn both_participants_hear_of_an_action_assigned_to_the_coach() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, coach) = (app.fixture.s1, app.fixture.user(Caller::Coach));
        let mut listeners = participants_listening(&mut app).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                "/actions",
                Some(json!({
                    "coaching_session_id": s1,
                    "body": "Action kumquat",
                    "status": "InProgress",
                    "assignee_ids": [coach],
                })),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        let action = created_id(&reply);

        for (caller, listener) in &mut listeners {
            let received = listener.received().await;
            assert_eq!(
                carried(&received, "action_created", "action"),
                BTreeSet::from([action]),
                "{caller:?}"
            );
        }

        let reply = app
            .send(
                Caller::Coachee,
                Method::PUT,
                &format!("/actions/{action}/status?value=on_hold"),
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);

        for (caller, listener) in &mut listeners {
            let received = listener.received().await;
            assert_eq!(
                carried(&received, "action_updated", "action"),
                BTreeSet::from([action]),
                "{caller:?}"
            );
        }

        let reply = app
            .send(
                Caller::Coach,
                Method::DELETE,
                &format!("/actions/{action}"),
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);

        for (caller, listener) in &mut listeners {
            let deleted: Vec<Value> = listener
                .received()
                .await
                .into_iter()
                .filter(|(name, _)| name == "action_deleted")
                .map(|(_, data)| data["data"]["action_id"].clone())
                .collect();
            assert_eq!(deleted, vec![json!(action)], "{caller:?}");
        }
    })
    .await;
}

/// SSE-03
#[tokio::test]
async fn a_removed_coachee_hears_nothing_of_their_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (r3, s3, sr3_first) = (fixture.r3, fixture.s3, fixture.sr3_sessions[0]);
        let mut coach = app.listen(Caller::Coach).await;
        let mut removed = app.listen(Caller::Removed).await;

        let action = Some(json!({
            "coaching_session_id": s3,
            "body": "Action kumquat",
            "status": "InProgress",
        }));
        let reply = app
            .send(Caller::Coach, Method::POST, "/actions", action)
            .await;
        assert_eq!(reply.status, StatusCode::OK, "action");
        let reply = app
            .send(Caller::Coach, Method::POST, "/goals", Some(goal(r3)))
            .await;
        assert_eq!(reply.status, StatusCode::OK, "goal");
        let new_goal = created_id(&reply);
        for (method, path, body) in [
            (
                Method::POST,
                format!("/coaching_sessions/{sr3_first}/goals"),
                json!({ "goal_id": new_goal }),
            ),
            (
                Method::PATCH,
                format!("/coaching_sessions/{s3}/title"),
                json!({ "title": "Retitled kumquat" }),
            ),
            (
                Method::POST,
                format!("/coaching_sessions/{s3}/topics"),
                json!({ "body": "Topic kumquat" }),
            ),
        ] {
            let reply = app
                .send(Caller::Coach, method.clone(), &path, Some(body))
                .await;
            assert_eq!(reply.status, StatusCode::OK, "{method} {path}");
        }

        let heard: BTreeSet<String> = coach.received_types().await.into_iter().collect();
        assert_eq!(
            heard,
            [
                "action_created",
                "goal_created",
                "coaching_session_goal_created",
                "coaching_session_title_updated",
                "topics_changed",
            ]
            .map(str::to_string)
            .into(),
            "the coach hears every change"
        );
        assert_eq!(removed.received_types().await, Vec::<String>::new());
    })
    .await;
}

/// SSE-04
#[tokio::test]
async fn both_participants_hear_of_a_goal_change() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let mut listeners = participants_listening(&mut app).await;

        let reply = app
            .send(Caller::Coach, Method::POST, "/goals", Some(goal(r1)))
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        let new_goal = created_id(&reply);
        let reply = app
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/goals/{new_goal}"),
                Some(goal(r1)),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);

        for (caller, listener) in &mut listeners {
            let received = listener.received().await;
            for event in ["goal_created", "goal_updated"] {
                assert_eq!(
                    carried(&received, event, "goal"),
                    BTreeSet::from([new_goal]),
                    "{caller:?} {event}"
                );
            }
        }
    })
    .await;
}

/// SSE-05
#[tokio::test]
async fn both_participants_hear_that_suggestions_changed() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (r1, coach, coachee) = (
            fixture.r1,
            fixture.user(Caller::Coach),
            fixture.user(Caller::Coachee),
        );
        let session = seed_content_free_session(app.db(), r1).await;
        seed_unscanned_transcription(app.db(), session, coach, coachee).await;
        let mut listeners = participants_listening(&mut app).await;

        // Starting a scan announces through the same participant lookup as the sweep.
        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_sessions/{session}/transcript_suggestions/scan"),
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::ACCEPTED);

        for (caller, listener) in &mut listeners {
            assert_eq!(
                listener.received_types().await,
                vec!["transcript_suggestions_changed".to_string()],
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SSE-06
#[tokio::test]
async fn both_participants_hear_of_a_recording_update() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let bot_id = seed_active_recording(app.db(), s1).await;
        let mut listeners = participants_listening(&mut app).await;

        let status = app
            .webhook("bot.in_call_recording", json!({ "bot": { "id": bot_id } }))
            .await;
        assert_eq!(status, StatusCode::OK);

        for (caller, listener) in &mut listeners {
            let received = listener.received().await;
            let sessions: Vec<&Value> = received
                .iter()
                .filter(|(name, _)| name == "meeting_recording_updated")
                .map(|(_, data)| &data["data"]["coaching_session_id"])
                .collect();
            assert_eq!(sessions, vec![&json!(s1)], "{caller:?}");
        }
    })
    .await;
}

/// SSE-07
#[tokio::test]
async fn signed_out_callers_cannot_open_an_event_stream() {
    within_time_limit(assert_signed_out_refused(|_| {
        vec![(Method::GET, "/sse".to_string())]
    }))
    .await;
}
