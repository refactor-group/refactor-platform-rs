use super::*;
use crate::{coaching_sessions, organizations, users, Id};
use chrono::NaiveDate;
use mockito::{Server, ServerGuard};
use service::config::Config;

async fn setup_test_server() -> ServerGuard {
    Server::new_async().await
}

/// `Matcher::Json` for a Resend body that guards against a `subject` key —
/// Resend templates own the subject line; payload-level subject is a bug.
fn expect_resend_body(expected: serde_json::Value) -> mockito::Matcher {
    assert!(
        expected.get("subject").is_none(),
        "test bug: expected body must not include `subject`",
    );
    mockito::Matcher::Json(expected)
}

/// Body matcher for a Resend request that also carries an `.ics` attachment:
/// partial-JSON on the template vars plus regexes requiring the `invite.ics`
/// attachment for the given `method`. The base64 `content` is intentionally not
/// asserted (dtstamp is `now`).
fn expect_resend_body_with_ics(
    expected: serde_json::Value,
    method: &ical::Method,
) -> mockito::Matcher {
    assert!(
        expected.get("subject").is_none(),
        "test bug: expected body must not include `subject`",
    );
    let method_name = match method {
        ical::Method::Request => "REQUEST",
        ical::Method::Cancel => "CANCEL",
    };
    mockito::Matcher::AllOf(vec![
        mockito::Matcher::PartialJson(expected),
        mockito::Matcher::Regex(r#""filename":"invite\.ics""#.to_string()),
        mockito::Matcher::Regex(format!(
            r#""content_type":"text/calendar; method={method_name}; charset=UTF-8""#
        )),
    ])
}

fn create_test_user() -> users::Model {
    users::Model {
        id: Id::new_v4(),
        first_name: "John".to_string(),
        last_name: "Doe".to_string(),
        email: "john.doe@example.com".to_string(),
        display_name: Some("John Doe".to_string()),
        password: Some("hashed_password".to_string()),
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        role: users::Role::User,
        roles: vec![],
        invite_status: None,
        created_at: chrono::Utc::now().fixed_offset(),
        updated_at: chrono::Utc::now().fixed_offset(),
    }
}

fn create_test_user_with(
    first_name: &str,
    last_name: &str,
    email: &str,
    timezone: &str,
) -> users::Model {
    users::Model {
        id: Id::new_v4(),
        first_name: first_name.to_string(),
        last_name: last_name.to_string(),
        email: email.to_string(),
        display_name: Some(format!("{first_name} {last_name}")),
        password: Some("hashed_password".to_string()),
        github_username: None,
        github_profile_url: None,
        timezone: timezone.to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        role: users::Role::User,
        roles: vec![],
        invite_status: None,
        created_at: chrono::Utc::now().fixed_offset(),
        updated_at: chrono::Utc::now().fixed_offset(),
    }
}

fn create_test_session() -> coaching_sessions::Model {
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: Id::new_v4(),
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: NaiveDate::from_ymd_opt(2026, 3, 4)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap(),
        duration_minutes: crate::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: chrono::Utc::now().fixed_offset(),
        updated_at: chrono::Utc::now().fixed_offset(),
        hydrated_at: Some(chrono::Utc::now().fixed_offset()),
    }
}

/// A session materialized inside a recurring series, sitting at its original start.
fn create_test_series_session(
    series_id: Id,
    original_start: NaiveDateTime,
) -> coaching_sessions::Model {
    coaching_sessions::Model {
        coaching_session_series_id: Some(series_id),
        ical_recurrence_id: Some(original_start),
        date: original_start,
        ..create_test_session()
    }
}

fn create_test_organization() -> organizations::Model {
    organizations::Model {
        id: Id::new_v4(),
        name: "Acme Corp".to_string(),
        logo: None,
        slug: "acme-corp".to_string(),
        created_at: chrono::Utc::now().fixed_offset(),
        updated_at: chrono::Utc::now().fixed_offset(),
        archived_at: None,
        archived_by: None,
    }
}

fn create_config_with_mock(server_url: &str) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        "--welcome-email-template-id=template_123",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={server_url}"),
    ])
}

fn create_full_config_with_mock(server_url: &str) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        "--welcome-email-template-id=template_123",
        "--session-scheduled-email-template-id=session_template_456",
        "--recurring-sessions-scheduled-email-template-id=recurring_template_xyz",
        "--session-rescheduled-email-template-id=session_reschedule_template_abc",
        "--recurring-sessions-rescheduled-email-template-id=series_reschedule_template_abc",
        "--session-cancelled-email-template-id=session_cancel_template_abc",
        "--recurring-sessions-cancelled-email-template-id=series_cancel_template_abc",
        "--action-assigned-email-template-id=action_template_789",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={server_url}"),
    ])
}

