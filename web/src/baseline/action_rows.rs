//! Actions (`ACT`).

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use domain::status::Status;
use domain::test_utils::sqlite::access_world::{
    action_assignees, action_state, remove_roles, session_content,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use super::harness::{ids, ids_by_key, Baseline, NON_PARTICIPANTS};

/// An action body in `session` with `assignee_ids`.
fn action(session: Id, assignee_ids: &[Id]) -> Value {
    json!({
        "coaching_session_id": session,
        "body": "Revised action kumquat",
        "status": "InProgress",
        "assignee_ids": assignee_ids,
    })
}

/// The batch action list's ids per relationship.
async fn batch_actions(
    baseline: &mut Baseline,
    caller: Caller,
    query: &str,
) -> (StatusCode, BTreeMap<Id, BTreeSet<Id>>) {
    let path = format!(
        "/organizations/{}/coaching_relationships/actions{query}",
        baseline.world.o1
    );
    let reply = baseline.send(caller, Method::GET, &path, None).await;
    let by_relationship = match reply.status {
        StatusCode::OK => ids_by_key(&reply.body["data"]["coachee_actions"]),
        _ => BTreeMap::new(),
    };
    (reply.status, by_relationship)
}

/// The per-relationship action list path.
fn relationship_actions(baseline: &Baseline, relationship: Id, query: &str) -> String {
    format!(
        "/organizations/{}/coaching_relationships/{relationship}/actions{query}",
        baseline.world.o1
    )
}

/// An action's body, status and assignees, for comparing before and after a refusal.
async fn action_snapshot(
    baseline: &Baseline,
    action: Id,
) -> (Option<(Option<String>, Status)>, Option<BTreeSet<Id>>) {
    (
        action_state(baseline.db(), action).await,
        action_assignees(baseline.db(), action).await,
    )
}

/// ACT-01
#[tokio::test]
async fn act_01_a_coach_lists_actions_per_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let expected = BTreeMap::from([
            (world.r1, ids([world.a1, world.a2, world.a3])),
            (world.r3, ids([world.a6, world.a7])),
        ]);

        assert_eq!(
            batch_actions(&mut baseline, Caller::Coach, "").await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-02
#[tokio::test]
async fn act_02_a_coachee_lists_only_own_and_unassigned_actions_per_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let expected = BTreeMap::from([
            (world.r1, ids([world.a2, world.a3])),
            (world.r2, ids([world.a5])),
        ]);

        assert_eq!(
            batch_actions(&mut baseline, Caller::Coachee, "").await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-03
#[tokio::test]
async fn act_03_another_coach_lists_only_their_relationships_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let expected = BTreeMap::from([(world.r2, ids([world.a4, world.a5]))]);

        assert_eq!(
            batch_actions(&mut baseline, Caller::OtherCoach, "").await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-04
#[tokio::test]
async fn act_04_members_in_no_relationship_list_no_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;

        for caller in [Caller::Member, Caller::Admin, Caller::Super] {
            assert_eq!(
                batch_actions(&mut baseline, caller, "").await,
                (StatusCode::OK, BTreeMap::new()),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ACT-05
#[tokio::test]
async fn act_05_non_members_cannot_list_the_orgs_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;

        for caller in [Caller::Outsider, Caller::Removed] {
            let (status, _) = batch_actions(&mut baseline, caller, "").await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ACT-06
#[tokio::test]
async fn act_06_a_coachee_who_coaches_no_one_cannot_scope_to_the_coach() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;

        let (status, _) = batch_actions(&mut baseline, Caller::Coachee, "?assignee=coach").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-07
#[tokio::test]
async fn act_07_a_coach_lists_the_relationships_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = relationship_actions(&baseline, baseline.world.r1, "");
        let world = &baseline.world;
        let expected = ids([world.a1, world.a2, world.a3]);

        assert_eq!(
            baseline
                .expect(Caller::Coach, Method::GET, &path, None)
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-08
#[tokio::test]
async fn act_08_a_coachee_lists_only_own_and_unassigned_relationship_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = relationship_actions(&baseline, baseline.world.r1, "");
        let expected = ids([baseline.world.a2, baseline.world.a3]);

        assert_eq!(
            baseline
                .expect(Caller::Coachee, Method::GET, &path, None)
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-09
#[tokio::test]
async fn act_09_a_coachee_scopes_relationship_actions_only_to_themselves() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let r1 = baseline.world.r1;
        let coachee = baseline.world.user(Caller::Coachee);
        let a2 = baseline.world.a2;

        let path = relationship_actions(&baseline, r1, "?assignee=coach");
        let (status, _) = baseline
            .expect(Caller::Coachee, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "assignee=coach");

        for scope in ["coachee".to_string(), coachee.to_string()] {
            let path = relationship_actions(&baseline, r1, &format!("?assignee={scope}"));
            assert_eq!(
                baseline
                    .expect(Caller::Coachee, Method::GET, &path, None)
                    .await,
                (StatusCode::OK, ids([a2])),
                "assignee={scope}"
            );
        }
    })
    .await;
}

/// ACT-10
#[tokio::test]
async fn act_10_non_participants_cannot_list_the_relationships_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = relationship_actions(&baseline, baseline.world.r1, "");

        for caller in [
            Caller::Member,
            Caller::OtherCoach,
            Caller::Admin,
            Caller::Super,
        ] {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ACT-11
#[tokio::test]
async fn act_11_an_outsider_cannot_list_the_relationships_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = relationship_actions(&baseline, baseline.world.r1, "");

        let (status, _) = baseline
            .expect(Caller::Outsider, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-12
#[tokio::test]
async fn act_12_a_removed_coachee_cannot_list_their_relationships_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = relationship_actions(&baseline, baseline.world.r3, "");

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-13
#[tokio::test]
async fn act_13_a_coach_lists_actions_of_a_relationship_whose_coachee_left() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = relationship_actions(&baseline, baseline.world.r3, "");
        let expected = ids([baseline.world.a6, baseline.world.a7]);

        assert_eq!(
            baseline
                .expect(Caller::Coach, Method::GET, &path, None)
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-14
#[tokio::test]
async fn act_14_a_coach_lists_the_sessions_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/actions?coaching_session_id={}", world.s1);
        let expected = ids([world.a1, world.a2, world.a3]);

        assert_eq!(
            baseline
                .expect(Caller::Coach, Method::GET, &path, None)
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-15
#[tokio::test]
async fn act_15_a_coachee_lists_every_action_of_the_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!("/actions?coaching_session_id={}", world.s1);
        let expected = ids([world.a1, world.a2, world.a3]);

        assert_eq!(
            baseline
                .expect(Caller::Coachee, Method::GET, &path, None)
                .await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-16
#[tokio::test]
async fn act_16_non_participants_cannot_list_the_sessions_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/actions?coaching_session_id={}", baseline.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ACT-17
#[tokio::test]
async fn act_17_a_removed_coachee_cannot_list_their_sessions_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/actions?coaching_session_id={}", baseline.world.s3);

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-20
#[tokio::test]
async fn act_20_a_coach_lists_their_own_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let coach = world.user(Caller::Coach);
        let sessions = ids([world.a1, world.a2, world.a3, world.a6, world.a7]);
        let assigned = ids([world.a1]);

        for (query, expected) in [("", sessions), ("?scope=assigned", assigned)] {
            assert_eq!(
                baseline
                    .expect(
                        Caller::Coach,
                        Method::GET,
                        &format!("/users/{coach}/actions{query}"),
                        None
                    )
                    .await,
                (StatusCode::OK, expected),
                "{query}"
            );
        }
    })
    .await;
}

/// ACT-21
#[tokio::test]
async fn act_21_a_removed_coachee_lists_none_of_their_own_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let removed = baseline.world.user(Caller::Removed);

        assert_eq!(
            baseline
                .expect(
                    Caller::Removed,
                    Method::GET,
                    &format!("/users/{removed}/actions"),
                    None
                )
                .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// ACT-25
#[tokio::test]
async fn act_25_members_who_do_not_coach_the_user_cannot_list_their_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/users/{}/actions", baseline.world.user(Caller::Coachee));

        for caller in [Caller::Member, Caller::Admin, Caller::Super] {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ACT-26
#[tokio::test]
async fn act_26_a_coachee_cannot_list_their_coachs_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/users/{}/actions", baseline.world.user(Caller::Coach));

        let (status, _) = baseline
            .expect(Caller::Coachee, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-26b
#[tokio::test]
async fn act_26b_a_coach_removed_from_the_org_cannot_list_their_coachees_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        remove_roles(
            baseline.db(),
            baseline.world.user(Caller::Coach),
            baseline.world.o1,
        )
        .await;
        let path = format!("/users/{}/actions", baseline.world.user(Caller::Coachee));

        let (status, _) = baseline
            .expect(Caller::Coach, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-27
#[tokio::test]
async fn act_27_participants_read_the_action() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let a1 = baseline.world.a1;

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                baseline
                    .expect(caller, Method::GET, &format!("/actions/{a1}"), None)
                    .await,
                (StatusCode::OK, ids([a1])),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ACT-28
#[tokio::test]
async fn act_28_participants_create_an_action_assigned_to_the_coachee() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let coachee = baseline.world.user(Caller::Coachee);

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = session_content(baseline.db(), s1).await;
            let reply = baseline
                .send(
                    caller,
                    Method::POST,
                    "/actions",
                    Some(action(s1, &[coachee])),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created: Vec<Id> = reply.ids().into_iter().collect();
            let [new_action] = created[..] else {
                panic!("{caller:?}: one new action id, got {created:?}");
            };
            assert!(!before.contains(&new_action), "{caller:?}");
            assert!(
                session_content(baseline.db(), s1)
                    .await
                    .contains(&new_action),
                "{caller:?}"
            );
            assert_eq!(
                action_assignees(baseline.db(), new_action).await,
                Some(BTreeSet::from([coachee])),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ACT-29
#[tokio::test]
async fn act_29_participants_update_the_action() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, a1) = (baseline.world.s1, baseline.world.a1);

        for (caller, text) in [(Caller::Coach, "By coach"), (Caller::Coachee, "By coachee")] {
            let body = json!({
                "coaching_session_id": s1,
                "body": text,
                "status": "InProgress",
            });
            assert_eq!(
                baseline
                    .expect(caller, Method::PUT, &format!("/actions/{a1}"), Some(body))
                    .await,
                (StatusCode::OK, ids([a1])),
                "{caller:?}"
            );
            assert_eq!(
                action_state(baseline.db(), a1).await,
                Some((Some(text.to_string()), Status::InProgress)),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ACT-30
#[tokio::test]
async fn act_30_participants_update_the_action_status() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut baseline = Baseline::new().await;
            let a1 = baseline.world.a1;

            assert_eq!(
                baseline
                    .expect(
                        caller,
                        Method::PUT,
                        &format!("/actions/{a1}/status?value=completed"),
                        None
                    )
                    .await,
                (StatusCode::OK, ids([a1])),
                "{caller:?}"
            );
            let status = action_state(baseline.db(), a1)
                .await
                .map(|(_, status)| status);
            assert_eq!(status, Some(Status::Completed), "{caller:?}");
        }
    })
    .await;
}

/// ACT-31
#[tokio::test]
async fn act_31_participants_delete_the_action() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut baseline = Baseline::new().await;
            let a1 = baseline.world.a1;

            let reply = baseline
                .send(caller, Method::DELETE, &format!("/actions/{a1}"), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["id"], json!(a1), "{caller:?}");
            assert_eq!(action_state(baseline.db(), a1).await, None, "{caller:?}");
        }
    })
    .await;
}

/// ACT-32
#[tokio::test]
async fn act_32_non_participants_cannot_read_the_action() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let a1 = baseline.world.a1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(caller, Method::GET, &format!("/actions/{a1}"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        let (status, _) = baseline
            .expect(
                Caller::Coach,
                Method::GET,
                &format!("/actions/{}", Id::new_v4()),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "an unknown action");
    })
    .await;
}

/// ACT-33
#[tokio::test]
async fn act_33_non_participants_cannot_create_an_action_in_the_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let coachee = baseline.world.user(Caller::Coachee);
        let before = session_content(baseline.db(), s1).await;
        let emails = baseline.expect_emails(0).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline
                .expect(
                    caller,
                    Method::POST,
                    "/actions",
                    Some(action(s1, &[coachee])),
                )
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
                "/actions",
                Some(action(Id::new_v4(), &[coachee])),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "an unknown session");
        emails.assert_async().await;
    })
    .await;
}

/// ACT-34
#[tokio::test]
async fn act_34_non_participants_cannot_change_the_action() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, a1) = (baseline.world.s1, baseline.world.a1);
        let coach = baseline.world.user(Caller::Coach);
        let before = action_snapshot(&baseline, a1).await;

        for caller in NON_PARTICIPANTS {
            for (method, path, body) in [
                (
                    Method::PUT,
                    format!("/actions/{a1}"),
                    Some(action(s1, &[coach])),
                ),
                (
                    Method::PUT,
                    format!("/actions/{a1}/status?value=completed"),
                    None,
                ),
                (Method::DELETE, format!("/actions/{a1}"), None),
            ] {
                let (status, _) = baseline.expect(caller, method.clone(), &path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method} {path}");
                assert_eq!(
                    action_snapshot(&baseline, a1).await,
                    before,
                    "{caller:?} {method} {path}"
                );
            }
        }
    })
    .await;
}

