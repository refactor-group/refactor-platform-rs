//! Transcripts, recordings, suggestions and images (`TRN`).

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{
    action_assignees, image_deleted, seed_content_free_session, seed_unscanned_transcription,
    session_content, session_images, session_recordings, suggestion_outcome, transcription_scanned,
    transcription_segments, IMG1_BYTES,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::transcript_suggestion::Status;
use domain::Id;
use serde_json::json;

use super::harness::{ids, Baseline, Reply, NON_PARTICIPANTS};

/// A valid 2x3 PNG, so the upload sees a real image.
const TINY_PNG: [u8; 73] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x08, 0x02, 0x00, 0x00, 0x00, 0x36, 0x88, 0x49,
    0xd6, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x44, 0x0c, 0x28, 0x14, 0x00, 0x44, 0xd0, 0x05, 0xfb, 0xa4, 0xcf, 0xde, 0x80, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const BOUNDARY: &str = "access-baseline-boundary";

/// The coach's and the coachee's transcript lines, as seeded.
const COACH_LINE: &str = "What will you do next, kumquat?";
const COACHEE_LINE: &str = "I will send the kumquat plan.";

/// S1's suggestions path, plus `suffix`.
fn suggestions(baseline: &Baseline, suffix: &str) -> String {
    format!(
        "/coaching_sessions/{}/transcript_suggestions{suffix}",
        baseline.world.s1
    )
}

/// The suggestion statuses and S1's content, to check a refusal changed nothing.
async fn suggestion_state(baseline: &Baseline) -> (Vec<(Status, Option<Id>)>, BTreeSet<Id>) {
    let world = &baseline.world;
    let mut outcomes = Vec::new();
    for suggestion in [world.sg1, world.sg2, world.sg3] {
        outcomes.push(suggestion_outcome(baseline.db(), suggestion).await);
    }
    (outcomes, session_content(baseline.db(), world.s1).await)
}

/// TRN-01
#[tokio::test]
async fn trn_01_participants_read_the_latest_transcription() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/coaching_sessions/{}/transcriptions", baseline.world.s1);
        let expected = ids([baseline.world.tr1]);

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

/// TRN-02, TRN-04 and TRN-06: every non-participant is refused each transcription read.
async fn assert_transcription_reads_refused(suffix: impl Fn(&Baseline) -> String) {
    let mut baseline = Baseline::new().await;
    let path = format!(
        "/coaching_sessions/{}/transcriptions{}",
        baseline.world.s1,
        suffix(&baseline)
    );

    for caller in NON_PARTICIPANTS {
        let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
    }
}

/// TRN-02
#[tokio::test]
async fn trn_02_non_participants_cannot_read_the_latest_transcription() {
    within_time_limit(assert_transcription_reads_refused(|_| String::new())).await;
}

/// TRN-03
#[tokio::test]
async fn trn_03_participants_download_only_the_coachs_lines() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let path = format!(
            "/coaching_sessions/{}/transcriptions/{}?speaker=coach",
            world.s1, world.tr1
        );

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline
                .send_with(
                    caller,
                    Method::GET,
                    &path,
                    &[("accept", "text/plain")],
                    Body::empty(),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            let text = String::from_utf8_lossy(&reply.bytes);
            assert!(text.contains(COACH_LINE), "{caller:?}: {text}");
            assert!(!text.contains(COACHEE_LINE), "{caller:?}: {text}");
        }
    })
    .await;
}