#[tokio::test]
async fn test_send_welcome_email_success() {
    let mut server = setup_test_server().await;
    let user = create_test_user();
    let inviter = create_test_user_with("Sarah", "Coach", "sarah.coach@example.com", "UTC");
    let config = create_config_with_mock(&server.url());

    let _mock = server
        .mock("POST", "/emails")
        .match_header("authorization", "Bearer test_api_key_123")
        .match_header("content-type", "application/json")
        .match_body(expect_resend_body(serde_json::json!({
            "from": FROM_ADDRESS,
            "to": ["\"John Doe\" <john.doe@example.com>"],
            "template": {
                "id": "template_123",
                "variables": {
                    "first_name": "John",
                    "last_name": "Doe",
                    "coach_first_name": "Sarah",
                    "coach_full_name": "Sarah Coach",
                    "magic_link_url": "https://app.example.com/setup/test-magic-link-token"
                }
            }
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"id":"email_msg_123456789"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_send_welcome_email_missing_api_key() {
    let config = Config::from_args(["test", "--welcome-email-template-id=template_123"]);
    assert!(config.resend_api_key().is_none(), "API key should be None");

    let user = create_test_user();
    let inviter = create_test_user();

    let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
    assert!(result.is_err());

    if let Err(e) = result {
        match e.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Config) => {}
            _ => panic!("Expected Config error, got: {:?}", e.error_kind),
        }
    }
}

#[tokio::test]
async fn test_send_welcome_email_missing_template_id() {
    let config = Config::from_args(["test", "--resend-api-key=test_api_key_123"]);
    assert!(
        config.resend_api_key().is_some(),
        "API key should be present"
    );
    assert!(
        config.welcome_email_template_id().is_none(),
        "Template ID should be None"
    );

    let user = create_test_user();
    let inviter = create_test_user();

    let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
    assert!(result.is_err());

    if let Err(e) = result {
        match e.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Config) => {}
            _ => panic!("Expected Config error, got: {:?}", e.error_kind),
        }
    }
}

#[tokio::test]
async fn test_send_welcome_email_http_error() {
    let mut server = setup_test_server().await;
    let user = create_test_user();
    let inviter = create_test_user();
    let config = create_config_with_mock(&server.url());

    let _mock = server
        .mock("POST", "/emails")
        .with_status(400)
        .with_body(r#"{"message": "Invalid request"}"#)
        .expect(1)
        .create_async()
        .await;

    // HTTP 400 from Resend should propagate as an error that carries the
    // response body — that body is the caller's only diagnostic.
    let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
    let err = result.unwrap_err();
    match err.error_kind {
        DomainErrorKind::Internal(InternalErrorKind::Other(text)) => assert!(
            text.contains("Invalid request"),
            "response body not propagated into error, got: {text}"
        ),
        other => panic!("expected Internal(Other), got: {other:?}"),
    }
}

#[tokio::test]
async fn test_send_welcome_email_escapes_name_with_specials() {
    // Integration-level counterpart to gateway::resend's
    // `test_format_mailbox_quotes_and_escapes_specials`: a user whose
    // assembled name contains a comma must land in the `to` field as a
    // quoted-string, not as two malformed mailboxes.
    let mut server = setup_test_server().await;
    let user = create_test_user_with("Jane", "Doe, Jr.", "jane.jr@example.com", "UTC");
    let inviter = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let config = create_config_with_mock(&server.url());

    let _mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body(serde_json::json!({
            "from": FROM_ADDRESS,
            "to": ["\"Jane Doe, Jr.\" <jane.jr@example.com>"],
            "template": {
                "id": "template_123",
                "variables": {
                    "first_name": "Jane",
                    "last_name": "Doe, Jr.",
                    "coach_first_name": "Alex",
                    "coach_full_name": "Alex Smith",
                    "magic_link_url": "https://app.example.com/setup/test-magic-link-token"
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
    assert!(result.is_ok());
}

// ── Password Reset Email Tests ─────────────────────────────────────

#[tokio::test]
async fn test_send_password_reset_email_wire_contract() {
    // Pins the wire contract that the Resend template depends on. The
    // gateway-level tests already prove the builder/HTTP plumbing works;
    // this test exists to catch regressions that are unique to *how this
    // function wires up* the request:
    //   1. `from` is the `mail.` subdomain — apex would not be verified
    //      in Resend and prod sends would silently fail.
    //   2. Variable keys are exactly `first_name`, `last_name`,
    //      `password_reset_url` — a rename would render to empty strings
    //      in the recipient's inbox (Resend still returns 200).
    //   3. The URL substitutes `{token}` (not `{session_id}` like other
    //      email types) — a copy-paste regression would send a malformed
    //      reset link.
    //   4. No `subject` field is present in the JSON payload — main moved
    //      subjects to be template-owned (commit 172907a); re-adding
    //      `.subject(...)` here would override the template default.
    let mut server = setup_test_server().await;
    let user = create_test_user_with("John", "Doe", "john@example.com", "UTC");
    let config = Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        "--password-reset-email-template-id=pw_reset_template_test",
        "--password-reset-email-url-path=/reset-password/{token}",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
    ]);

    // `Matcher::Json` is structural — any extra field (e.g. an
    // accidentally-readded `subject`) or missing/renamed variable will
    // fail the mock match and the test will hang on `expect(1)`.
    let _mock = server
        .mock("POST", "/emails")
        .match_header("authorization", "Bearer test_api_key_123")
        .match_body(expect_resend_body(serde_json::json!({
            "from": FROM_ADDRESS,
            "to": ["\"John Doe\" <john@example.com>"],
            "template": {
                "id": "pw_reset_template_test",
                "variables": {
                    "first_name": "John",
                    "last_name": "Doe",
                    "password_reset_url": "https://app.example.com/reset-password/raw-reset-token-abc"
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_pw_reset"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_password_reset_email(&config, &user, "raw-reset-token-abc").await;
    assert!(
        result.is_ok(),
        "send_password_reset_email failed: {result:?}"
    );
}

// ── Session Scheduled Email Tests ──────────────────────────────────

/// RFC 5545 line unfolding: remove the CRLF followed by a leading space/tab.
fn unfold(s: &str) -> String {
    s.replace("\r\n ", "").replace("\r\n\t", "")
}

/// A single-session invite from a New York coach and a Los Angeles coachee.
fn invite_ics_for_participants(coach: &users::Model, coachee: &users::Model) -> String {
    let mut session = create_test_session();
    session.date = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    session.duration_minutes = 60;
    let org = create_test_organization();
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics =
        build_session_invite_ics(coach, coachee, &session, &org, "desc".into(), dtstamp).unwrap();
    unfold(&ics)
}

/// The `ORGANIZER` must equal the sending address: when the two disagree, calendar
/// clients treat the invite as untrusted and silently drop reschedules.
#[test]
fn test_organizer_is_the_sending_address() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/Los_Angeles");
    let ics = invite_ics_for_participants(&coach, &coachee);

    let organizer = ics
        .lines()
        .find(|l| l.starts_with("ORGANIZER"))
        .expect("no ORGANIZER line");
    assert!(
        organizer.contains(&format!("mailto:{FROM_ADDRESS}")),
        "ORGANIZER must carry FROM_ADDRESS, got: {organizer}"
    );
}

#[test]
fn test_coach_and_coachee_are_both_attendees() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/Los_Angeles");
    let ics = invite_ics_for_participants(&coach, &coachee);

    let attendees: Vec<&str> = ics.lines().filter(|l| l.starts_with("ATTENDEE")).collect();
    assert_eq!(attendees.len(), 2);
    assert!(attendees
        .iter()
        .any(|l| l.contains("mailto:alex@example.com")));
    assert!(attendees
        .iter()
        .any(|l| l.contains("mailto:jane@example.com")));

    let organizer = ics
        .lines()
        .find(|l| l.starts_with("ORGANIZER"))
        .expect("no ORGANIZER line");
    assert!(!organizer.contains("alex@example.com"));
    assert!(!organizer.contains("jane@example.com"));
}

/// The platform organizer has no meaningful zone, so the anchor stays the coach's.
#[test]
fn test_anchor_timezone_still_follows_the_coach() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/Los_Angeles");
    let ics = invite_ics_for_participants(&coach, &coachee);

    assert!(ics.contains("DTSTART;TZID=America/New_York:"));
    assert!(ics.contains("BEGIN:VTIMEZONE"));
    assert!(ics.contains("TZID:America/New_York"));
}

#[test]
fn test_build_session_invite_ics_structure() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let mut session = create_test_session();
    session.date = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    session.duration_minutes = 60;
    session.ical_sequence = 0;
    session.meeting_url = Some("https://meet.example/xyz".into());
    let mut org = create_test_organization();
    org.name = "Acme".to_string();
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics = build_session_invite_ics(
        &coach,
        &coachee,
        &session,
        &org,
        "View this session: https://app/x".into(),
        dtstamp,
    )
    .unwrap();

    assert!(ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
    assert!(ics.contains("METHOD:REQUEST"));
    assert!(ics.contains("STATUS:CONFIRMED"));
    assert!(ics.contains("SEQUENCE:0"));
    assert!(ics.contains("SUMMARY:Coaching Session: Acme"));
    assert!(ics.contains("BEGIN:VTIMEZONE"));
    assert!(ics.contains("TZID:America/New_York"));
    assert!(ics.contains("DTSTART;TZID=America/New_York:20260915T150000"));
    assert!(ics.contains("View this session: https://app/x"));
}

/// A reschedule bumps `ical_sequence`; the invite must carry the bumped
/// `SEQUENCE` while keeping the same `UID` (so calendar clients update the
/// existing event in place rather than creating a duplicate).
#[test]
fn test_build_session_invite_ics_bumped_sequence() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let mut session = create_test_session();
    session.date = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    session.duration_minutes = 60;
    session.ical_sequence = 1;
    session.meeting_url = Some("https://meet.example/xyz".into());
    let mut org = create_test_organization();
    org.name = "Acme".to_string();
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics = build_session_invite_ics(
        &coach,
        &coachee,
        &session,
        &org,
        "View this session: https://app/x".into(),
        dtstamp,
    )
    .unwrap();

    assert!(ics.contains("SEQUENCE:1"));
    assert!(ics.contains("METHOD:REQUEST"));
    // UID keys off the session id, unchanged from a SEQUENCE:0 invite.
    assert!(ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_session_scheduled_email_variables() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    // Description loaders return empty sets; the `.ics` content is not asserted here.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::coaching_session_topics::Model, _, _>(vec![vec![]])
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .append_query_results::<entity::actions::Model, _, _>(vec![vec![]])
        .into_connection();

    // Coach and coachee in different timezones so a single body-match per
    // recipient proves BOTH the role swap (coach <-> coachee) AND that each
    // recipient's own timezone is used. Session is 2026-03-04 15:00 UTC:
    //   - coachee (America/New_York, EST): 10:00 AM, Wed March 4
    //   - coach   (Asia/Tokyo):            12:00 AM, Thu March 5 (date rolls)
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "Asia/Tokyo");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/New_York");
    let session = create_test_session();
    let org = create_test_organization();

    let session_url = format!("https://app.example.com/coaching-sessions/{}", session.id);

    // Email to coachee — other_user is the coach, formatted in NY time.
    let mock_coachee = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "session_template_456",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_first_name": "Alex",
                        "other_user_last_name": "Smith",
                        "other_user_role": "coach",
                        "organization_name": "Acme Corp",
                        "session_date": "Wednesday, March 4, 2026",
                        "session_time": "10:00 AM",
                        "session_duration": "1 hour",
                        "session_url": session_url.clone(),
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    // Email to coach — other_user is the coachee, formatted in Tokyo time
    // (the session date rolls forward a day).
    let mock_coach = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Alex Smith\" <alex@example.com>"],
                "template": {
                    "id": "session_template_456",
                    "variables": {
                        "first_name": "Alex",
                        "other_user_first_name": "Jane",
                        "other_user_last_name": "Doe",
                        "other_user_role": "coachee",
                        "organization_name": "Acme Corp",
                        "session_date": "Thursday, March 5, 2026",
                        "session_time": "12:00 AM",
                        "session_duration": "1 hour",
                        "session_url": session_url.clone(),
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_session_scheduled_email(&db, &config, &coach, &coachee, &session, &org).await;
    assert!(result.is_ok());

    // Sends are best-effort (errors are swallowed), so assert the mocks
    // matched to prove the attachment-bearing bodies actually went out.
    mock_coachee.assert_async().await;
    mock_coach.assert_async().await;
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_session_rescheduled_email() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    // Description loaders return empty sets; the `.ics` content is not asserted here.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::coaching_session_topics::Model, _, _>(vec![vec![]])
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .append_query_results::<entity::actions::Model, _, _>(vec![vec![]])
        .into_connection();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "Asia/Tokyo");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/New_York");
    // Already-bumped session: a reschedule invite carries SEQUENCE:2.
    let mut session = create_test_session();
    session.ical_sequence = 2;
    let org = create_test_organization();

    // Both recipients target the reschedule template and carry the
    // `session_or_series=session` discriminant plus the `.ics` attachment.
    let mock_coachee = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "session_reschedule_template_abc",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_role": "coach",
                        "session_or_series": "session",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let mock_coach = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "to": ["\"Alex Smith\" <alex@example.com>"],
                "template": {
                    "id": "session_reschedule_template_abc",
                    "variables": {
                        "first_name": "Alex",
                        "other_user_role": "coachee",
                        "session_or_series": "session",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    // The start moved forward from March 2.
    let previous_start = NaiveDate::from_ymd_opt(2026, 3, 2)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap();
    let result = send_session_rescheduled_email(
        &db,
        &config,
        &coach,
        &coachee,
        &session,
        &org,
        previous_start,
    )
    .await;
    assert!(result.is_ok());

    // The send swallows errors, so the mock assertions are what give this
    // test teeth: they prove the reschedule template + attachment went out.
    mock_coachee.assert_async().await;
    mock_coach.assert_async().await;
}

/// Loaders for the single-session `.ics` description: topics, goals, actions.
#[cfg(feature = "mock")]
fn mock_description_loaders() -> DatabaseConnection {
    use sea_orm::{DatabaseBackend, MockDatabase};

    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::coaching_session_topics::Model, _, _>(vec![vec![]])
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .append_query_results::<entity::actions::Model, _, _>(vec![vec![]])
        .into_connection()
}

/// mockito has no negative body matcher, so absence of a template variable rides on
/// `match_request`. An unreadable or unparsable body fails the match.
fn reject_template_variables(keys: &'static [&'static str]) -> impl Fn(&mockito::Request) -> bool {
    move |request| {
        request
            .utf8_lossy_body()
            .ok()
            .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
            .map(|payload| {
                keys.iter()
                    .all(|key| payload["template"]["variables"].get(key).is_none())
            })
            .unwrap_or(false)
    }
}

/// T1: both `when` variables ride along on a reschedule, each rendered in the
/// recipient's own timezone.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_session_rescheduled_email_carries_both_when_variables_per_recipient() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let db = mock_description_loaders();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/Chicago");
    // New start 2026-03-04 15:00 UTC: 9:00 AM Central, 10:00 AM Eastern.
    let session = create_test_session();
    let org = create_test_organization();
    // Old start 2026-02-25 20:00 UTC: 2:00 PM Central, 3:00 PM Eastern.
    let previous_start = NaiveDate::from_ymd_opt(2026, 2, 25)
        .unwrap()
        .and_hms_opt(20, 0, 0)
        .unwrap();

    let mock_coachee = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "session_reschedule_template_abc",
                    "variables": {
                        "first_name": "Jane",
                        "session_when": "Wednesday, March 4, 2026 at 9:00 AM",
                        "previous_session_when": "Wednesday, February 25, 2026 at 2:00 PM",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let mock_coach = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "to": ["\"Alex Smith\" <alex@example.com>"],
                "template": {
                    "id": "session_reschedule_template_abc",
                    "variables": {
                        "first_name": "Alex",
                        "session_when": "Wednesday, March 4, 2026 at 10:00 AM",
                        "previous_session_when": "Wednesday, February 25, 2026 at 3:00 PM",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_session_rescheduled_email(
        &db,
        &config,
        &coach,
        &coachee,
        &session,
        &org,
        previous_start,
    )
    .await;
    assert!(result.is_ok());

    mock_coachee.assert_async().await;
    mock_coach.assert_async().await;
}

/// T2: a title-only reschedule leaves the start where it was, so the previous time
/// reads `Unchanged` while the new time still shows the real start.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_session_rescheduled_email_unmoved_start_renders_unchanged() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let db = mock_description_loaders();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let mut session = create_test_session();
    session.title = Some("Renamed session".to_string());
    let org = create_test_organization();

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "session_reschedule_template_abc",
                    "variables": {
                        "session_when": "Wednesday, March 4, 2026 at 3:00 PM",
                        "previous_session_when": "Unchanged",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_session_rescheduled_email(
        &db,
        &config,
        &coach,
        &coachee,
        &session,
        &org,
        session.date,
    )
    .await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

/// T3: a moved start must render the old time, never the `Unchanged` literal.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_session_rescheduled_email_moved_start_is_not_unchanged() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let db = mock_description_loaders();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session = create_test_session();
    let org = create_test_organization();
    let previous_start = NaiveDate::from_ymd_opt(2026, 3, 2)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap();

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "session_reschedule_template_abc",
                    "variables": {
                        "session_when": "Wednesday, March 4, 2026 at 3:00 PM",
                        "previous_session_when": "Monday, March 2, 2026 at 3:00 PM",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_session_rescheduled_email(
        &db,
        &config,
        &coach,
        &coachee,
        &session,
        &org,
        previous_start,
    )
    .await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

/// T5: the scheduled path keeps its existing variables and carries neither
/// `when` variable, whose template does not declare them.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_session_scheduled_email_omits_when_variables() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let db = mock_description_loaders();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session = create_test_session();
    let org = create_test_organization();
    let session_url = format!("https://app.example.com/coaching-sessions/{}", session.id);

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "session_template_456",
                    "variables": {
                        "organization_name": "Acme Corp",
                        "session_date": "Wednesday, March 4, 2026",
                        "session_time": "3:00 PM",
                        "session_duration": "1 hour",
                        "session_url": session_url,
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .match_request(reject_template_variables(&[
            "session_when",
            "previous_session_when",
        ]))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_session_scheduled_email(&db, &config, &coach, &coachee, &session, &org).await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

/// Frequency is meaningless for a one-off session, so neither recurrence variable
/// ships on the single-session reschedule.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_session_rescheduled_email_omits_recurrence_variables() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let db = mock_description_loaders();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session = create_test_session();
    let org = create_test_organization();
    let previous_start = NaiveDate::from_ymd_opt(2026, 3, 2)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap();

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "session_reschedule_template_abc",
                    "variables": {
                        "previous_session_when": "Monday, March 2, 2026 at 3:00 PM",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .match_request(reject_template_variables(&[
            "recurrence_summary",
            "previous_recurrence_summary",
        ]))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_session_rescheduled_email(
        &db,
        &config,
        &coach,
        &coachee,
        &session,
        &org,
        previous_start,
    )
    .await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

// ── Session Cancelled Email Tests ──────────────────────────────────

/// A cancellation must supersede the invite it replaces: same `UID`, next `SEQUENCE`.
#[test]
fn test_build_session_cancel_ics_structure() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let mut session = create_test_session();
    session.date = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    session.duration_minutes = 60;
    session.ical_sequence = 2;
    let org = create_test_organization();
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics = build_session_cancel_ics(
        &coach,
        &coachee,
        &session,
        &org,
        SESSION_CANCELLED_DESCRIPTION.to_string(),
        dtstamp,
    )
    .unwrap();

    assert!(ics.contains("METHOD:CANCEL"));
    assert!(ics.contains("STATUS:CANCELLED"));
    assert!(ics.contains("SEQUENCE:3"));
    assert!(ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
}

#[tokio::test]
async fn test_send_session_cancelled_email() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    // No session link: the row is gone by the time a recipient could click it.
    assert!(
        SessionCancelled::url_path_template(&config).is_none(),
        "a cancellation must not carry a session URL template"
    );

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "Asia/Tokyo");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/New_York");
    let session = create_test_session();
    let org = create_test_organization();

    // Email to coachee: other_user is the coach, formatted in NY time.
    let mock_coachee = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "session_cancel_template_abc",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_first_name": "Alex",
                        "other_user_last_name": "Smith",
                        "other_user_role": "coach",
                        "organization_name": "Acme Corp",
                        "session_date": "Wednesday, March 4, 2026",
                        "session_time": "10:00 AM",
                    }
                }
            }),
            &ical::Method::Cancel,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    // Email to coach: other_user is the coachee, Tokyo time rolls the date forward.
    let mock_coach = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Alex Smith\" <alex@example.com>"],
                "template": {
                    "id": "session_cancel_template_abc",
                    "variables": {
                        "first_name": "Alex",
                        "other_user_first_name": "Jane",
                        "other_user_last_name": "Doe",
                        "other_user_role": "coachee",
                        "organization_name": "Acme Corp",
                        "session_date": "Thursday, March 5, 2026",
                        "session_time": "12:00 AM",
                    }
                }
            }),
            &ical::Method::Cancel,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_session_cancelled_email(&config, &coach, &coachee, &session, &org).await;
    assert!(result.is_ok());

    // The send swallows errors, so the mock assertions are what give this test teeth.
    mock_coachee.assert_async().await;
    mock_coach.assert_async().await;
}

/// T5: a cancellation keeps its existing variables and carries neither `when`
/// variable, whose template does not declare them.
#[tokio::test]
async fn test_send_session_cancelled_email_omits_when_variables() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session = create_test_session();
    let org = create_test_organization();

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "session_cancel_template_abc",
                    "variables": {
                        "organization_name": "Acme Corp",
                        "session_date": "Wednesday, March 4, 2026",
                        "session_time": "3:00 PM",
                    }
                }
            }),
            &ical::Method::Cancel,
        ))
        .match_request(reject_template_variables(&[
            "session_when",
            "previous_session_when",
        ]))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_session_cancelled_email(&config, &coach, &coachee, &session, &org).await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

