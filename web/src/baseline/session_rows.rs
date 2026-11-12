//! Sessions (`SES`).

use axum::http::{Method, StatusCode};
use chrono::TimeDelta;
use domain::test_utils::sqlite::access_world::session_schedule;
use domain::test_utils::sqlite::{within_time_limit, Caller};
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
