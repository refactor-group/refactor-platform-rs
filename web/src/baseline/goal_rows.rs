//! Goals (`GOAL`).

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{goal_state, relationship_goals, session_goals};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use super::harness::{ids, ids_by_key, Baseline, NON_PARTICIPANTS};

/// A goal body in `relationship`, titled `title`.
fn goal(relationship: Id, title: &str) -> Value {
    json!({
        "coaching_relationship_id": relationship,
        "title": title,
        "body": "Goal body kumquat",
        "status": "InProgress",
    })
}

/// `field` of every object in `value`, as ids.
fn field_ids(value: &Value, field: &str) -> BTreeSet<Id> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(|item| item[field].as_str().and_then(|id| id.parse().ok()))
        .collect()
}

/// The batch session-goals read's ids per session.
async fn batch_goals(
    baseline: &mut Baseline,
    caller: Caller,
    query: &str,
) -> (StatusCode, BTreeMap<Id, BTreeSet<Id>>) {
    let reply = baseline
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

/// The goal progress path for R1.
fn goal_progress(baseline: &Baseline, query: &str) -> String {
    format!(
        "/organizations/{}/coaching_relationships/{}/goal_progress{query}",
        baseline.world.o1, baseline.world.r1
    )
}

/// GOAL-01
#[tokio::test]
async fn goal_01_participants_list_the_relationships_goals() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/goals?coaching_relationship_id={}", baseline.world.r1);
        let expected = ids([baseline.world.g1]);

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

/// GOAL-02
#[tokio::test]
async fn goal_02_non_participants_cannot_list_the_relationships_goals() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/goals?coaching_relationship_id={}", baseline.world.r1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-03
#[tokio::test]
async fn goal_03_a_removed_coachee_cannot_list_their_relationships_goals() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/goals?coaching_relationship_id={}", baseline.world.r3);

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// GOAL-04
#[tokio::test]
async fn goal_04_participants_read_the_goal() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let g1 = baseline.world.g1;

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                baseline
                    .expect(caller, Method::GET, &format!("/goals/{g1}"), None)
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
async fn goal_05_non_participants_cannot_read_the_goal() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let g1 = baseline.world.g1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(caller, Method::GET, &format!("/goals/{g1}"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        let (status, _) = baseline
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
async fn goal_06_participants_update_the_goal() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (r1, g1) = (baseline.world.r1, baseline.world.g1);

        for (caller, title) in [(Caller::Coach, "By coach"), (Caller::Coachee, "By coachee")] {
            assert_eq!(
                baseline
                    .expect(
                        caller,
                        Method::PUT,
                        &format!("/goals/{g1}"),
                        Some(goal(r1, title))
                    )
                    .await,
                (StatusCode::OK, ids([g1])),
                "{caller:?}"
            );
            let stored = goal_state(baseline.db(), g1).await.and_then(|(t, _, _)| t);
            assert_eq!(stored.as_deref(), Some(title), "{caller:?}");
        }
    })
    .await;
}