// ── Per-Occurrence (Series Member) Tests ───────────────────────────

/// The VEVENT span only. A spliced VTIMEZONE block carries its own daylight-saving
/// `RRULE` lines, so whole-calendar RRULE assertions would be meaningless.
fn vevent(ics: &str) -> &str {
    let start = ics.find("BEGIN:VEVENT").expect("no BEGIN:VEVENT");
    let end = ics.find("END:VEVENT").expect("no END:VEVENT");
    &ics[start..end]
}

/// Cancelling one occurrence must address the SERIES event plus a RECURRENCE-ID.
/// A session-id UID names an event no calendar holds.
#[test]
fn test_build_occurrence_cancel_ics_addresses_the_series_uid() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let series_id = Id::new_v4();
    let original_start = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    let mut session = create_test_series_session(series_id, original_start);
    session.ical_sequence = 2;
    session.duration_minutes = 60;
    let org = create_test_organization();
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics = build_occurrence_cancel_ics(
        &coach,
        &coachee,
        &session,
        series_id,
        &org,
        SESSION_CANCELLED_DESCRIPTION.to_string(),
        dtstamp,
    )
    .unwrap();

    assert!(ics.contains(&format!("UID:{series_id}@myrefactor.com")));
    assert!(!ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
    assert!(ics.contains("RECURRENCE-ID;TZID=America/New_York:20260915T150000"));
    assert!(ics.contains("METHOD:CANCEL"));
    assert!(ics.contains("STATUS:CANCELLED"));
    assert!(ics.contains("SEQUENCE:3"));
    assert!(!vevent(&ics).contains("RRULE"));
}

