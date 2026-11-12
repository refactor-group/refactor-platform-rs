//! Transcripts, suggestions and images (`TRN`).

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{
    action_assignees, session_content, suggestion_outcome, IMG1_BYTES,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::transcript_suggestion::Status;
use domain::Id;

use super::harness::{ids, Baseline};

/// TRN-11
#[tokio::test]
async fn trn_11_a_coach_accepts_a_coach_only_suggestion() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let (s1, sg1, coach) = (world.s1, world.sg1, world.user(Caller::Coach));
        let before = session_content(baseline.db(), s1).await;

        let reply = baseline
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
        assert!(session_content(baseline.db(), s1).await.contains(&action));
        assert_eq!(
            action_assignees(baseline.db(), action).await,
            Some(ids([coach]))
        );
        assert_eq!(
            suggestion_outcome(baseline.db(), sg1).await,
            (Status::Accepted, Some(action))
        );
    })
    .await;
}

/// TRN-12
#[tokio::test]
async fn trn_12_a_coachee_cannot_accept_a_coach_only_suggestion() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, sg1) = (baseline.world.s1, baseline.world.sg1);
        let before = session_content(baseline.db(), s1).await;

        let reply = baseline
            .send(
                Caller::Coachee,
                Method::POST,
                &format!("/coaching_sessions/{s1}/transcript_suggestions/{sg1}/accept"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(session_content(baseline.db(), s1).await, before);
        assert_eq!(
            suggestion_outcome(baseline.db(), sg1).await,
            (Status::Pending, None)
        );
    })
    .await;
}

/// TRN-25
#[tokio::test]
async fn trn_25_participants_read_a_note_image() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/coaching_session_images/{}", baseline.world.img1);

        // The local store cannot presign, so it serves the bytes rather than a redirect.
        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline.send(caller, Method::GET, &path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(&reply.bytes[..], IMG1_BYTES, "{caller:?}");
        }
    })
    .await;
}
