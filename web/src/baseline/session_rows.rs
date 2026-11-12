//! Sessions (`SES`).

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use chrono::{TimeDelta, Utc};
use domain::test_utils::sqlite::access_world::{
    seed_content_free_session, session_content, session_ical_sequence, session_relationship,
    session_schedule,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::json;

use super::harness::{ids, Baseline, NON_PARTICIPANTS};

/// Wide enough to hold every seeded session.
const DATE_RANGE: &str = "from_date=2000-01-01&to_date=2100-01-01";

/// SES-01
#[tokio::test]
async fn ses_01_participants_list_the_relationships_sessions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!(
            "/coaching_sessions?coaching_relationship_id={}&{DATE_RANGE}",
            world.r1
        );
        let expected = ids([world.s1, world.sr1_sessions[0], world.sr1_sessions[1]]);

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

/// SES-02
#[tokio::test]
async fn ses_02_non_participants_cannot_list_the_relationships_sessions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!(
            "/coaching_sessions?coaching_relationship_id={}&{DATE_RANGE}",
            baseline.world.r1
        );

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// SES-09
#[tokio::test]
async fn ses_09_a_coach_creates_a_session_with_its_notes_document() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let r1 = baseline.world.r1;
        let date = (Utc::now() + TimeDelta::days(5))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let documents_created = baseline.expect_tiptap("POST", 1).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::POST,
                "/coaching_sessions",
                Some(json!({ "coaching_relationship_id": r1, "date": date })),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let created: Vec<Id> = reply.ids().into_iter().collect();
        let [session] = created[..] else {
            panic!("one new session id, got {created:?}");
        };
        assert_eq!(session_relationship(baseline.db(), session).await, Some(r1));
        documents_created.assert_async().await;
    })
    .await;
}

/// SES-21
#[tokio::test]
async fn ses_21_a_coach_deletes_a_content_free_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s4 = seed_content_free_session(baseline.db(), baseline.world.r1).await;
        let document_deleted = baseline.expect_tiptap("DELETE", 1).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::DELETE,
                &format!("/coaching_sessions/{s4}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(session_schedule(baseline.db(), s4).await, None);
        document_deleted.assert_async().await;
    })
    .await;
}

/// SES-24
#[tokio::test]
async fn ses_24_a_coach_resends_a_sessions_invite() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let before = session_ical_sequence(baseline.db(), s1)
            .await
            .expect("S1 is seeded");
        let invites = baseline.expect_emails(2).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_sessions/{s1}/resend_invite"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        assert_eq!(
            session_ical_sequence(baseline.db(), s1).await,
            Some(before + 1)
        );
        invites.assert_async().await;
    })
    .await;
}

/// SES-34
#[tokio::test]
async fn ses_34_moving_a_session_whose_coachee_left_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s3 = baseline.world.s3;
        let before = session_schedule(baseline.db(), s3).await;
        let (date, _) = before.expect("S3 is seeded");
        let moved = (date + TimeDelta::days(1))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();

        let reply = baseline
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/coaching_sessions/{s3}"),
                Some(json!({ "date": moved })),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(reply.body["details"]["reason"], "coachee_not_a_member");
        assert_eq!(session_schedule(baseline.db(), s3).await, before);
    })
    .await;
}

/// SES-37
#[tokio::test]
async fn ses_37_deleting_a_session_with_content_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let s1 = world.s1;
        let content: BTreeSet<Id> = ids([world.n1, world.ag1, world.a1, world.a2, world.a3]);
        let schedule = session_schedule(baseline.db(), s1).await;
        let documents_deleted = baseline.expect_tiptap("DELETE", 0).await;
        let emails = baseline.expect_emails(0).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::DELETE,
                &format!("/coaching_sessions/{s1}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(reply.body["details"]["reason"], "sessions_have_content");
        assert_eq!(reply.body["details"]["coaching_session_id"], s1.to_string());
        assert!(schedule.is_some());
        assert_eq!(session_schedule(baseline.db(), s1).await, schedule);
        assert_eq!(session_content(baseline.db(), s1).await, content);
        documents_deleted.assert_async().await;
        emails.assert_async().await;
    })
    .await;
}