/// TRN-04
#[tokio::test]
async fn trn_04_non_participants_cannot_read_a_transcription() {
    within_time_limit(async {
        assert_transcription_reads_refused(|b| format!("/{}", b.world.tr1)).await;

        // The row's note: TR1 under a session it is not in is not found.
        let mut baseline = Baseline::new().await;
        let path = format!(
            "/coaching_sessions/{}/transcriptions/{}",
            baseline.world.s3, baseline.world.tr1
        );
        let (status, _) = baseline
            .expect(Caller::Coach, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}

/// TRN-05
#[tokio::test]
async fn trn_05_participants_read_the_transcriptions_segments() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s1, tr1) = (baseline.world.s1, baseline.world.tr1);
        let path = format!("/coaching_sessions/{s1}/transcriptions/{tr1}/transcription_segments");
        let expected = transcription_segments(baseline.db(), tr1).await;
        assert_eq!(expected.len(), 2);

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

/// TRN-06
#[tokio::test]
async fn trn_06_non_participants_cannot_read_the_transcriptions_segments() {
    within_time_limit(assert_transcription_reads_refused(|b| {
        format!("/{}/transcription_segments", b.world.tr1)
    }))
    .await;
}

/// TRN-07
#[tokio::test]
async fn trn_07_participants_list_the_sessions_suggestions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = suggestions(&baseline, "");
        let expected = ids([baseline.world.sg1, baseline.world.sg2]);

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

/// TRN-08
#[tokio::test]
async fn trn_08_non_participants_cannot_list_the_sessions_suggestions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = suggestions(&baseline, "");

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// TRN-09
#[tokio::test]
async fn trn_09_participants_read_and_start_a_scan() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut baseline = Baseline::new().await;
            let world = &baseline.world;
            let (r1, coach, coachee) = (
                world.r1,
                world.user(Caller::Coach),
                world.user(Caller::Coachee),
            );
            let scan = suggestions(&baseline, "/scan");

            let reply = baseline.send(caller, Method::GET, &scan, None).await;
            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["data"]["status"], "completed", "{caller:?}");

            // The row's note: S1's scan already completed, so it is not repeated.
            let reply = baseline.send(caller, Method::POST, &scan, None).await;
            assert_eq!(reply.status, StatusCode::CONFLICT, "{caller:?}");

            let session = seed_content_free_session(baseline.db(), r1).await;
            let transcription =
                seed_unscanned_transcription(baseline.db(), session, coach, coachee).await;
            let reply = baseline
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
                transcription_scanned(baseline.db(), transcription).await,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TRN-10
#[tokio::test]
async fn trn_10_non_participants_cannot_read_or_start_a_scan() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let (s1, coach, coachee) = (
            world.s1,
            world.user(Caller::Coach),
            world.user(Caller::Coachee),
        );
        // A scannable transcript of S1, so a refusal is all that stops a scan.
        let transcription = seed_unscanned_transcription(baseline.db(), s1, coach, coachee).await;
        let scan = suggestions(&baseline, "/scan");

        for caller in NON_PARTICIPANTS {
            for method in [Method::GET, Method::POST] {
                let (status, _) = baseline.expect(caller, method.clone(), &scan, None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
            }
        }
        assert!(!transcription_scanned(baseline.db(), transcription).await);
    })
    .await;
}

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

/// TRN-13
#[tokio::test]
async fn trn_13_a_coachee_accepts_their_own_suggestion() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let (s1, sg2, coach, coachee) = (
            world.s1,
            world.sg2,
            world.user(Caller::Coach),
            world.user(Caller::Coachee),
        );
        let path = suggestions(&baseline, &format!("/{sg2}/accept"));
        let before = session_content(baseline.db(), s1).await;

        // The row's note: an edit that adds the coach is refused.
        let (status, _) = baseline
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
        assert_eq!(session_content(baseline.db(), s1).await, before);
        assert_eq!(
            suggestion_outcome(baseline.db(), sg2).await,
            (Status::Pending, None)
        );

        let reply = baseline
            .send(Caller::Coachee, Method::POST, &path, None)
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let created: Vec<Id> = reply.ids().into_iter().collect();
        let [action] = created[..] else {
            panic!("one new action id, got {created:?}");
        };
        assert!(!before.contains(&action));
        assert_eq!(
            action_assignees(baseline.db(), action).await,
            Some(ids([coachee]))
        );
        assert_eq!(
            suggestion_outcome(baseline.db(), sg2).await,
            (Status::Accepted, Some(action))
        );
    })
    .await;
}