/// Moving one occurrence: DTSTART carries the NEW time while RECURRENCE-ID keeps
/// naming the original slot, which is what makes it an override and not a duplicate.
#[test]
fn test_build_occurrence_reschedule_ics_keeps_original_recurrence_id() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let series_id = Id::new_v4();
    let original_start = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    let mut session = create_test_series_session(series_id, original_start);
    session.ical_sequence = 3;
    session.duration_minutes = 60;
    session.date = NaiveDate::from_ymd_opt(2026, 9, 17)
        .unwrap()
        .and_hms_opt(20, 0, 0)
        .unwrap();
    let org = create_test_organization();
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics = build_occurrence_reschedule_ics(
        &coach,
        &coachee,
        &session,
        series_id,
        &org,
        "View this session: https://app/x".into(),
        dtstamp,
    )
    .unwrap();

    assert!(ics.contains("DTSTART;TZID=America/New_York:20260917T160000"));
    assert!(ics.contains("RECURRENCE-ID;TZID=America/New_York:20260915T150000"));
    assert_ne!(session.date, original_start);
    assert!(ics.contains(&format!("UID:{series_id}@myrefactor.com")));
    assert!(!ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
    assert!(ics.contains("METHOD:REQUEST"));
    assert!(ics.contains("SEQUENCE:3"));
    assert!(!vevent(&ics).contains("RRULE"));
}

