//! Real-time events (`SSE`): who each change reaches, read from the callers' open streams.

use std::collections::BTreeSet;

use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{
    seed_active_recording, seed_content_free_session, seed_unscanned_transcription,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use super::harness::{Baseline, Listener, Reply};

/// A goal body in `relationship`.
fn goal(relationship: Id) -> Value {
    json!({
        "coaching_relationship_id": relationship,
        "title": "Goal kumquat",
        "body": "Goal body kumquat",
        "status": "InProgress",
    })
}

/// The new entity's id from a create reply.
fn created_id(reply: &Reply) -> Id {
    let created: Vec<Id> = reply.ids().into_iter().collect();
    let [id] = created[..] else {
        panic!("one new id, got {created:?}");
    };
    id
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
async fn participants_listening(baseline: &mut Baseline) -> [(Caller, Listener); 2] {
    [
        (Caller::Coach, baseline.listen(Caller::Coach).await),
        (Caller::Coachee, baseline.listen(Caller::Coachee).await),
    ]
}

/// SSE-01
#[tokio::test]
async fn sse_01_every_caller_opens_an_event_stream() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;

        for caller in Caller::ALL {
            let (status, headers) = baseline.open(caller, "/sse").await;

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
async fn sse_02_both_participants_hear_of_an_action_assigned_to_the_coach() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, coach) = (baseline.world.s1, baseline.world.user(Caller::Coach));
        let mut listeners = participants_listening(&mut baseline).await;

        let reply = baseline
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
    })
    .await;
}

/// SSE-03
#[tokio::test]
async fn sse_03_a_removed_coachee_hears_nothing_of_their_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let (r3, s3, sr3_first) = (world.r3, world.s3, world.sr3_sessions[0]);
        let mut coach = baseline.listen(Caller::Coach).await;
        let mut removed = baseline.listen(Caller::Removed).await;

        let action = Some(json!({
            "coaching_session_id": s3,
            "body": "Action kumquat",
            "status": "InProgress",
        }));
        let reply = baseline
            .send(Caller::Coach, Method::POST, "/actions", action)
            .await;
        assert_eq!(reply.status, StatusCode::OK, "action");
        let reply = baseline
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
            let reply = baseline
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
async fn sse_04_both_participants_hear_of_a_goal_change() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let r1 = baseline.world.r1;
        let mut listeners = participants_listening(&mut baseline).await;

        let reply = baseline
            .send(Caller::Coach, Method::POST, "/goals", Some(goal(r1)))
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        let new_goal = created_id(&reply);
        let reply = baseline
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
async fn sse_05_both_participants_hear_that_suggestions_changed() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let (r1, coach, coachee) = (
            world.r1,
            world.user(Caller::Coach),
            world.user(Caller::Coachee),
        );
        let session = seed_content_free_session(baseline.db(), r1).await;
        seed_unscanned_transcription(baseline.db(), session, coach, coachee).await;
        let mut listeners = participants_listening(&mut baseline).await;

        // Starting a scan announces through the same participant lookup as the sweep.
        let reply = baseline
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
async fn sse_06_both_participants_hear_of_a_recording_update() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let bot_id = seed_active_recording(baseline.db(), s1).await;
        let mut listeners = participants_listening(&mut baseline).await;

        let status = baseline
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