/// TRN-14
#[tokio::test]
async fn trn_14_non_participants_cannot_accept_a_suggestion() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = suggestions(&baseline, &format!("/{}/accept", baseline.world.sg1));
        let before = suggestion_state(&baseline).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::POST, &path, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{caller:?}");
        }
        assert_eq!(suggestion_state(&baseline).await, before);
    })
    .await;
}

/// TRN-15
#[tokio::test]
async fn trn_15_a_coach_dismisses_and_restores_a_coach_only_suggestion() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let sg1 = baseline.world.sg1;

        for (verb, status) in [("dismiss", Status::Dismissed), ("restore", Status::Pending)] {
            let path = suggestions(&baseline, &format!("/{sg1}/{verb}"));
            let reply = baseline
                .send(Caller::Coach, Method::POST, &path, None)
                .await;

            assert_eq!(reply.status, StatusCode::NO_CONTENT, "{verb}");
            assert_eq!(
                suggestion_outcome(baseline.db(), sg1).await,
                (status, None),
                "{verb}"
            );
        }
    })
    .await;
}

/// TRN-16
#[tokio::test]
async fn trn_16_a_coachee_cannot_dismiss_or_restore_a_coach_only_suggestion() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (sg1, sg2) = (baseline.world.sg1, baseline.world.sg2);
        let before = suggestion_state(&baseline).await;

        for verb in ["dismiss", "restore"] {
            let path = suggestions(&baseline, &format!("/{sg1}/{verb}"));
            let (status, _) = baseline
                .expect(Caller::Coachee, Method::POST, &path, None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{verb}");
        }
        assert_eq!(suggestion_state(&baseline).await, before);

        // The row's note: the coachee may dismiss their own SG2.
        let path = suggestions(&baseline, &format!("/{sg2}/dismiss"));
        let reply = baseline
            .send(Caller::Coachee, Method::POST, &path, None)
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        assert_eq!(
            suggestion_outcome(baseline.db(), sg2).await,
            (Status::Dismissed, None)
        );
    })
    .await;
}

/// TRN-17
#[tokio::test]
async fn trn_17_non_participants_cannot_dismiss_or_restore_a_suggestion() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let sg1 = baseline.world.sg1;
        let before = suggestion_state(&baseline).await;

        for caller in NON_PARTICIPANTS {
            for verb in ["dismiss", "restore"] {
                let path = suggestions(&baseline, &format!("/{sg1}/{verb}"));
                let (status, _) = baseline.expect(caller, Method::POST, &path, None).await;
                assert_eq!(status, StatusCode::NOT_FOUND, "{caller:?} {verb}");
            }
        }
        assert_eq!(suggestion_state(&baseline).await, before);
    })
    .await;
}

/// TRN-18
#[tokio::test]
async fn trn_18_participants_read_the_latest_recording() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let path = format!("/coaching_sessions/{s1}/meeting_recording");
        let expected = session_recordings(baseline.db(), s1).await;
        assert_eq!(expected.len(), 1);

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