/// Standalone sessions keep their own UID and gain no RECURRENCE-ID.
#[test]
fn test_standalone_session_ics_carries_no_recurrence_id() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session = create_test_session();
    let org = create_test_organization();
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    assert!(session.coaching_session_series_id.is_none());

    let invite =
        build_session_invite_ics(&coach, &coachee, &session, &org, "desc".into(), dtstamp).unwrap();
    assert!(invite.contains(&format!("UID:{}@myrefactor.com", session.id)));
    assert!(!invite.contains("RECURRENCE-ID"));

    let cancel =
        build_session_cancel_ics(&coach, &coachee, &session, &org, "desc".into(), dtstamp).unwrap();
    assert!(cancel.contains(&format!("UID:{}@myrefactor.com", session.id)));
    assert!(!cancel.contains("RECURRENCE-ID"));
}

/// A series member with no `ical_recurrence_id` has no valid slot to address, so the
/// guard must fire before any lookup and nothing may be sent.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_notify_session_cancelled_skips_series_member_without_recurrence_id() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let mut session = create_test_session();
    session.coaching_session_series_id = Some(Id::new_v4());
    session.ical_recurrence_id = None;
    // Future-dated so the past-session guard cannot be what stops the send.
    session.date = chrono::Utc::now().naive_utc() + chrono::Duration::days(7);

    // Zero appended results: any query would panic or error rather than pass.
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    notify_session_cancelled(&db, &config, &session).await;

    assert!(
        db.into_transaction_log().is_empty(),
        "a series member without a RECURRENCE-ID must return before any statement runs"
    );
}

/// Per-occurrence cancellations are still one session being cancelled, so they use the
/// session template and the CANCEL attachment, not any series-level variant.
#[tokio::test]
async fn test_send_occurrence_cancelled_email_uses_session_template() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "Asia/Tokyo");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/New_York");
    let series_id = Id::new_v4();
    let original_start = NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap();
    let session = create_test_series_session(series_id, original_start);
    let org = create_test_organization();

    let mock_coachee = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "session_cancel_template_abc",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_role": "coach",
                        "session_date": "Wednesday, March 4, 2026",
                        "session_time": "10:00 AM",
                    }
                }
            }),
            &ical::Method::Cancel,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let mock_coach = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "to": ["\"Alex Smith\" <alex@example.com>"],
                "template": {
                    "id": "session_cancel_template_abc",
                    "variables": {
                        "first_name": "Alex",
                        "other_user_role": "coachee",
                        "session_date": "Thursday, March 5, 2026",
                        "session_time": "12:00 AM",
                    }
                }
            }),
            &ical::Method::Cancel,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_session_cancelled_email(&config, &coach, &coachee, &session, &org).await;
    assert!(result.is_ok());

    // The send swallows errors, so the mock assertions are what give this test teeth.
    mock_coachee.assert_async().await;
    mock_coach.assert_async().await;
}

/// Deleting an already-completed session is housekeeping, not a cancellation. The
/// guard must fire before any lookup, so the connection sees no statements at all.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_notify_session_cancelled_skips_past_session() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let mut session = create_test_session();
    session.date = NaiveDate::from_ymd_opt(2020, 1, 15)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap();

    // Zero appended results: any query would panic or error rather than pass.
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    notify_session_cancelled(&db, &config, &session).await;

    assert!(
        db.into_transaction_log().is_empty(),
        "a past session must return before any statement runs"
    );
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_session_scheduled_email_missing_template_id() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    // Config has an API key and frontend base URL but no session-scheduled
    // template id — mirrors the welcome/action missing-template-id tests.
    // Fails at config resolution, before any loader query runs.
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
    let server = setup_test_server().await;
    let config = Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        &format!("--resend-base-url={}", server.url()),
        "--frontend-base-url=https://app.example.com",
    ]);

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session = create_test_session();
    let org = create_test_organization();

    let result = send_session_scheduled_email(&db, &config, &coach, &coachee, &session, &org).await;

    assert!(result.is_err());
    if let Err(e) = result {
        match e.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Config) => {}
            _ => panic!("Expected Config error, got: {:?}", e.error_kind),
        }
    }
}

// ── Action Assigned Email Tests ────────────────────────────────────

