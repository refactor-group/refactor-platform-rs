//! Who can read, create, change and delete actions.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::status::Status;
use domain::test_utils::sqlite::read_back::{action_assignees, action_state, session_content};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{assert_signed_out_refused, ids, TestApp, NON_PARTICIPANTS};

/// An action body in `session` with `assignee_ids`.
fn action(session: Id, assignee_ids: &[Id]) -> Value {
    json!({
        "coaching_session_id": session,
        "body": "Revised action kumquat",
        "status": "InProgress",
        "assignee_ids": assignee_ids,
    })
}

/// An action's body, status and assignees, for comparing before and after a refusal.
async fn action_snapshot(
    app: &TestApp,
    action: Id,
) -> (Option<(Option<String>, Status)>, Option<BTreeSet<Id>>) {
    (
        action_state(app.db(), action).await,
        action_assignees(app.db(), action).await,
    )
}

/// Asserts a 409 naming REMOVED as an assignee no longer in O1.
fn assert_assignee_not_a_member(app: &TestApp, body: &Value) {
    let details = &body["details"];
    assert_eq!(details["reason"], "assignee_not_a_member", "{body}");
    assert_eq!(
        details["user_id"],
        json!(app.fixture.user(Caller::Removed)),
        "{body}"
    );
    assert_eq!(details["organization_id"], json!(app.fixture.o1), "{body}");
}

