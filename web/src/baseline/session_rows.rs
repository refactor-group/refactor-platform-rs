//! Sessions (`SES`).

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use chrono::{TimeDelta, Utc};
use domain::test_utils::sqlite::access_world::{
    relationship_sessions, seed_content_free_session, session_content, session_ical_sequence,
    session_relationship, session_schedule, session_title, session_viewers,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use super::harness::{ids, Baseline, NON_PARTICIPANTS};

/// A new title for S1 or S3.
const NEW_TITLE: &str = "Retitled kumquat";

/// A session create body for `relationship_id`, five days out.
fn create_body(relationship_id: Id) -> Value {
    let date = (Utc::now() + TimeDelta::days(5))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    json!({ "coaching_relationship_id": relationship_id, "date": date })
}

/// Asserts each caller's session create in `relationship` is refused with `expected`, with
/// no session written, no document created and no email sent.
async fn assert_create_refused(
    callers: &[Caller],
    relationship: impl Fn(&Baseline) -> Id,
    expected: StatusCode,
) {
    let mut baseline = Baseline::new().await;
    let relationship = relationship(&baseline);
    let before = relationship_sessions(baseline.db(), relationship).await;
    let documents = baseline.expect_tiptap("POST", 0).await;
    let emails = baseline.expect_emails(0).await;

    for &caller in callers {
        let reply = baseline
            .send(
                caller,
                Method::POST,
                "/coaching_sessions",
                Some(create_body(relationship)),
            )
            .await;
        assert_eq!(reply.status, expected, "{caller:?}");
        if expected == StatusCode::CONFLICT {
            assert_eq!(
                reply.body["details"]["reason"], "coachee_not_a_member",
                "{caller:?}"
            );
        }
        assert_eq!(
            relationship_sessions(baseline.db(), relationship).await,
            before,
            "{caller:?}"
        );
    }
    documents.assert_async().await;
    emails.assert_async().await;
}

/// Asserts each caller's `method` on S1 at `suffix` with `body` is refused, leaving S1's
/// schedule and title as they were.
async fn assert_s1_write_refused(callers: &[Caller], method: Method, suffix: &str, body: Value) {
    let mut baseline = Baseline::new().await;
    let s1 = baseline.world.s1;
    let path = format!("/coaching_sessions/{s1}{suffix}");
    let schedule = session_schedule(baseline.db(), s1).await;
    let title = session_title(baseline.db(), s1).await;

    for &caller in callers {
        let (status, _) = baseline
            .expect(caller, method.clone(), &path, Some(body.clone()))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        assert_eq!(
            session_schedule(baseline.db(), s1).await,
            schedule,
            "{caller:?}"
        );
        assert_eq!(session_title(baseline.db(), s1).await, title, "{caller:?}");
    }
}

/// A PUT body keeping the session's date and changing its title.
async fn retitle_body(baseline: &Baseline, session: Id) -> Value {
    let (date, _) = session_schedule(baseline.db(), session)
        .await
        .expect("the session is seeded");
    json!({ "date": date, "title": NEW_TITLE })
}

/// Asserts each caller's DELETE of a fresh content-free R1 session is refused, leaving it
/// and its document in place.
async fn assert_delete_refused(callers: &[Caller]) {
    let mut baseline = Baseline::new().await;
    let s4 = seed_content_free_session(baseline.db(), baseline.world.r1).await;
    let schedule = session_schedule(baseline.db(), s4).await;
    let documents_deleted = baseline.expect_tiptap("DELETE", 0).await;

    for &caller in callers {
        let (status, _) = baseline
            .expect(
                caller,
                Method::DELETE,
                &format!("/coaching_sessions/{s4}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        assert!(schedule.is_some(), "{caller:?}");
        assert_eq!(
            session_schedule(baseline.db(), s4).await,
            schedule,
            "{caller:?}"
        );
    }
    documents_deleted.assert_async().await;
}

/// Asserts each caller's resend of `session`'s invite is answered `expected`, with no email
/// sent and the calendar SEQUENCE unchanged.
async fn assert_resend_refused(
    callers: &[Caller],
    session: impl Fn(&Baseline) -> Id,
    expected: StatusCode,
) {
    let mut baseline = Baseline::new().await;
    let session = session(&baseline);
    let sequence = session_ical_sequence(baseline.db(), session).await;
    let emails = baseline.expect_emails(0).await;

    for &caller in callers {
        let reply = baseline
            .send(
                caller,
                Method::POST,
                &format!("/coaching_sessions/{session}/resend_invite"),
                None,
            )
            .await;
        assert_eq!(reply.status, expected, "{caller:?}");
        if expected == StatusCode::CONFLICT {
            assert_eq!(
                reply.body["details"]["reason"], "coachee_not_a_member",
                "{caller:?}"
            );
        }
        assert_eq!(
            session_ical_sequence(baseline.db(), session).await,
            sequence,
            "{caller:?}"
        );
    }
    emails.assert_async().await;
}

/// Asserts `caller`'s own session list is `expected`.
async fn assert_own_sessions(caller: Caller, expected: impl Fn(&Baseline) -> BTreeSet<Id>) {
    let mut baseline = Baseline::new().await;
    let user = baseline.world.user(caller);
    let expected = expected(&baseline);

    assert_eq!(
        baseline
            .expect(
                caller,
                Method::GET,
                &format!("/users/{user}/coaching_sessions"),
                None
            )
            .await,
        (StatusCode::OK, expected),
        "{caller:?}"
    );
}

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

/// SES-03
#[tokio::test]
async fn ses_03_a_removed_coachee_cannot_list_their_relationships_sessions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!(
            "/coaching_sessions?coaching_relationship_id={}&{DATE_RANGE}",
            baseline.world.r3
        );

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// SES-04
#[tokio::test]
async fn ses_04_participants_read_a_session_with_its_organization() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, s1) = (baseline.world.o1, baseline.world.s1);

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline
                .send(
                    caller,
                    Method::GET,
                    &format!("/coaching_sessions/{s1}"),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.ids(), ids([s1]), "{caller:?}");
            assert_eq!(
                reply.body["data"]["organization_id"],
                o1.to_string(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SES-05
#[tokio::test]
async fn ses_05_non_participants_cannot_read_a_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/coaching_sessions/{}", baseline.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// SES-06
#[tokio::test]
async fn ses_06_a_removed_coachee_cannot_read_their_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/coaching_sessions/{}", baseline.world.s3);

        let (status, _) = baseline
            .expect(Caller::Removed, Method::GET, &path, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// SES-08
#[tokio::test]
async fn ses_08_non_participants_cannot_mark_a_session_viewed() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let path = format!("/coaching_sessions/{s1}/view");

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::POST, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                session_viewers(baseline.db(), s1).await,
                BTreeSet::new(),
                "{caller:?}"
            );
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

/// SES-10
#[tokio::test]
async fn ses_10_a_coachee_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Coachee],
        |b| b.world.r1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-11
#[tokio::test]
async fn ses_11_members_outside_the_relationship_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Member, Caller::OtherCoach],
        |b| b.world.r1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-12
#[tokio::test]
async fn ses_12_admins_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Admin, Caller::Super],
        |b| b.world.r1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-13
#[tokio::test]
async fn ses_13_an_outsider_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Outsider],
        |b| b.world.r1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-14
