//! Emails (`MAIL`) whose recipients the router decides, read from the Resend stand-in.

use axum::http::{Method, StatusCode};
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

/// MAIL-04
#[tokio::test]
async fn mail_04_a_session_resend_mails_both_members() {
    within_time_limit(assert_resend_mails_both(|b| {
        format!("/coaching_sessions/{}/resend_invite", b.world.s1)
    }))
    .await;
}

/// MAIL-09
#[tokio::test]
async fn mail_09_a_series_resend_mails_both_members() {
    within_time_limit(assert_resend_mails_both(|b| {
        format!("/coaching_session_series/{}/resend_invite", b.world.sr1)
    }))
    .await;
}
