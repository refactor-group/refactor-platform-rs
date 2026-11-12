//! Series (`SER`).

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use chrono::{TimeDelta, Utc};
use domain::test_utils::sqlite::access_world::{
    relationship_series, relationship_sessions, remove_roles, series_state,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use super::harness::{ids, ids_in, Baseline, NON_PARTICIPANTS};

/// Every non-participant of R1, plus the coachee.
const COACHEE_AND_NON_PARTICIPANTS: [Caller; 6] = [
    Caller::Coachee,
    Caller::Member,
    Caller::OtherCoach,
    Caller::Admin,
    Caller::Super,
    Caller::Outsider,
];

/// `days` from now, as the routes take a start.
fn start_in(days: i64) -> String {
    (Utc::now() + TimeDelta::days(days))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

/// A two-session weekly series create body for `relationship_id`.
fn create_body(relationship_id: Id) -> Value {
    json!({
        "coaching_relationship_id": relationship_id,
        "start_at": start_in(10),
        "recurrence": { "frequency": "weekly", "interval": 1, "count": 2 },
        "duration_minutes": 60,
    })
}

/// A PUT body moving the series to a new weekly rule.
fn reschedule_body() -> Value {
    json!({
        "start_at": start_in(12),
        "recurrence": { "frequency": "weekly", "interval": 1, "count": 2 },
        "duration_minutes": 45,
    })
}

/// A reschedule-following body moving `session` and those after it.
fn reschedule_following_body(session: Id) -> Value {
    json!({ "coaching_session_id": session, "start_at": start_in(12) })
}

/// Asserts each caller's series create in `relationship` is answered `expected`, with no
/// series or session written, no document created and no email sent.
async fn assert_create_refused(
    baseline: &mut Baseline,
    callers: &[Caller],
    relationship: Id,
    expected: StatusCode,
) {
    let series = relationship_series(baseline.db(), relationship).await;
    let sessions = relationship_sessions(baseline.db(), relationship).await;
    let documents = baseline.expect_tiptap("POST", 0).await;
    let emails = baseline.expect_emails(0).await;

    for &caller in callers {
        let reply = baseline
            .send(
                caller,
                Method::POST,
                "/coaching_session_series",
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
            relationship_series(baseline.db(), relationship).await,
            series,
            "{caller:?}"
        );
        assert_eq!(
            relationship_sessions(baseline.db(), relationship).await,
            sessions,
            "{caller:?}"
        );
    }
    documents.assert_async().await;
    emails.assert_async().await;
}

/// Asserts each caller's `method` on a series at `suffix` is answered `expected`, leaving
/// the series, its sessions and its documents as they were and sending no email.
async fn assert_series_write_refused(
    callers: &[Caller],
    series: impl Fn(&Baseline) -> (Id, Id),
    method: Method,
    suffix: &str,
    body: impl Fn(Id) -> Option<Value>,
    expected: StatusCode,
) {
    let mut baseline = Baseline::new().await;
    let (series, first_session) = series(&baseline);
    let path = format!("/coaching_session_series/{series}{suffix}");
    let before = series_state(baseline.db(), series).await;
    let documents_created = baseline.expect_tiptap("POST", 0).await;
    let documents_deleted = baseline.expect_tiptap("DELETE", 0).await;
    let emails = baseline.expect_emails(0).await;

    for &caller in callers {
        let reply = baseline
            .send(caller, method.clone(), &path, body(first_session))
            .await;
        assert_eq!(reply.status, expected, "{caller:?}");
        if expected == StatusCode::CONFLICT {
            assert_eq!(
                reply.body["details"]["reason"], "coachee_not_a_member",
                "{caller:?}"
            );
        }
        assert!(before.is_some(), "{caller:?}");
        assert_eq!(
            series_state(baseline.db(), series).await,
            before,
            "{caller:?}"
        );
    }
    documents_created.assert_async().await;
    documents_deleted.assert_async().await;
    emails.assert_async().await;
}

/// SR1 and its first session.
fn sr1(baseline: &Baseline) -> (Id, Id) {
    (baseline.world.sr1, baseline.world.sr1_sessions[0])
}

/// SR3 and its first session.
fn sr3(baseline: &Baseline) -> (Id, Id) {
    (baseline.world.sr3, baseline.world.sr3_sessions[0])
}

/// SER-01
#[tokio::test]
async fn ser_01_a_coach_creates_a_series_with_its_sessions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let r1 = baseline.world.r1;
        let series_before = relationship_series(baseline.db(), r1).await;
        let sessions_before = relationship_sessions(baseline.db(), r1).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::POST,
                "/coaching_session_series",
                Some(create_body(r1)),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let series = ids_in(&reply.body["data"]["series"]);
        let sessions = ids_in(&reply.body["data"]["sessions"]);
        assert_eq!(series.len(), 1);
        assert_eq!(sessions.len(), 2);
        let series_after = relationship_series(baseline.db(), r1).await;
        assert_eq!(
            series_after
                .difference(&series_before)
                .copied()
                .collect::<BTreeSet<_>>(),
            series
        );
        let sessions_after = relationship_sessions(baseline.db(), r1).await;
        assert_eq!(
            sessions_after
                .difference(&sessions_before)
                .copied()
                .collect::<BTreeSet<_>>(),
            sessions
        );
    })
    .await;
}