#[tokio::test]
async fn ses_14_a_removed_coachee_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Removed],
        |b| b.world.r3,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-15
#[tokio::test]
async fn ses_15_a_coach_updates_a_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let body = retitle_body(&baseline, s1).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/coaching_sessions/{s1}"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(
            session_title(baseline.db(), s1).await,
            Some(Some(NEW_TITLE.to_string()))
        );
    })
    .await;
}

/// SES-16
#[tokio::test]
async fn ses_16_a_coachee_cannot_update_a_session() {
    within_time_limit(async {
        let body = json!({ "date": "2100-01-01T00:00:00", "title": NEW_TITLE });
        assert_s1_write_refused(&[Caller::Coachee], Method::PUT, "", body).await;
    })
    .await;
}

/// SES-17
#[tokio::test]
async fn ses_17_non_participants_cannot_update_a_session() {
    within_time_limit(async {
        let body = json!({ "date": "2100-01-01T00:00:00", "title": NEW_TITLE });
        assert_s1_write_refused(&NON_PARTICIPANTS, Method::PUT, "", body).await;
    })
    .await;
}

/// SES-18
#[tokio::test]
async fn ses_18_a_coach_retitles_a_session_whose_coachee_left() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s3 = baseline.world.s3;
        let schedule = session_schedule(baseline.db(), s3).await;
        let body = retitle_body(&baseline, s3).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/coaching_sessions/{s3}"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(
            session_title(baseline.db(), s3).await,
            Some(Some(NEW_TITLE.to_string()))
        );
        assert_eq!(session_schedule(baseline.db(), s3).await, schedule);
    })
    .await;
}