/// ACT-35
#[tokio::test]
async fn act_35_a_removed_coachee_cannot_reach_their_sessions_actions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s3, a6) = (baseline.world.s3, baseline.world.a6);
        let removed = baseline.world.user(Caller::Removed);
        let before = action_snapshot(&baseline, a6).await;
        let content = session_content(baseline.db(), s3).await;
        let path = format!("/actions/{a6}");

        for (method, path, body) in [
            (Method::GET, path.as_str(), None),
            (Method::PUT, path.as_str(), Some(action(s3, &[removed]))),
            (Method::DELETE, path.as_str(), None),
            (Method::POST, "/actions", Some(action(s3, &[removed]))),
        ] {
            let (status, _) = baseline
                .expect(Caller::Removed, method.clone(), path, body)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
        assert_eq!(action_snapshot(&baseline, a6).await, before);
        assert_eq!(session_content(baseline.db(), s3).await, content);
    })
    .await;
}

/// ACT-36
#[tokio::test]
async fn act_36_assignees_outside_the_relationship_are_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, a3) = (baseline.world.s1, baseline.world.a3);
        let member = baseline.world.user(Caller::Member);
        let content = session_content(baseline.db(), s1).await;
        let a3_before = action_snapshot(&baseline, a3).await;
        let emails = baseline.expect_emails(0).await;

        let (status, _) = baseline
            .expect(
                Caller::Coach,
                Method::POST,
                "/actions",
                Some(action(s1, &[member])),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "POST");
        assert_eq!(session_content(baseline.db(), s1).await, content, "POST");

        let (status, _) = baseline
            .expect(
                Caller::Coach,
                Method::PUT,
                &format!("/actions/{a3}"),
                Some(action(s1, &[member])),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "PUT");
        assert_eq!(action_snapshot(&baseline, a3).await, a3_before, "PUT");
        emails.assert_async().await;
    })
    .await;
}