/// GOAL-07
#[tokio::test]
async fn goal_07_participants_delete_the_goal() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut baseline = Baseline::new().await;
            let g1 = baseline.world.g1;

            let reply = baseline
                .send(caller, Method::DELETE, &format!("/goals/{g1}"), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["id"], json!(g1), "{caller:?}");
            assert_eq!(goal_state(baseline.db(), g1).await, None, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-08
#[tokio::test]
async fn goal_08_non_participants_cannot_change_the_goal() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (r1, g1) = (baseline.world.r1, baseline.world.g1);
        let path = format!("/goals/{g1}");
        let before = goal_state(baseline.db(), g1).await;

        for caller in NON_PARTICIPANTS {
            for (method, body) in [
                (Method::PUT, Some(goal(r1, "Refused"))),
                (Method::DELETE, None),
            ] {
                let (status, _) = baseline.expect(caller, method.clone(), &path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
                assert_eq!(
                    goal_state(baseline.db(), g1).await,
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
async fn goal_09_participants_list_the_goals_sessions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let g1 = baseline.world.g1;
        let expected = ids([baseline.world.s1]);

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline
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
async fn goal_10_participants_create_a_goal_in_the_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let r1 = baseline.world.r1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = relationship_goals(baseline.db(), r1).await;
            let reply = baseline
                .send(caller, Method::POST, "/goals", Some(goal(r1, "New goal")))
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created = reply.ids();
            assert_eq!(created.len(), 1, "{caller:?}: one new goal");
            assert!(created.is_disjoint(&before), "{caller:?}");
            assert_eq!(
                relationship_goals(baseline.db(), r1).await,
                before.union(&created).copied().collect(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-11
#[tokio::test]
async fn goal_11_participants_list_the_sessions_goals() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/coaching_sessions/{}/goals", baseline.world.s1);
        let expected = ids([baseline.world.g1]);

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

/// GOAL-12
#[tokio::test]
async fn goal_12_non_participants_cannot_list_the_sessions_goals() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/coaching_sessions/{}/goals", baseline.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-14
#[tokio::test]
async fn goal_14_participants_unlink_a_goal_from_the_session() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut baseline = Baseline::new().await;
            let (s1, g1) = (baseline.world.s1, baseline.world.g1);

            let reply = baseline
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
                session_goals(baseline.db(), s1).await,
                BTreeSet::new(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-15
#[tokio::test]
async fn goal_15_participants_read_goals_per_session_of_the_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let query = format!("coaching_relationship_id={}", world.r1);
        let expected = BTreeMap::from([
            (world.s1, ids([world.g1])),
            (world.sr1_sessions[0], BTreeSet::new()),
            (world.sr1_sessions[1], BTreeSet::new()),
        ]);

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                batch_goals(&mut baseline, caller, &query).await,
                (StatusCode::OK, expected.clone()),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// GOAL-16
#[tokio::test]
async fn goal_16_non_participants_cannot_read_goals_per_session_of_the_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let query = format!("coaching_relationship_id={}", baseline.world.r1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = batch_goals(&mut baseline, caller, &query).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-17
#[tokio::test]
async fn goal_17_every_batch_session_must_be_the_callers() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, s2, g1) = (baseline.world.s1, baseline.world.s2, baseline.world.g1);

        for query in [
            format!("coaching_session_ids={s1},{s2}"),
            format!("coaching_session_ids={s2},{s1}"),
        ] {
            let (status, _) = batch_goals(&mut baseline, Caller::Coach, &query).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{query}");
        }
        assert_eq!(
            batch_goals(
                &mut baseline,
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
async fn goal_18_non_participants_cannot_batch_read_the_sessions_goals() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let query = format!("coaching_session_ids={}", baseline.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = batch_goals(&mut baseline, caller, &query).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-19
#[tokio::test]
async fn goal_19_participants_read_the_relationships_goal_progress() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let g1 = baseline.world.g1;
        let coachee = baseline.world.user(Caller::Coachee);
        let queries = [
            String::new(),
            "?assignee=coach".to_string(),
            "?assignee=coachee".to_string(),
            format!("?assignee={coachee}"),
        ];

        for caller in [Caller::Coach, Caller::Coachee] {
            for query in &queries {
                let path = goal_progress(&baseline, query);
                let reply = baseline.send(caller, Method::GET, &path, None).await;
                assert_eq!(reply.status, StatusCode::OK, "{caller:?} {query}");
                assert_eq!(
                    field_ids(&reply.body["data"]["goal_progress"], "goal_id"),
                    ids([g1]),
                    "{caller:?} {query}"
                );
            }
        }
    })
    .await;
}

/// GOAL-20
#[tokio::test]
async fn goal_20_goal_progress_refuses_a_non_participant_assignee() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let member = baseline.world.user(Caller::Member);
        let path = goal_progress(&baseline, &format!("?assignee={member}"));

        let (status, _) = baseline
            .expect(Caller::Coach, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    })
    .await;
}

/// GOAL-21
#[tokio::test]
async fn goal_21_non_participants_cannot_read_the_relationships_goal_progress() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = goal_progress(&baseline, "");

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-22
#[tokio::test]
async fn goal_22_a_coach_lists_the_goals_they_created() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let coach = baseline.world.user(Caller::Coach);
        let expected = ids([baseline.world.g1]);

        assert_eq!(
            baseline
                .expect(
                    Caller::Coach,
                    Method::GET,
                    &format!("/users/{coach}/goals"),
                    None
                )
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// GOAL-23
#[tokio::test]
async fn goal_23_a_coachee_who_created_no_goal_lists_none() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let coachee = baseline.world.user(Caller::Coachee);

        assert_eq!(
            baseline
                .expect(
                    Caller::Coachee,
                    Method::GET,
                    &format!("/users/{coachee}/goals"),
                    None
                )
                .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// GOAL-24
#[tokio::test]
async fn goal_24_only_the_user_lists_their_own_goals() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/users/{}/goals", baseline.world.user(Caller::Coachee));

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// GOAL-25
#[tokio::test]
async fn goal_25_non_participants_cannot_create_a_goal_in_the_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (r1, r3) = (baseline.world.r1, baseline.world.r3);
        let r1_goals = relationship_goals(baseline.db(), r1).await;
        let r3_goals = relationship_goals(baseline.db(), r3).await;

        let refusals = NON_PARTICIPANTS
            .into_iter()
            .map(|caller| (caller, r1))
            .chain([(Caller::Removed, r3)]);
        for (caller, relationship) in refusals {
            let (status, _) = baseline
                .expect(
                    caller,
                    Method::POST,
                    "/goals",
                    Some(goal(relationship, "Refused")),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        assert_eq!(relationship_goals(baseline.db(), r1).await, r1_goals);
        assert_eq!(relationship_goals(baseline.db(), r3).await, r3_goals);

        let (status, _) = baseline
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

/// GOAL-26
#[tokio::test]
async fn goal_26_non_participants_cannot_change_the_sessions_goal_links() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, s3, g1) = (baseline.world.s1, baseline.world.s3, baseline.world.g1);
        let s1_links = session_goals(baseline.db(), s1).await;
        let s3_links = session_goals(baseline.db(), s3).await;

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
                let (status, _) = baseline.expect(caller, method.clone(), &path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
            }
        }
        assert_eq!(session_goals(baseline.db(), s1).await, s1_links);
        assert_eq!(session_goals(baseline.db(), s3).await, s3_links);
    })
    .await;
}

/// GOAL-27
#[tokio::test]
async fn goal_27_linking_a_goal_from_another_relationship_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, g2) = (baseline.world.s1, baseline.world.g2);
        let links = session_goals(baseline.db(), s1).await;

        let (status, _) = baseline
            .expect(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_sessions/{s1}/goals"),
                Some(json!({ "goal_id": g2 })),
            )
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(session_goals(baseline.db(), s1).await, links);
    })
    .await;
}

/// GOAL-28
#[tokio::test]
async fn goal_28_a_created_in_session_from_another_relationship_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (r1, s2) = (baseline.world.r1, baseline.world.s2);
        let before = relationship_goals(baseline.db(), r1).await;

        for (session, expected) in [
            (s2, StatusCode::UNPROCESSABLE_ENTITY),
            (Id::new_v4(), StatusCode::NOT_FOUND),
        ] {
            let mut body = goal(r1, "Refused");
            body["created_in_session_id"] = json!(session);
            let (status, _) = baseline
                .expect(Caller::Coach, Method::POST, "/goals", Some(body))
                .await;
            assert_eq!(status, expected, "created in {session}");
            assert_eq!(relationship_goals(baseline.db(), r1).await, before);
        }
    })
    .await;
}