/// SES-19
#[tokio::test]
async fn ses_19_either_participant_retitles_a_session() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s1 = baseline.world.s1;
        let path = format!("/coaching_sessions/{s1}/title");

        for caller in [Caller::Coach, Caller::Coachee] {
            let title = format!("{caller:?} kumquat");
            let reply = baseline
                .send(
                    caller,
                    Method::PATCH,
                    &path,
                    Some(json!({ "title": title })),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.ids(), ids([s1]), "{caller:?}");
            assert_eq!(reply.body["data"]["title"], title.as_str(), "{caller:?}");
            assert_eq!(
                session_title(baseline.db(), s1).await,
                Some(Some(title)),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SES-20
#[tokio::test]
async fn ses_20_non_participants_cannot_retitle_a_session() {
    within_time_limit(assert_s1_write_refused(
        &NON_PARTICIPANTS,
        Method::PATCH,
        "/title",
        json!({ "title": NEW_TITLE }),
    ))
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

/// SES-22
#[tokio::test]
async fn ses_22_a_coachee_cannot_delete_a_session() {
    within_time_limit(assert_delete_refused(&[Caller::Coachee])).await;
}

/// SES-23
#[tokio::test]
async fn ses_23_non_participants_cannot_delete_a_session() {
    within_time_limit(assert_delete_refused(&NON_PARTICIPANTS)).await;
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

/// SES-25
#[tokio::test]
async fn ses_25_a_coachee_cannot_resend_a_sessions_invite() {
    within_time_limit(assert_resend_refused(
        &[Caller::Coachee],
        |b| b.world.s1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-26
#[tokio::test]
async fn ses_26_non_participants_cannot_resend_a_sessions_invite() {
    within_time_limit(assert_resend_refused(
        &NON_PARTICIPANTS,
        |b| b.world.s1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-27
#[tokio::test]
async fn ses_27_a_coach_lists_their_own_sessions() {
    within_time_limit(assert_own_sessions(Caller::Coach, |b| {
        let world = &b.world;
        ids([
            world.s1,
            world.s3,
            world.sr1_sessions[0],
            world.sr1_sessions[1],
            world.sr3_sessions[0],
            world.sr3_sessions[1],
        ])
    }))
    .await;
}

/// SES-28
#[tokio::test]
async fn ses_28_a_coachee_lists_their_own_sessions() {
    within_time_limit(assert_own_sessions(Caller::Coachee, |b| {
        let world = &b.world;
        ids([
            world.s1,
            world.s2,
            world.sr1_sessions[0],
            world.sr1_sessions[1],
        ])
    }))
    .await;
}

/// SES-29
#[tokio::test]
async fn ses_29_only_the_user_lists_their_own_sessions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let coachee = baseline.world.user(Caller::Coachee);

        for suffix in ["", "/counts", "/nearest"] {
            let path = format!("/users/{coachee}/coaching_sessions{suffix}");
            for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
                let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {suffix}");
            }
        }
    })
    .await;
}

/// SES-30
#[tokio::test]
async fn ses_30_a_removed_coachee_lists_none_of_their_own_sessions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let removed = baseline.world.user(Caller::Removed);

        assert_eq!(
            baseline
                .expect(
                    Caller::Removed,
                    Method::GET,
                    &format!("/users/{removed}/coaching_sessions"),
                    None
                )
                .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// SES-32
#[tokio::test]
async fn ses_32_a_coachee_reads_the_nearest_session_of_each_relationship() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let coachee = world.user(Caller::Coachee);
        let (r1, r2, r3) = (world.r1, world.r2, world.r3);
        let expected = ids([world.s1, world.s2]);
        let around = Utc::now().format("%Y-%m-%dT%H:%M:%S");

        // R3 is not the coachee's, so it adds nothing.
        for relationships in [format!("{r1},{r2}"), format!("{r1},{r2},{r3}")] {
            let path = format!(
                "/users/{coachee}/coaching_sessions/nearest\
                 ?coaching_relationship_ids={relationships}&around={around}&before=1&after=1"
            );
            assert_eq!(
                baseline
                    .expect(Caller::Coachee, Method::GET, &path, None)
                    .await,
                (StatusCode::OK, expected.clone()),
                "{relationships}"
            );
        }
    })
    .await;
}

/// SES-33
#[tokio::test]
async fn ses_33_scheduling_with_a_coachee_who_left_is_refused() {
    within_time_limit(assert_create_refused(
        &[Caller::Coach],
        |b| b.world.r3,
        StatusCode::CONFLICT,
    ))
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

/// SES-35
#[tokio::test]
async fn ses_35_resending_to_a_coachee_who_left_is_refused() {
    within_time_limit(assert_resend_refused(
        &[Caller::Coach],
        |b| b.world.s3,
        StatusCode::CONFLICT,
    ))
    .await;
}

/// SES-36
#[tokio::test]
async fn ses_36_a_coach_deletes_a_session_whose_coachee_left() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let s5 = seed_content_free_session(baseline.db(), baseline.world.r3).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::DELETE,
                &format!("/coaching_sessions/{s5}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(session_schedule(baseline.db(), s5).await, None);
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