/// SER-02
#[tokio::test]
async fn ser_02_a_coachee_cannot_create_a_series() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let r1 = baseline.world.r1;
        assert_create_refused(&mut baseline, &[Caller::Coachee], r1, StatusCode::FORBIDDEN).await;
    })
    .await;
}

/// SER-03
#[tokio::test]
async fn ser_03_non_participants_cannot_create_a_series() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let r1 = baseline.world.r1;
        assert_create_refused(&mut baseline, &NON_PARTICIPANTS, r1, StatusCode::FORBIDDEN).await;
    })
    .await;
}

/// SER-04
#[tokio::test]
async fn ser_04_participants_list_the_relationships_series() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (r1, sr1) = (baseline.world.r1, baseline.world.sr1);
        let path = format!("/coaching_session_series?coaching_relationship_id={r1}");

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                baseline.expect(caller, Method::GET, &path, None).await,
                (StatusCode::OK, ids([sr1])),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SER-05
#[tokio::test]
async fn ser_05_non_participants_cannot_list_the_relationships_series() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!(
            "/coaching_session_series?coaching_relationship_id={}",
            baseline.world.r1
        );

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// SER-06
#[tokio::test]
async fn ser_06_participants_read_a_series_with_its_sessions() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let sr1 = world.sr1;
        let sessions = ids(world.sr1_sessions);
        let path = format!("/coaching_session_series/{sr1}");

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = baseline.send(caller, Method::GET, &path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(
                ids_in(&reply.body["data"]["series"]),
                ids([sr1]),
                "{caller:?}"
            );
            assert_eq!(
                ids_in(&reply.body["data"]["sessions"]),
                sessions,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SER-07
#[tokio::test]
async fn ser_07_non_participants_cannot_read_a_series() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!("/coaching_session_series/{}", baseline.world.sr1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = baseline.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// SER-09
#[tokio::test]
async fn ser_09_only_the_coach_reschedules_a_series() {
    within_time_limit(assert_series_write_refused(
        &COACHEE_AND_NON_PARTICIPANTS,
        sr1,
        Method::PUT,
        "",
        |_| Some(reschedule_body()),
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SER-11
#[tokio::test]
async fn ser_11_only_the_coach_reschedules_following_sessions() {
    within_time_limit(assert_series_write_refused(
        &COACHEE_AND_NON_PARTICIPANTS,
        sr1,
        Method::POST,
        "/reschedule",
        |session| Some(reschedule_following_body(session)),
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SER-13
#[tokio::test]
async fn ser_13_only_the_coach_deletes_a_series() {
    within_time_limit(assert_series_write_refused(
        &COACHEE_AND_NON_PARTICIPANTS,
        sr1,
        Method::DELETE,
        "",
        |_| None,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SER-14
#[tokio::test]
async fn ser_14_a_coach_resends_a_series_invite() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let sr1 = baseline.world.sr1;
        let (_, before, _) = series_state(baseline.db(), sr1)
            .await
            .expect("SR1 is seeded");
        let invites = baseline.expect_emails(2).await;

        let reply = baseline
            .send(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_session_series/{sr1}/resend_invite"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        let (_, after, _) = series_state(baseline.db(), sr1).await.expect("SR1 is kept");
        assert_eq!(after, before + 1);
        invites.assert_async().await;
    })
    .await;
}

/// SER-15
#[tokio::test]
async fn ser_15_only_the_coach_resends_a_series_invite() {
    within_time_limit(assert_series_write_refused(
        &COACHEE_AND_NON_PARTICIPANTS,
        sr1,
        Method::POST,
        "/resend_invite",
        |_| None,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SER-16
#[tokio::test]
async fn ser_16_a_coach_removed_from_the_org_cannot_create_a_series() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (r1, o1) = (baseline.world.r1, baseline.world.o1);
        remove_roles(baseline.db(), baseline.world.user(Caller::Coach), o1).await;
        assert_create_refused(&mut baseline, &[Caller::Coach], r1, StatusCode::FORBIDDEN).await;
    })
    .await;
}

/// SER-17
#[tokio::test]
async fn ser_17_a_series_with_a_coachee_who_left_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let r3 = baseline.world.r3;
        assert_create_refused(&mut baseline, &[Caller::Coach], r3, StatusCode::CONFLICT).await;
    })
    .await;
}

/// SER-18
#[tokio::test]
async fn ser_18_rescheduling_a_series_whose_coachee_left_is_refused() {
    within_time_limit(assert_series_write_refused(
        &[Caller::Coach],
        sr3,
        Method::PUT,
        "",
        |_| Some(reschedule_body()),
        StatusCode::CONFLICT,
    ))
    .await;
}

/// SER-20
#[tokio::test]
async fn ser_20_resending_a_series_to_a_coachee_who_left_is_refused() {
    within_time_limit(assert_series_write_refused(
        &[Caller::Coach],
        sr3,
        Method::POST,
        "/resend_invite",
        |_| None,
        StatusCode::CONFLICT,
    ))
    .await;
}
