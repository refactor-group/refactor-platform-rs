//! Emails (`MAIL`) whose recipients the router decides, read from the Resend stand-in.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::remove_roles;
use domain::test_utils::sqlite::{within_time_limit, Caller};

use super::harness::Baseline;

/// COACH resends the invite at `path`; exactly COACH and COACHEE are mailed.
async fn assert_resend_mails_both(path: impl Fn(&Baseline) -> String) {
    let mut baseline = Baseline::new().await;
    let path = path(&baseline);
    baseline.take_emailed();

    let reply = baseline
        .send(Caller::Coach, Method::POST, &path, None)
        .await;

    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let mut expected = vec![Caller::Coach.email(), Caller::Coachee.email()];
    expected.sort();
    assert_eq!(baseline.take_emailed(), expected);
}

/// COACH, holding no O1 role, resends the invite at `path`; refused, and nobody is mailed.
async fn assert_removed_coach_resend_refused(path: impl Fn(&Baseline) -> String) {
    let mut baseline = Baseline::new().await;
    let path = path(&baseline);
    let (coach, o1) = (baseline.world.user(Caller::Coach), baseline.world.o1);
    remove_roles(baseline.db(), coach, o1).await;
    baseline.take_emailed();

    let reply = baseline
        .send(Caller::Coach, Method::POST, &path, None)
        .await;

    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(baseline.take_emailed(), Vec::<String>::new());
}

/// S1's invite resend path.
fn session_resend(baseline: &Baseline) -> String {
    format!("/coaching_sessions/{}/resend_invite", baseline.world.s1)
}

/// SR1's invite resend path.
fn series_resend(baseline: &Baseline) -> String {
    format!(
        "/coaching_session_series/{}/resend_invite",
        baseline.world.sr1
    )
}

/// MAIL-04
#[tokio::test]
async fn mail_04_a_session_resend_mails_both_members() {
    within_time_limit(assert_resend_mails_both(session_resend)).await;
}

/// MAIL-04
#[tokio::test]
async fn mail_04_a_coach_who_left_cannot_resend_a_sessions_invite() {
    within_time_limit(assert_removed_coach_resend_refused(session_resend)).await;
}

/// MAIL-09
#[tokio::test]
async fn mail_09_a_series_resend_mails_both_members() {
    within_time_limit(assert_resend_mails_both(series_resend)).await;
}

/// MAIL-09
#[tokio::test]
async fn mail_09_a_coach_who_left_cannot_resend_a_series_invite() {
    within_time_limit(assert_removed_coach_resend_refused(series_resend)).await;
}
