//! Who can scan a session's transcript and accept, dismiss or restore its suggested actions.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in `transcript_suggestion_controller_tests.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::{
    seed_content_free_session, seed_unscanned_transcription,
};
use domain::test_utils::sqlite::read_back::{
    action_assignees, session_content, suggestion_outcome, transcription_scanned,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::transcript_suggestion::Status;
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{ids, TestApp, NON_PARTICIPANTS};

/// S1's suggestions path, plus `suffix`.
fn suggestions(app: &TestApp, suffix: &str) -> String {
    format!(
        "/coaching_sessions/{}/transcript_suggestions{suffix}",
        app.fixture.s1
    )
}

/// The suggestion statuses and S1's content, to check a refusal changed nothing.
async fn suggestion_state(app: &TestApp) -> (Vec<(Status, Option<Id>)>, BTreeSet<Id>) {
    let fixture = &app.fixture;
    let mut outcomes = Vec::new();
    for suggestion in [fixture.sg1, fixture.sg2, fixture.sg3] {
        outcomes.push(suggestion_outcome(app.db(), suggestion).await);
    }
    (outcomes, session_content(app.db(), fixture.s1).await)
}

/// TRN-07
#[tokio::test]
async fn participants_list_the_sessions_suggestions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = suggestions(&app, "");
        let expected = ids([app.fixture.sg1, app.fixture.sg2]);

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

/// TRN-08
#[tokio::test]
async fn non_participants_cannot_list_the_sessions_suggestions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = suggestions(&app, "");

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// TRN-09
#[tokio::test]
async fn participants_read_and_start_a_scan() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let fixture = &app.fixture;
            let (r1, coach, coachee) = (
                fixture.r1,
                fixture.user(Caller::Coach),
                fixture.user(Caller::Coachee),
            );
            let scan = suggestions(&app, "/scan");

            let reply = app.send(caller, Method::GET, &scan, None).await;
            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["data"]["status"], "completed", "{caller:?}");

            // The matrix's note: S1's scan already completed, so it is not repeated.
            let reply = app.send(caller, Method::POST, &scan, None).await;
            assert_eq!(reply.status, StatusCode::CONFLICT, "{caller:?}");

            let session = seed_content_free_session(app.db(), r1).await;
            let transcription =
                seed_unscanned_transcription(app.db(), session, coach, coachee).await;
            let reply = app
                .send(
                    caller,
                    Method::POST,
                    &format!("/coaching_sessions/{session}/transcript_suggestions/scan"),
                    None,
                )
                .await;
            assert_eq!(reply.status, StatusCode::ACCEPTED, "{caller:?}");
            assert_eq!(reply.body["data"]["status"], "pending", "{caller:?}");
            assert!(
                transcription_scanned(app.db(), transcription).await,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TRN-10
#[tokio::test]
async fn non_participants_cannot_read_or_start_a_scan() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (s1, coach, coachee) = (
            fixture.s1,
            fixture.user(Caller::Coach),
            fixture.user(Caller::Coachee),
        );
        // A scannable transcript of S1, so a refusal is all that stops a scan.
        let transcription = seed_unscanned_transcription(app.db(), s1, coach, coachee).await;
        let scan = suggestions(&app, "/scan");

        for caller in NON_PARTICIPANTS {
            for method in [Method::GET, Method::POST] {
                let (status, _) = app.expect(caller, method.clone(), &scan, None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
            }
        }
        assert!(!transcription_scanned(app.db(), transcription).await);
    })
    .await;
}

/// TRN-11
#[tokio::test]
async fn a_coach_accepts_a_coach_only_suggestion() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (s1, sg1, coach) = (fixture.s1, fixture.sg1, fixture.user(Caller::Coach));
        let before = session_content(app.db(), s1).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_sessions/{s1}/transcript_suggestions/{sg1}/accept"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let created: Vec<Id> = reply.ids().into_iter().collect();
        let [action] = created[..] else {
            panic!("one new action id, got {created:?}");
        };
        assert!(!before.contains(&action));
        assert!(session_content(app.db(), s1).await.contains(&action));
        assert_eq!(action_assignees(app.db(), action).await, Some(ids([coach])));
        assert_eq!(
            suggestion_outcome(app.db(), sg1).await,
            (Status::Accepted, Some(action))
        );
    })
    .await;
}

/// TRN-12
#[tokio::test]
async fn a_coachee_cannot_accept_a_coach_only_suggestion() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, sg1) = (app.fixture.s1, app.fixture.sg1);
        let before = session_content(app.db(), s1).await;

        let reply = app
            .send(
                Caller::Coachee,
                Method::POST,
                &format!("/coaching_sessions/{s1}/transcript_suggestions/{sg1}/accept"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(session_content(app.db(), s1).await, before);
        assert_eq!(
            suggestion_outcome(app.db(), sg1).await,
            (Status::Pending, None)
        );
    })
    .await;
}