/// ACT-14
#[tokio::test]
async fn a_coach_lists_the_sessions_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let path = format!("/actions?coaching_session_id={}", fixture.s1);
        let expected = ids([fixture.a1, fixture.a2, fixture.a3]);

        assert_eq!(
            app.expect(Caller::Coach, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-15
#[tokio::test]
async fn a_coachee_lists_every_action_of_the_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let path = format!("/actions?coaching_session_id={}", fixture.s1);
        let expected = ids([fixture.a1, fixture.a2, fixture.a3]);

        assert_eq!(
            app.expect(Caller::Coachee, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// ACT-16
#[tokio::test]
async fn non_participants_cannot_list_the_sessions_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/actions?coaching_session_id={}", app.fixture.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ACT-17
#[tokio::test]
async fn a_removed_coachee_cannot_list_their_sessions_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/actions?coaching_session_id={}", app.fixture.s3);

        let (status, _) = app.expect(Caller::Removed, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// ACT-27
#[tokio::test]
async fn participants_read_the_action() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let a1 = app.fixture.a1;
        let coach = app.fixture.user(Caller::Coach);

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app
                .send(caller, Method::GET, &format!("/actions/{a1}"), None)
                .await;
            assert_eq!(
                (reply.status, reply.ids()),
                (StatusCode::OK, ids([a1])),
                "{caller:?}"
            );
            assert_eq!(
                reply.body["data"]["assignee_ids"],
                json!([coach]),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ACT-28
#[tokio::test]
async fn participants_create_an_action_assigned_to_the_coachee() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let coachee = app.fixture.user(Caller::Coachee);

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = session_content(app.db(), s1).await;
            let reply = app
                .send(
                    caller,
                    Method::POST,
                    "/actions",
                    Some(action(s1, &[coachee])),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            assert_eq!(
                reply.body["data"]["assignee_ids"],
                json!([coachee]),
                "{caller:?}"
            );
            let created: Vec<Id> = reply.ids().into_iter().collect();
            let [new_action] = created[..] else {
                panic!("{caller:?}: one new action id, got {created:?}");
            };
            assert!(!before.contains(&new_action), "{caller:?}");
            assert!(
                session_content(app.db(), s1).await.contains(&new_action),
                "{caller:?}"
            );
            assert_eq!(
                action_assignees(app.db(), new_action).await,
                Some(BTreeSet::from([coachee])),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ACT-29
#[tokio::test]
async fn participants_update_the_action() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, a1) = (app.fixture.s1, app.fixture.a1);

        for (caller, text) in [(Caller::Coach, "By coach"), (Caller::Coachee, "By coachee")] {
            let body = json!({
                "coaching_session_id": s1,
                "body": text,
                "status": "InProgress",
            });
            assert_eq!(
                app.expect(caller, Method::PUT, &format!("/actions/{a1}"), Some(body))
                    .await,
                (StatusCode::OK, ids([a1])),
                "{caller:?}"
            );
            assert_eq!(
                action_state(app.db(), a1).await,
                Some((Some(text.to_string()), Status::InProgress)),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ACT-30
#[tokio::test]
async fn participants_update_the_action_status() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let a1 = app.fixture.a1;

            let reply = app
                .send(
                    caller,
                    Method::PUT,
                    &format!("/actions/{a1}/status?value=completed"),
                    None,
                )
                .await;
            assert_eq!(
                (reply.status, reply.ids()),
                (StatusCode::OK, ids([a1])),
                "{caller:?}"
            );
            assert_eq!(reply.body["data"]["status"], "Completed", "{caller:?}");
            let status = action_state(app.db(), a1).await.map(|(_, status)| status);
            assert_eq!(status, Some(Status::Completed), "{caller:?}");
        }
    })
    .await;
}

/// ACT-31
#[tokio::test]
async fn participants_delete_the_action() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let a1 = app.fixture.a1;

            let reply = app
                .send(caller, Method::DELETE, &format!("/actions/{a1}"), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["id"], json!(a1), "{caller:?}");
            assert_eq!(action_state(app.db(), a1).await, None, "{caller:?}");
        }
    })
    .await;
}

/// ACT-32
#[tokio::test]
async fn non_participants_cannot_read_the_action() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let a1 = app.fixture.a1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = app
                .expect(caller, Method::GET, &format!("/actions/{a1}"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        let (status, _) = app
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
async fn non_participants_cannot_create_an_action_in_the_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let coachee = app.fixture.user(Caller::Coachee);
        let before = session_content(app.db(), s1).await;
        let emails = app.expect_emails(0).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = app
                .expect(
                    caller,
                    Method::POST,
                    "/actions",
                    Some(action(s1, &[coachee])),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(session_content(app.db(), s1).await, before, "{caller:?}");
        }
        let (status, _) = app
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
async fn non_participants_cannot_change_the_action() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, a1) = (app.fixture.s1, app.fixture.a1);
        let coach = app.fixture.user(Caller::Coach);
        let before = action_snapshot(&app, a1).await;

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
                let (status, _) = app.expect(caller, method.clone(), &path, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method} {path}");
                assert_eq!(
                    action_snapshot(&app, a1).await,
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
async fn a_removed_coachee_cannot_reach_their_sessions_actions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s3, a6) = (app.fixture.s3, app.fixture.a6);
        let removed = app.fixture.user(Caller::Removed);
        let before = action_snapshot(&app, a6).await;
        let content = session_content(app.db(), s3).await;
        let path = format!("/actions/{a6}");

        for (method, path, body) in [
            (Method::GET, path.as_str(), None),
            (Method::PUT, path.as_str(), Some(action(s3, &[removed]))),
            (Method::DELETE, path.as_str(), None),
            (Method::POST, "/actions", Some(action(s3, &[removed]))),
        ] {
            let (status, _) = app
                .expect(Caller::Removed, method.clone(), path, body)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        }
        assert_eq!(action_snapshot(&app, a6).await, before);
        assert_eq!(session_content(app.db(), s3).await, content);
    })
    .await;
}

/// ACT-36
#[tokio::test]
async fn assignees_outside_the_relationship_are_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, a3) = (app.fixture.s1, app.fixture.a3);
        let member = app.fixture.user(Caller::Member);
        let content = session_content(app.db(), s1).await;
        let a3_before = action_snapshot(&app, a3).await;
        let emails = app.expect_emails(0).await;

        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::POST,
                "/actions",
                Some(action(s1, &[member])),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "POST");
        assert_eq!(session_content(app.db(), s1).await, content, "POST");

        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::PUT,
                &format!("/actions/{a3}"),
                Some(action(s1, &[member])),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "PUT");
        assert_eq!(action_snapshot(&app, a3).await, a3_before, "PUT");
        emails.assert_async().await;
    })
    .await;
}

