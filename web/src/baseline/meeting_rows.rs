//! Meeting links and provider connections (`MEET`).

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use chrono::{TimeDelta, Utc};
use domain::test_utils::sqlite::access_world::{
    connected_users, relationship_sessions, seed_google_connection, session_meeting_url,
    session_schedule, set_google_meeting_url,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use super::harness::{ids, Baseline};

/// R1's Google Meet link, which a new R1 session reuses.
const R1_LINK: &str = "https://meet.google.com/kum-quat-r1";

/// A session create body for `relationship`, five days out, asking for a Google link.
fn create_with_provider(relationship: Id) -> Value {
    let date = (Utc::now() + TimeDelta::days(5))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    json!({ "coaching_relationship_id": relationship, "date": date, "provider": "google" })
}

/// MEET-02
#[tokio::test]
async fn meet_02_only_the_coachs_create_attaches_a_meeting_link() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (r1, r3, s1) = (baseline.world.r1, baseline.world.r3, baseline.world.s1);
        set_google_meeting_url(baseline.db(), s1, R1_LINK).await;

        // SES-10 to SES-14: refused, with no session, document or link written.
        let documents = baseline.expect_tiptap("POST", 0).await;
        for (caller, relationship) in [
            (Caller::Coachee, r1),
            (Caller::Member, r1),
            (Caller::OtherCoach, r1),
            (Caller::Admin, r1),
            (Caller::Super, r1),
            (Caller::Outsider, r1),
            (Caller::Removed, r3),
        ] {
            let before = relationship_sessions(baseline.db(), relationship).await;
            let (status, _) = baseline
                .expect(
                    caller,
                    Method::POST,
                    "/coaching_sessions",
                    Some(create_with_provider(relationship)),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                relationship_sessions(baseline.db(), relationship).await,
                before,
                "{caller:?}"
            );
        }
        documents.assert_async().await;

        // SES-09: the coach's new session reuses R1's link.
        let reply = baseline
            .send(
                Caller::Coach,
                Method::POST,
                "/coaching_sessions",
                Some(create_with_provider(r1)),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let created: Vec<Id> = reply.ids().into_iter().collect();
        let [session] = created[..] else {
            panic!("one new session id, got {created:?}");
        };
        assert_eq!(
            session_meeting_url(baseline.db(), session).await,
            Some(Some(R1_LINK.to_string()))
        );
    })
    .await;
}

/// MEET-03
#[tokio::test]
async fn meet_03_a_coach_sets_the_sessions_meeting_link() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let (date, _) = session_schedule(baseline.db(), s1)
            .await
            .expect("S1 is seeded");

        let reply = baseline
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/coaching_sessions/{s1}"),
                Some(json!({ "date": date, "meeting_url": R1_LINK, "provider": "google" })),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(
            session_meeting_url(baseline.db(), s1).await,
            Some(Some(R1_LINK.to_string()))
        );
    })
    .await;
}

/// The connected account emails `caller` lists.
async fn listed_emails(baseline: &mut Baseline, caller: Caller) -> BTreeSet<String> {
    let reply = baseline
        .send(caller, Method::GET, "/oauth/connections", None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
    reply.body["data"]
        .as_array()
        .expect("a list of connections")
        .iter()
        .filter_map(|connection| connection["email"].as_str().map(str::to_string))
        .collect()
}

/// MEET-05
#[tokio::test]
async fn meet_05_every_caller_reaches_only_their_own_connections() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (coach, other_coach) = (
            baseline.world.user(Caller::Coach),
            baseline.world.user(Caller::OtherCoach),
        );
        seed_google_connection(baseline.db(), coach, "coach@google.test").await;
        seed_google_connection(baseline.db(), other_coach, "other@google.test").await;

        for caller in Caller::ALL {
            let own: BTreeSet<String> = match caller {
                Caller::Coach => ["coach@google.test".to_string()].into(),
                Caller::OtherCoach => ["other@google.test".to_string()].into(),
                _ => BTreeSet::new(),
            };
            assert_eq!(
                listed_emails(&mut baseline, caller).await,
                own,
                "{caller:?}"
            );

            let reply = baseline
                .send(caller, Method::GET, "/oauth/connections/google", None)
                .await;
            let expected = match own.first() {
                Some(email) => (StatusCode::OK, json!(email)),
                None => (StatusCode::NOT_FOUND, Value::Null),
            };
            assert_eq!(
                (reply.status, reply.body["data"]["email"].clone()),
                expected,
                "{caller:?}"
            );
        }

        // A caller with no connection deletes nothing of anyone else's.
        let reply = baseline
            .send(
                Caller::Member,
                Method::DELETE,
                "/oauth/connections/google",
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert_eq!(
            connected_users(baseline.db()).await,
            ids([coach, other_coach])
        );

        let reply = baseline
            .send(
                Caller::Coach,
                Method::DELETE,
                "/oauth/connections/google",
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(connected_users(baseline.db()).await, ids([other_coach]));
    })
    .await;
}