/// TRN-13
#[tokio::test]
async fn a_coachee_accepts_their_own_suggestion() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (s1, sg2, coach, coachee) = (
            fixture.s1,
            fixture.sg2,
            fixture.user(Caller::Coach),
            fixture.user(Caller::Coachee),
        );
        let path = suggestions(&app, &format!("/{sg2}/accept"));
        let before = session_content(app.db(), s1).await;

        // The matrix's note: an edit that adds the coach is refused.
        let (status, _) = app
            .expect(
                Caller::Coachee,
                Method::POST,
                &path,
                Some(json!({
                    "body": "Send the kumquat plan",
                    "due_by": null,
                    "assignee_ids": [coachee, coach],
                })),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(session_content(app.db(), s1).await, before);
        assert_eq!(
            suggestion_outcome(app.db(), sg2).await,
            (Status::Pending, None)
        );

        let reply = app.send(Caller::Coachee, Method::POST, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let created: Vec<Id> = reply.ids().into_iter().collect();
        let [action] = created[..] else {
            panic!("one new action id, got {created:?}");
        };
        assert!(!before.contains(&action));
        assert_eq!(
            action_assignees(app.db(), action).await,
            Some(ids([coachee]))
        );
        assert_eq!(
            suggestion_outcome(app.db(), sg2).await,
            (Status::Accepted, Some(action))
        );
    })
    .await;
}

/// TRN-14
#[tokio::test]
async fn non_participants_cannot_accept_a_suggestion() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = suggestions(&app, &format!("/{}/accept", app.fixture.sg1));
        let before = suggestion_state(&app).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::POST, &path, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{caller:?}");
        }
        assert_eq!(suggestion_state(&app).await, before);
    })
    .await;
}

/// TRN-15
#[tokio::test]
async fn a_coach_dismisses_and_restores_a_coach_only_suggestion() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let sg1 = app.fixture.sg1;

        for (verb, status) in [("dismiss", Status::Dismissed), ("restore", Status::Pending)] {
            let path = suggestions(&app, &format!("/{sg1}/{verb}"));
            let reply = app.send(Caller::Coach, Method::POST, &path, None).await;

            assert_eq!(reply.status, StatusCode::NO_CONTENT, "{verb}");
            assert_eq!(
                suggestion_outcome(app.db(), sg1).await,
                (status, None),
                "{verb}"
            );
        }
    })
    .await;
}

/// TRN-16
#[tokio::test]
async fn a_coachee_cannot_dismiss_or_restore_a_coach_only_suggestion() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (sg1, sg2) = (app.fixture.sg1, app.fixture.sg2);
        let before = suggestion_state(&app).await;

        for verb in ["dismiss", "restore"] {
            let path = suggestions(&app, &format!("/{sg1}/{verb}"));
            let (status, _) = app.expect(Caller::Coachee, Method::POST, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{verb}");
        }
        assert_eq!(suggestion_state(&app).await, before);

        // The matrix's note: the coachee may dismiss their own SG2.
        let path = suggestions(&app, &format!("/{sg2}/dismiss"));
        let reply = app.send(Caller::Coachee, Method::POST, &path, None).await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        assert_eq!(
            suggestion_outcome(app.db(), sg2).await,
            (Status::Dismissed, None)
        );
    })
    .await;
}

/// TRN-17
#[tokio::test]
async fn non_participants_cannot_dismiss_or_restore_a_suggestion() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let sg1 = app.fixture.sg1;
        let before = suggestion_state(&app).await;

        for caller in NON_PARTICIPANTS {
            for verb in ["dismiss", "restore"] {
                let path = suggestions(&app, &format!("/{sg1}/{verb}"));
                let (status, _) = app.expect(caller, Method::POST, &path, None).await;
                assert_eq!(status, StatusCode::NOT_FOUND, "{caller:?} {verb}");
            }
        }
        assert_eq!(suggestion_state(&app).await, before);
    })
    .await;
}

/// TRN-28
#[tokio::test]
async fn accepting_for_a_coachee_who_left_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s3, sg3) = (app.fixture.s3, app.fixture.sg3);
        let before = session_content(app.db(), s3).await;
        let emails = app.expect_emails(0).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_sessions/{s3}/transcript_suggestions/{sg3}/accept"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(reply.body["details"]["reason"], "assignee_not_a_member");
        assert_eq!(session_content(app.db(), s3).await, before);
        assert_eq!(
            suggestion_outcome(app.db(), sg3).await,
            (Status::Pending, None)
        );
        emails.assert_async().await;
    })
    .await;
}