/// Asserts a 409 naming REMOVED as an assignee no longer in O1.
fn assert_assignee_not_a_member(baseline: &Baseline, body: &Value) {
    let details = &body["details"];
    assert_eq!(details["reason"], "assignee_not_a_member", "{body}");
    assert_eq!(
        details["user_id"],
        json!(baseline.world.user(Caller::Removed)),
        "{body}"
    );
    assert_eq!(
        details["organization_id"],
        json!(baseline.world.o1),
        "{body}"
    );
}

/// ACT-37
#[tokio::test]
async fn act_37_assigning_a_coachee_who_left_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s3 = baseline.world.s3;
        let removed = baseline.world.user(Caller::Removed);
        let content = session_content(baseline.db(), s3).await;
        let emails = baseline.expect_emails(0).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::POST,
                "/actions",
                Some(action(s3, &[removed])),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_assignee_not_a_member(&baseline, &reply.body);
        assert_eq!(session_content(baseline.db(), s3).await, content);
        emails.assert_async().await;
    })
    .await;
}

/// ACT-38
#[tokio::test]
async fn act_38_a_coach_edits_an_action_still_assigned_to_a_coachee_who_left() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s3, a6) = (baseline.world.s3, baseline.world.a6);
        let removed = baseline.world.user(Caller::Removed);
        let emails = baseline.expect_emails(0).await;
        let body = json!({
            "coaching_session_id": s3,
            "body": "Edited kumquat",
            "status": "InProgress",
            "assignee_ids": [removed],
        });

        assert_eq!(
            baseline
                .expect(
                    Caller::Coach,
                    Method::PUT,
                    &format!("/actions/{a6}"),
                    Some(body)
                )
                .await,
            (StatusCode::OK, ids([a6]))
        );
        assert_eq!(
            action_state(baseline.db(), a6).await,
            Some((Some("Edited kumquat".to_string()), Status::InProgress))
        );
        assert_eq!(
            action_assignees(baseline.db(), a6).await,
            Some(BTreeSet::from([removed]))
        );
        emails.assert_async().await;
    })
    .await;
}

/// ACT-39
#[tokio::test]
async fn act_39_adding_a_coachee_who_left_as_an_assignee_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s3, a7) = (baseline.world.s3, baseline.world.a7);
        let removed = baseline.world.user(Caller::Removed);
        let before = action_snapshot(&baseline, a7).await;
        let emails = baseline.expect_emails(0).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/actions/{a7}"),
                Some(action(s3, &[removed])),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_assignee_not_a_member(&baseline, &reply.body);
        assert_eq!(action_snapshot(&baseline, a7).await, before);
        emails.assert_async().await;
    })
    .await;
}

/// ACT-40
#[tokio::test]
async fn act_40_a_goal_from_another_relationship_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, g2) = (baseline.world.s1, baseline.world.g2);
        let content = session_content(baseline.db(), s1).await;
        let mut body = action(s1, &[]);
        body["goal_id"] = json!(g2);

        let (status, _) = baseline
            .expect(Caller::Coach, Method::POST, "/actions", Some(body))
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(session_content(baseline.db(), s1).await, content);
    })
    .await;
}