/// ACT-37
#[tokio::test]
async fn assigning_a_coachee_who_left_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s3 = app.fixture.s3;
        let removed = app.fixture.user(Caller::Removed);
        let content = session_content(app.db(), s3).await;
        let emails = app.expect_emails(0).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                "/actions",
                Some(action(s3, &[removed])),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_assignee_not_a_member(&app, &reply.body);
        assert_eq!(session_content(app.db(), s3).await, content);
        emails.assert_async().await;
    })
    .await;
}

/// ACT-38
#[tokio::test]
async fn a_coach_edits_an_action_still_assigned_to_a_coachee_who_left() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s3, a6) = (app.fixture.s3, app.fixture.a6);
        let removed = app.fixture.user(Caller::Removed);
        let emails = app.expect_emails(0).await;
        let body = json!({
            "coaching_session_id": s3,
            "body": "Edited kumquat",
            "status": "InProgress",
            "assignee_ids": [removed],
        });

        let reply = app
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/actions/{a6}"),
                Some(body),
            )
            .await;
        assert_eq!((reply.status, reply.ids()), (StatusCode::OK, ids([a6])));
        assert_eq!(reply.body["data"]["assignee_ids"], json!([removed]));
        assert_eq!(
            action_state(app.db(), a6).await,
            Some((Some("Edited kumquat".to_string()), Status::InProgress))
        );
        assert_eq!(
            action_assignees(app.db(), a6).await,
            Some(BTreeSet::from([removed]))
        );
        emails.assert_async().await;
    })
    .await;
}

/// ACT-39
#[tokio::test]
async fn adding_a_coachee_who_left_as_an_assignee_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s3, a7) = (app.fixture.s3, app.fixture.a7);
        let removed = app.fixture.user(Caller::Removed);
        let before = action_snapshot(&app, a7).await;
        let emails = app.expect_emails(0).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/actions/{a7}"),
                Some(action(s3, &[removed])),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_assignee_not_a_member(&app, &reply.body);
        assert_eq!(action_snapshot(&app, a7).await, before);
        emails.assert_async().await;
    })
    .await;
}

/// ACT-40
#[tokio::test]
async fn a_goal_from_another_relationship_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, g2) = (app.fixture.s1, app.fixture.g2);
        let content = session_content(app.db(), s1).await;
        let mut body = action(s1, &[]);
        body["goal_id"] = json!(g2);

        let (status, _) = app
            .expect(Caller::Coach, Method::POST, "/actions", Some(body))
            .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(session_content(app.db(), s1).await, content);
    })
    .await;
}

/// ACT-42
#[tokio::test]
async fn signed_out_callers_cannot_reach_actions() {
    within_time_limit(assert_signed_out_refused(|app| {
        let fixture = &app.fixture;
        let (o1, r1, s1, a1) = (fixture.o1, fixture.r1, fixture.s1, fixture.a1);
        let coachee = fixture.user(Caller::Coachee);
        vec![
            (Method::GET, format!("/actions?coaching_session_id={s1}")),
            (Method::POST, "/actions".to_string()),
            (Method::GET, format!("/actions/{a1}")),
            (Method::PUT, format!("/actions/{a1}")),
            (Method::PUT, format!("/actions/{a1}/status?value=completed")),
            (Method::DELETE, format!("/actions/{a1}")),
            (Method::GET, format!("/users/{coachee}/actions")),
            (
                Method::GET,
                format!("/organizations/{o1}/coaching_relationships/actions"),
            ),
            (
                Method::GET,
                format!("/organizations/{o1}/coaching_relationships/{r1}/actions"),
            ),
        ]
    }))
    .await;
}