/// Asserts each caller is refused every recording route on `session`, with no recording
/// written. The POST is refused before any bot is sent.
async fn assert_recording_refused(callers: &[Caller], session: impl Fn(&Baseline) -> Id) {
    let mut baseline = Baseline::new().await;
    let session = session(&baseline);
    let path = format!("/coaching_sessions/{session}/meeting_recording");
    let before = session_recordings(baseline.db(), session).await;

    for &caller in callers {
        for (method, body) in [
            (Method::GET, None),
            (
                Method::POST,
                Some(json!({ "meeting_url": "https://meet.google.com/abc-defg-hij" })),
            ),
            (Method::DELETE, None),
        ] {
            let (status, _) = baseline.expect(caller, method.clone(), &path, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
        }
    }
    assert_eq!(session_recordings(baseline.db(), session).await, before);
}

/// TRN-21
#[tokio::test]
async fn trn_21_non_participants_cannot_reach_the_sessions_recording() {
    within_time_limit(assert_recording_refused(&NON_PARTICIPANTS, |b| b.world.s1)).await;
}

/// TRN-22
#[tokio::test]
async fn trn_22_a_removed_coachee_cannot_reach_their_sessions_recording() {
    within_time_limit(assert_recording_refused(&[Caller::Removed], |b| b.world.s3)).await;
}

/// A `multipart/form-data` body carrying TINY_PNG as the `file` part.
fn image_upload() -> Body {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; \
         filename=\"paste.png\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&TINY_PNG);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    Body::from(body)
}

/// Uploads TINY_PNG to `session` as `caller`.
async fn upload(baseline: &mut Baseline, caller: Caller, session: Id) -> Reply {
    let content_type = format!("multipart/form-data; boundary={BOUNDARY}");
    baseline
        .send_with(
            caller,
            Method::POST,
            &format!("/coaching_sessions/{session}/images"),
            &[("content-type", content_type.as_str())],
            image_upload(),
        )
        .await
}

/// TRN-23
#[tokio::test]
async fn trn_23_participants_upload_a_note_image() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = session_images(baseline.db(), s1).await;

            let reply = upload(&mut baseline, caller, s1).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created: Vec<Id> = reply.ids().into_iter().collect();
            let [image] = created[..] else {
                panic!("{caller:?}: one new image id, got {created:?}");
            };
            assert!(!before.contains(&image), "{caller:?}");
            assert!(
                session_images(baseline.db(), s1).await.contains(&image),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TRN-24
#[tokio::test]
async fn trn_24_non_participants_cannot_upload_a_note_image() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let before = session_images(baseline.db(), s1).await;

        for caller in NON_PARTICIPANTS {
            let reply = upload(&mut baseline, caller, s1).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        assert_eq!(session_images(baseline.db(), s1).await, before);
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

/// TRN-26
#[tokio::test]
async fn trn_26_either_participant_deletes_and_restores_a_note_image() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let img1 = baseline.world.img1;

        // IMG1 was uploaded by COACH, so COACHEE acts on an image they did not upload.
        for caller in [Caller::Coach, Caller::Coachee] {
            for (method, suffix, deleted) in [
                (Method::DELETE, "", true),
                (Method::POST, "/restore", false),
            ] {
                let path = format!("/coaching_session_images/{img1}{suffix}");
                assert_eq!(
                    baseline.expect(caller, method.clone(), &path, None).await,
                    (StatusCode::OK, ids([img1])),
                    "{caller:?} {method}"
                );
                assert_eq!(
                    image_deleted(baseline.db(), img1).await,
                    Some(deleted),
                    "{caller:?} {method}"
                );
            }
        }
    })
    .await;
}

/// TRN-27
#[tokio::test]
async fn trn_27_non_participants_cannot_read_or_delete_a_note_image() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let img1 = baseline.world.img1;
        let path = format!("/coaching_session_images/{img1}");

        for caller in NON_PARTICIPANTS {
            for method in [Method::GET, Method::DELETE] {
                let (status, _) = baseline.expect(caller, method.clone(), &path, None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
            }
        }
        assert_eq!(image_deleted(baseline.db(), img1).await, Some(false));

        // The row's note: an unknown image is not found.
        let (status, _) = baseline
            .expect(
                Caller::Coach,
                Method::GET,
                &format!("/coaching_session_images/{}", Id::new_v4()),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}

/// TRN-28
#[tokio::test]
async fn trn_28_accepting_for_a_coachee_who_left_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (s3, sg3) = (baseline.world.s3, baseline.world.sg3);
        let before = session_content(baseline.db(), s3).await;
        let emails = baseline.expect_emails(0).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_sessions/{s3}/transcript_suggestions/{sg3}/accept"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(reply.body["details"]["reason"], "assignee_not_a_member");
        assert_eq!(session_content(baseline.db(), s3).await, before);
        assert_eq!(
            suggestion_outcome(baseline.db(), sg3).await,
            (Status::Pending, None)
        );
        emails.assert_async().await;
    })
    .await;
}