#[tokio::test]
async fn test_send_action_assigned_email_success() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let assignee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session_id = Id::new_v4();
    let org = create_test_organization();

    let session_url = format!("https://app.example.com/coaching-sessions/{session_id}?tab=actions");
    let due_by: DateTime<FixedOffset> = NaiveDate::from_ymd_opt(2026, 3, 7)
        .unwrap()
        .and_hms_opt(17, 0, 0)
        .unwrap()
        .and_utc()
        .fixed_offset();

    let _mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body(serde_json::json!({
            "from": FROM_ADDRESS,
            "to": ["\"Jane Doe\" <jane@example.com>"],
            "template": {
                "id": "action_template_789",
                "variables": {
                    "first_name": "Jane",
                    "action_body": "Read chapters 3-5 of Radical Candor",
                    "due_date": "Saturday, March 7, 2026",
                    "assigner_first_name": "Alex",
                    "assigner_last_name": "Smith",
                    "organization_name": "Acme Corp",
                    "goal": "Improve communication",
                    "session_url": session_url,
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let ctx = ActionEmailContext {
        action_body: "Read chapters 3-5 of Radical Candor",
        due_by: Some(due_by),
        session_id,
        organization: &org,
        goal: Some("Improve communication"),
    };

    let result = send_action_assigned_email(&config, &[assignee], &assigner, &ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_send_action_assigned_email_no_due_date() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let assignee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session_id = Id::new_v4();
    let org = create_test_organization();

    let session_url = format!("https://app.example.com/coaching-sessions/{session_id}?tab=actions");

    let _mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body(serde_json::json!({
            "from": FROM_ADDRESS,
            "to": ["\"Jane Doe\" <jane@example.com>"],
            "template": {
                "id": "action_template_789",
                "variables": {
                    "first_name": "Jane",
                    "action_body": "Follow up with team",
                    "due_date": "No due date set",
                    "assigner_first_name": "Alex",
                    "assigner_last_name": "Smith",
                    "organization_name": "Acme Corp",
                    "session_url": session_url,
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let ctx = ActionEmailContext {
        action_body: "Follow up with team",
        due_by: None,
        session_id,
        organization: &org,
        goal: None,
    };

    let result = send_action_assigned_email(&config, &[assignee], &assigner, &ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_send_action_assigned_email_multiple_assignees() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let assignee1 = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let assignee2 = create_test_user_with("Bob", "Jones", "bob@example.com", "UTC");
    let session_id = Id::new_v4();
    let org = create_test_organization();

    let session_url = format!("https://app.example.com/coaching-sessions/{session_id}?tab=actions");

    // Each assignee must get their OWN email with their OWN first_name and
    // recipient address. Body-match per recipient so a regression that sends
    // both emails to the same person (or with swapped variables) fails here.
    let _mock_jane = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body(serde_json::json!({
            "from": FROM_ADDRESS,
            "to": ["\"Jane Doe\" <jane@example.com>"],
            "template": {
                "id": "action_template_789",
                "variables": {
                    "first_name": "Jane",
                    "action_body": "Complete the survey",
                    "due_date": "No due date set",
                    "assigner_first_name": "Alex",
                    "assigner_last_name": "Smith",
                    "organization_name": "Acme Corp",
                    "session_url": session_url.clone(),
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let _mock_bob = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body(serde_json::json!({
            "from": FROM_ADDRESS,
            "to": ["\"Bob Jones\" <bob@example.com>"],
            "template": {
                "id": "action_template_789",
                "variables": {
                    "first_name": "Bob",
                    "action_body": "Complete the survey",
                    "due_date": "No due date set",
                    "assigner_first_name": "Alex",
                    "assigner_last_name": "Smith",
                    "organization_name": "Acme Corp",
                    "session_url": session_url.clone(),
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let ctx = ActionEmailContext {
        action_body: "Complete the survey",
        due_by: None,
        session_id,
        organization: &org,
        goal: None,
    };

    let result =
        send_action_assigned_email(&config, &[assignee1, assignee2], &assigner, &ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_send_action_assigned_email_missing_template_id() {
    let server = setup_test_server().await;
    let config = Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        &format!("--resend-base-url={}", server.url()),
        "--frontend-base-url=https://app.example.com",
    ]);

    let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let assignee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let session_id = Id::new_v4();
    let org = create_test_organization();

    let ctx = ActionEmailContext {
        action_body: "Some action",
        due_by: None,
        session_id,
        organization: &org,
        goal: None,
    };

    let result = send_action_assigned_email(&config, &[assignee], &assigner, &ctx).await;

    assert!(result.is_err());
    if let Err(e) = result {
        match e.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Config) => {}
            _ => panic!("Expected Config error, got: {:?}", e.error_kind),
        }
    }
}

#[tokio::test]
async fn test_send_action_assigned_email_empty_assignees_sends_nothing() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let session_id = Id::new_v4();
    let org = create_test_organization();

    // Expect exactly zero calls — no assignees means no emails
    let _mock = server
        .mock("POST", "/emails")
        .expect(0)
        .create_async()
        .await;

    let ctx = ActionEmailContext {
        action_body: "Some action",
        due_by: None,
        session_id,
        organization: &org,
        goal: None,
    };

    let result = send_action_assigned_email(&config, &[], &assigner, &ctx).await;
    assert!(result.is_ok());
}

// ── build_session_url Unit Tests ────────────────────────────────────

/// Helper to construct a `ResolvedEmailConfig` with an optional
/// `SessionUrlBuilder`, without needing a real Resend client.
async fn create_test_email_config(
    server_url: &str,
    url_builder: Option<SessionUrlBuilder>,
) -> ResolvedEmailConfig {
    let config = create_config_with_mock(server_url);
    ResolvedEmailConfig {
        client: ResendClient::new(&config).await.unwrap(),
        template_id: "test_template".to_string(),
        session_url_builder: url_builder,
    }
}

#[tokio::test]
async fn test_build_session_url_success() {
    let server = setup_test_server().await;
    let email_config = create_test_email_config(
        &server.url(),
        Some(SessionUrlBuilder {
            base_url: "https://app.example.com".to_string(),
            path_template: "/coaching-sessions/{session_id}".to_string(),
        }),
    )
    .await;

    let session_id = Id::new_v4();
    let result = email_config.build_session_url(&session_id);

    assert!(result.is_ok());
    assert_eq!(
        result.unwrap(),
        format!("https://app.example.com/coaching-sessions/{session_id}")
    );
}

#[tokio::test]
async fn test_build_session_url_custom_path_template() {
    let server = setup_test_server().await;
    let email_config = create_test_email_config(
        &server.url(),
        Some(SessionUrlBuilder {
            base_url: "https://app.example.com".to_string(),
            path_template: "/sessions/{session_id}?tab=actions".to_string(),
        }),
    )
    .await;

    let session_id = Id::new_v4();
    let result = email_config.build_session_url(&session_id).unwrap();

    assert_eq!(
        result,
        format!("https://app.example.com/sessions/{session_id}?tab=actions")
    );
}

#[tokio::test]
async fn test_build_session_url_no_url_builder() {
    let server = setup_test_server().await;
    let email_config = create_test_email_config(&server.url(), None).await;

    let result = email_config.build_session_url(&Id::new_v4());

    assert!(result.is_err());
    if let Err(e) = result {
        match e.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Config) => {}
            _ => panic!("Expected Config error, got: {:?}", e.error_kind),
        }
    }
}

// ── Recurring Sessions Scheduled Email Tests ───────────────────────

#[cfg(feature = "mock")]
fn create_test_session_on(date: NaiveDate) -> coaching_sessions::Model {
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: Id::new_v4(),
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: date.and_hms_opt(15, 0, 0).unwrap(),
        duration_minutes: crate::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: chrono::Utc::now().fixed_offset(),
        updated_at: chrono::Utc::now().fixed_offset(),
        hydrated_at: None,
    }
}

fn create_test_series() -> coaching_session_series::Model {
    coaching_session_series::Model {
        id: Id::new_v4(),
        coaching_relationship_id: Id::new_v4(),
        rule: serde_json::json!({
            "start_at": "2026-09-15T15:00:00",
            "recurrence": { "frequency": "weekly", "interval": 1 },
            "duration_minutes": 60,
        }),
        ical_sequence: 0,
        created_by_user_id: Id::new_v4(),
        created_at: chrono::Utc::now().fixed_offset(),
        updated_at: chrono::Utc::now().fixed_offset(),
    }
}

#[test]
fn test_build_series_invite_ics_structure() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let mut first = create_test_session();
    first.date = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    first.duration_minutes = 60;
    first.meeting_url = Some("https://meet.example/xyz".into());
    let mut org = create_test_organization();
    org.name = "Acme".to_string();
    let series = create_test_series();
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics = build_series_invite_ics(
        &coach,
        &coachee,
        &first,
        &org,
        &series,
        "View this session: https://app/x".into(),
        dtstamp,
    )
    .unwrap();

    assert!(ics.contains(&format!("UID:{}@myrefactor.com", series.id)));
    assert!(ics.contains("METHOD:REQUEST"));
    assert!(ics.contains("STATUS:CONFIRMED"));
    assert!(ics.contains("SEQUENCE:0"));
    assert!(ics.contains("RRULE:FREQ=WEEKLY"));
    assert!(ics.contains("BEGIN:VTIMEZONE"));
    assert!(ics.contains("TZID:America/New_York"));
    assert!(ics.contains("DTSTART;TZID=America/New_York:20260915T150000"));
    assert!(ics.contains("View this session: https://app/x"));
}

/// A series reschedule bumps `ical_sequence`; the invite must carry the bumped
/// `SEQUENCE` under the same series-derived `UID` so calendar clients replace the
/// existing recurring event instead of duplicating it.
#[test]
fn test_build_series_invite_ics_carries_bumped_sequence() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let mut first = create_test_session();
    first.date = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    first.duration_minutes = 60;
    let org = create_test_organization();
    let mut series = create_test_series();
    series.ical_sequence = 3;
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics = build_series_invite_ics(
        &coach,
        &coachee,
        &first,
        &org,
        &series,
        "View this session: https://app/x".into(),
        dtstamp,
    )
    .unwrap();

    assert!(ics.contains("SEQUENCE:3"));
    assert!(ics.contains(&format!("UID:{}@myrefactor.com", series.id)));
    assert!(ics.contains("RRULE:FREQ=WEEKLY"));
    assert!(ics.contains("METHOD:REQUEST"));
    assert!(ics.contains("STATUS:CONFIRMED"));
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_scheduled_email_personalization() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    // Series description loads only the first session's in-progress goals.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .into_connection();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();
    let series = create_test_series();

    let sessions = vec![
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
    ];

    let first_session_url = format!(
        "https://app.example.com/coaching-sessions/{}",
        sessions[0].id
    );

    // Email to coachee — other_user is the coach. Body-match per recipient
    // proves the role swap; the `.ics` attachment must be present.
    let mock_coachee = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "recurring_template_xyz",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_first_name": "Alex",
                        "other_user_last_name": "Smith",
                        "other_user_role": "coach",
                        "organization_name": "Acme Corp",
                        "session_count": 3,
                        "first_session_date": "Wednesday, March 4, 2026",
                        "first_session_time": "3:00 PM",
                        "last_session_date": "Wednesday, March 18, 2026",
                        "session_duration": "1 hour",
                        "session_url": first_session_url,
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    // Email to coach — require the `.ics` attachment too.
    let mock_coach = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": { "id": "recurring_template_xyz" }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_recurring_sessions_scheduled_email(
        &db, &config, &series, &coach, &coachee, &sessions, &org,
    )
    .await;
    assert!(result.is_ok());

    // Sends are best-effort (errors are swallowed), so assert the mocks
    // matched to prove the attachment-bearing bodies actually went out.
    mock_coachee.assert_async().await;
    mock_coach.assert_async().await;
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_rescheduled_email() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    // Series description loads only the first session's in-progress goals.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .into_connection();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();
    // Already-bumped series: a reschedule invite carries SEQUENCE:3.
    let mut series = create_test_series();
    series.ical_sequence = 3;

    let sessions = vec![
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
    ];

    // Both recipients target the reschedule template and carry the
    // `session_or_series=series` discriminant plus the `.ics` attachment.
    let mock_coachee = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "series_reschedule_template_abc",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_role": "coach",
                        "session_or_series": "series",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let mock_coach = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "to": ["\"Alex Smith\" <alex@example.com>"],
                "template": {
                    "id": "series_reschedule_template_abc",
                    "variables": {
                        "first_name": "Alex",
                        "other_user_role": "coachee",
                        "session_or_series": "series",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_recurring_sessions_rescheduled_email(
        &db,
        &config,
        &series,
        &create_test_series(),
        &coach,
        &coachee,
        &sessions,
        &org,
    )
    .await;
    assert!(result.is_ok());

    // The send swallows errors, so the mock assertions are what give this
    // test teeth: they prove the reschedule template + attachment went out.
    mock_coachee.assert_async().await;
    mock_coach.assert_async().await;
}

/// T4: the series reschedule shows the OLD rule's `start_at` as the previous time and
/// the new first occurrence as the new time.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_rescheduled_email_previous_when_comes_from_old_rule() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    // Series description loads only the first session's in-progress goals.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .into_connection();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();

    // Old rule starts 2026-09-15 15:00; the replacement rule starts three weeks later.
    let previous_series = create_test_series();
    let mut series = create_test_series();
    series.ical_sequence = 1;
    series.rule = serde_json::json!({
        "start_at": "2026-10-06T15:00:00",
        "recurrence": { "frequency": "weekly", "interval": 1 },
        "duration_minutes": 60,
    });
    let sessions = vec![
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 10, 6).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 10, 13).unwrap()),
    ];

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "series_reschedule_template_abc",
                    "variables": {
                        "session_when": "Tuesday, October 6, 2026 at 3:00 PM",
                        "previous_session_when": "Tuesday, September 15, 2026 at 3:00 PM",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_recurring_sessions_rescheduled_email(
        &db,
        &config,
        &series,
        &previous_series,
        &coach,
        &coachee,
        &sessions,
        &org,
    )
    .await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

/// A series with a biweekly rule, starting on the given date.
#[cfg(feature = "mock")]
fn create_test_series_with_rule(rule: serde_json::Value) -> coaching_session_series::Model {
    coaching_session_series::Model {
        rule,
        ..create_test_series()
    }
}

/// The create path states the cadence and, having no previous rule, must not
/// declare `previous_recurrence_summary`.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_scheduled_email_carries_recurrence_summary() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .into_connection();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();
    let series = create_test_series_with_rule(serde_json::json!({
        "start_at": "2026-03-04T15:00:00",
        "recurrence": { "frequency": "weekly", "interval": 2 },
        "duration_minutes": 60,
    }));
    let sessions = vec![
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
    ];

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "recurring_template_xyz",
                    "variables": { "recurrence_summary": "Every 2 weeks" }
                }
            }),
            &ical::Method::Request,
        ))
        .match_request(reject_template_variables(&["previous_recurrence_summary"]))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_recurring_sessions_scheduled_email(
        &db, &config, &series, &coach, &coachee, &sessions, &org,
    )
    .await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

/// A frequency-only reschedule: the previous cadence comes from the OLD rule, so the
/// change is visible even though the start never moved.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_rescheduled_email_previous_recurrence_from_old_rule() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .into_connection();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();

    // Same start, weekly becomes biweekly.
    let previous_series = create_test_series_with_rule(serde_json::json!({
        "start_at": "2026-03-04T15:00:00",
        "recurrence": { "frequency": "weekly", "interval": 1 },
        "duration_minutes": 60,
    }));
    let mut series = create_test_series_with_rule(serde_json::json!({
        "start_at": "2026-03-04T15:00:00",
        "recurrence": { "frequency": "biweekly", "interval": 1 },
        "duration_minutes": 60,
    }));
    series.ical_sequence = 1;
    let sessions = vec![
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
    ];

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "series_reschedule_template_abc",
                    "variables": {
                        "recurrence_summary": "Every 2 weeks",
                        "previous_recurrence_summary": "Weekly",
                        "previous_session_when": "Unchanged",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_recurring_sessions_rescheduled_email(
        &db,
        &config,
        &series,
        &previous_series,
        &coach,
        &coachee,
        &sessions,
        &org,
    )
    .await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

/// A start-only move keeps the cadence, so the previous cadence reads `Unchanged`
/// while the previous start still shows a real date. The two checks are independent.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_rescheduled_email_unchanged_recurrence() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .into_connection();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();

    // Weekly both before and after; only the start moves.
    let previous_series = create_test_series();
    let mut series = create_test_series_with_rule(serde_json::json!({
        "start_at": "2026-10-06T15:00:00",
        "recurrence": { "frequency": "weekly", "interval": 1 },
        "duration_minutes": 60,
    }));
    series.ical_sequence = 1;
    let sessions = vec![
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 10, 6).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 10, 13).unwrap()),
    ];

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": {
                    "id": "series_reschedule_template_abc",
                    "variables": {
                        "recurrence_summary": "Weekly",
                        "previous_recurrence_summary": "Unchanged",
                        "previous_session_when": "Tuesday, September 15, 2026 at 3:00 PM",
                    }
                }
            }),
            &ical::Method::Request,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_recurring_sessions_rescheduled_email(
        &db,
        &config,
        &series,
        &previous_series,
        &coach,
        &coachee,
        &sessions,
        &org,
    )
    .await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

/// A reschedule can legitimately leave zero future sessions. The early return must
/// happen before any lookup, so the connection sees no statements at all.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_notify_recurring_sessions_rescheduled_with_no_sessions_does_nothing() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let series = create_test_series();

    // Zero appended results: any query would panic or error rather than pass.
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    notify_recurring_sessions_rescheduled(&db, &config, &series, PreviousSeries(&series), &[])
        .await;

    assert!(
        db.into_transaction_log().is_empty(),
        "an empty sessions slice must return before any statement runs"
    );
}

// ── Series Cancelled Email Tests ───────────────────────────────────

/// A series cancellation keeps the `UID`, `DTSTART`, and `RRULE` of the invite it
/// supersedes so clients can match it, and carries the next `SEQUENCE`.
#[test]
fn test_build_series_cancel_ics_structure() {
    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let mut first = create_test_session();
    first.date = NaiveDate::from_ymd_opt(2026, 9, 15)
        .unwrap()
        .and_hms_opt(19, 0, 0)
        .unwrap();
    first.duration_minutes = 60;
    let org = create_test_organization();
    let mut series = create_test_series();
    series.ical_sequence = 4;
    let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();

    let ics = build_series_cancel_ics(
        &coach,
        &coachee,
        &first,
        &org,
        &series,
        SERIES_CANCELLED_DESCRIPTION.to_string(),
        dtstamp,
    )
    .unwrap();

    assert!(ics.contains("METHOD:CANCEL"));
    assert!(ics.contains("STATUS:CANCELLED"));
    assert!(ics.contains("SEQUENCE:5"));
    assert!(ics.contains(&format!("UID:{}@myrefactor.com", series.id)));
    assert!(ics.contains("RRULE:FREQ=WEEKLY"));
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_cancelled_email() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    // No session link: the rows are gone by the time a recipient could click one.
    assert!(
        RecurringSessionsCancelled::url_path_template(&config).is_none(),
        "a cancellation must not carry a session URL template"
    );

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();
    let series = create_test_series();

    let sessions = vec![
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
    ];

    let mock_coachee = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "series_cancel_template_abc",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_first_name": "Alex",
                        "other_user_last_name": "Smith",
                        "other_user_role": "coach",
                        "organization_name": "Acme Corp",
                        "session_count": 3,
                        "first_session_date": "Wednesday, March 4, 2026",
                        "last_session_date": "Wednesday, March 18, 2026",
                    }
                }
            }),
            &ical::Method::Cancel,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let mock_coach = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Alex Smith\" <alex@example.com>"],
                "template": {
                    "id": "series_cancel_template_abc",
                    "variables": {
                        "first_name": "Alex",
                        "other_user_first_name": "Jane",
                        "other_user_last_name": "Doe",
                        "other_user_role": "coachee",
                        "organization_name": "Acme Corp",
                        "session_count": 3,
                        "first_session_date": "Wednesday, March 4, 2026",
                        "last_session_date": "Wednesday, March 18, 2026",
                    }
                }
            }),
            &ical::Method::Cancel,
        ))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_recurring_sessions_cancelled_email(
        &config, &series, &coach, &coachee, &sessions, &org,
    )
    .await;
    assert!(result.is_ok());

    // The send swallows errors, so the mock assertions are what give this test teeth.
    mock_coachee.assert_async().await;
    mock_coach.assert_async().await;
}

/// A cancelled series needs no cadence, so neither recurrence variable ships.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_cancelled_email_omits_recurrence_variables() {
    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();
    let series = create_test_series();
    let sessions = vec![
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
        create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()),
    ];

    let mock = server
        .mock("POST", "/emails")
        .match_body(expect_resend_body_with_ics(
            serde_json::json!({
                "template": { "id": "series_cancel_template_abc" }
            }),
            &ical::Method::Cancel,
        ))
        .match_request(reject_template_variables(&[
            "recurrence_summary",
            "previous_recurrence_summary",
        ]))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(2)
        .create_async()
        .await;

    let result = send_recurring_sessions_cancelled_email(
        &config, &series, &coach, &coachee, &sessions, &org,
    )
    .await;
    assert!(result.is_ok());

    mock.assert_async().await;
}

/// The series row is deleted even when nothing upcoming remains. The early return must
/// happen before any lookup, so the connection sees no statements at all.
#[cfg(feature = "mock")]
#[tokio::test]
async fn test_notify_recurring_sessions_cancelled_with_no_sessions_does_nothing() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());
    let series = create_test_series();

    // Zero appended results: any query would panic or error rather than pass.
    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    notify_recurring_sessions_cancelled(&db, &config, &series, &[]).await;

    assert!(
        db.into_transaction_log().is_empty(),
        "an empty sessions slice must return before any statement runs"
    );
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_scheduled_email_single_session() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let mut server = setup_test_server().await;
    let config = create_full_config_with_mock(&server.url());

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
        .into_connection();

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();
    let series = create_test_series();

    let sessions = vec![create_test_session_on(
        NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
    )];

    // With a single session, first and last dates must match.
    let _mock_coachee = server
        .mock("POST", "/emails")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "template": {
                "variables": {
                    "session_count": 1,
                    "first_session_date": "Wednesday, March 4, 2026",
                    "last_session_date": "Wednesday, March 4, 2026",
                }
            }
        })))
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let _mock_coach = server
        .mock("POST", "/emails")
        .with_status(200)
        .with_body(r#"{"id":"email_test"}"#)
        .expect(1)
        .create_async()
        .await;

    let result = send_recurring_sessions_scheduled_email(
        &db, &config, &series, &coach, &coachee, &sessions, &org,
    )
    .await;
    assert!(result.is_ok());
}

#[cfg(feature = "mock")]
#[tokio::test]
async fn test_send_recurring_sessions_scheduled_email_missing_template_id() {
    use sea_orm::{DatabaseBackend, MockDatabase};

    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
    let server = setup_test_server().await;
    let config = Config::from_args([
        "test",
        "--resend-api-key=test_api_key_123",
        &format!("--resend-base-url={}", server.url()),
        "--frontend-base-url=https://app.example.com",
    ]);

    let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let org = create_test_organization();
    let series = create_test_series();
    let sessions = vec![create_test_session_on(
        NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
    )];

    let result = send_recurring_sessions_scheduled_email(
        &db, &config, &series, &coach, &coachee, &sessions, &org,
    )
    .await;

    assert!(result.is_err());
    if let Err(e) = result {
        match e.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Config) => {}
            _ => panic!("Expected Config error, got: {:?}", e.error_kind),
        }
    }
}

#[tokio::test]
async fn test_send_recurring_series_email_to_recipient_empty_sessions_errors() {
    let server = setup_test_server().await;
    let email_config = create_test_email_config(
        &server.url(),
        Some(SessionUrlBuilder {
            base_url: "https://app.example.com".to_string(),
            path_template: "/coaching-sessions/{session_id}".to_string(),
        }),
    )
    .await;

    let recipient = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
    let other = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
    let org = create_test_organization();

    let result = send_recurring_series_email_to_recipient(
        &email_config,
        &recipient,
        &other,
        "coach",
        &[],
        &org,
        "",
        "Weekly",
        None,
    )
    .await;

    assert!(result.is_err());
    if let Err(e) = result {
        match e.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Other(msg)) => {
                assert!(msg.contains("sessions slice is empty"));
            }
            _ => panic!("Expected Internal(Other) error, got: {:?}", e.error_kind),
        }
    }
}

// ── format_session_date_time Unit Tests ────────────────────────────

#[test]
fn test_format_session_date_time_utc() {
    let date = NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap();
    let (date_str, time_str) = format_session_date_time(date, "UTC");
    assert_eq!(date_str, "Wednesday, March 4, 2026");
    assert_eq!(time_str, "3:00 PM");
}

#[test]
fn test_format_session_date_time_eastern() {
    let date = NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap();
    let (date_str, time_str) = format_session_date_time(date, "America/New_York");
    assert_eq!(date_str, "Wednesday, March 4, 2026");
    assert_eq!(time_str, "10:00 AM");
}

#[test]
fn test_format_session_date_time_invalid_timezone_falls_back_to_utc() {
    let date = NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap();
    let (date_str, time_str) = format_session_date_time(date, "Invalid/Timezone");
    assert_eq!(date_str, "Wednesday, March 4, 2026");
    assert_eq!(time_str, "3:00 PM UTC");
}

#[test]
fn test_format_session_date_time_date_rolls_over_with_timezone() {
    // 2026-03-07 23:00 UTC → 2026-03-08 08:00 in Asia/Tokyo (UTC+9)
    let date = NaiveDate::from_ymd_opt(2026, 3, 7)
        .unwrap()
        .and_hms_opt(23, 0, 0)
        .unwrap();
    let (date_str, time_str) = format_session_date_time(date, "Asia/Tokyo");
    assert_eq!(date_str, "Sunday, March 8, 2026");
    assert_eq!(time_str, "8:00 AM");
}
