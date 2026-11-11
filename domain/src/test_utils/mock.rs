//! Helpers for mock database tests: a recording event publisher and membership fixtures.

use crate::coaching_relationships;
use crate::events::{DomainEvent, EventPublisher};
use crate::Id;
use async_trait::async_trait;
use events::EventHandler;
use mockito::{Matcher, Mock, ServerGuard};
use sea_orm::{Transaction, Value};
use service::config::Config;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Captures every published event, in order, for assertion.
struct RecordingHandler {
    events: Arc<Mutex<Vec<DomainEvent>>>,
}

#[async_trait]
impl EventHandler for RecordingHandler {
    async fn handle(&self, event: &DomainEvent) {
        self.events.lock().unwrap().push(event.clone());
    }
}

/// Builds an `EventPublisher` wired to a recording handler, returning the
/// publisher plus a shared handle to the events it captures.
pub(crate) fn recording_publisher() -> (EventPublisher, Arc<Mutex<Vec<DomainEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(RecordingHandler {
        events: events.clone(),
    });
    (EventPublisher::new().with_handler(handler), events)
}

/// Mock rows for the organization-membership filter, keeping both participants.
///
/// Every notify-set builder runs this filter after loading the relationship, so
/// mocks feeding those paths need one appended result per participant.
pub(crate) fn both_participants_are_members(
    relationship: &coaching_relationships::Model,
) -> Vec<BTreeMap<String, Value>> {
    members(&[relationship.coach_id, relationship.coachee_id])
}

/// Mock rows for the organization-membership filter, keeping exactly `user_ids`.
pub(crate) fn members(user_ids: &[Id]) -> Vec<BTreeMap<String, Value>> {
    user_ids
        .iter()
        .map(|&user_id| BTreeMap::from([("user_id".to_string(), user_id.into())]))
        .collect()
}

/// How many logged statements check membership, i.e. select from `user_roles`.
///
/// A user load that joins in its roles is not a membership check, so it does not count.
pub(crate) fn membership_statements(log: &[Transaction]) -> usize {
    log.iter()
        .flat_map(Transaction::statements)
        .filter(|statement| {
            statement
                .sql
                .contains(r#"FROM "refactor_platform"."user_roles""#)
        })
        .count()
}

/// One mock server standing in for Tiptap and Resend, recording who each email went to.
pub(crate) struct Mailbox {
    server: ServerGuard,
    sent: Arc<Mutex<Vec<String>>>,
    _mocks: [Mock; 2],
}

impl Mailbox {
    pub(crate) async fn new() -> Self {
        let mut server = mockito::Server::new_async().await;
        let sent = Arc::new(Mutex::new(Vec::new()));
        let seen = sent.clone();
        let resend = server
            .mock("POST", "/emails")
            .match_request(move |request| {
                let to = request
                    .body()
                    .ok()
                    .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
                    .and_then(|body| body["to"][0].as_str().map(String::from))
                    .unwrap_or_default();
                seen.lock().unwrap().push(to);
                true
            })
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .create_async()
            .await;
        let tiptap = server
            .mock("POST", Matcher::Regex("^/api/documents/".to_string()))
            .with_status(200)
            .create_async()
            .await;
        Self {
            server,
            sent,
            _mocks: [resend, tiptap],
        }
    }

    /// A config sending every session and series email, and Tiptap calls, here.
    pub(crate) fn config(&self) -> Config {
        let url = self.server.url();
        Config::from_args([
            "test".to_string(),
            "--resend-api-key=test_api_key".to_string(),
            "--session-scheduled-email-template-id=session_template".to_string(),
            "--recurring-sessions-scheduled-email-template-id=recurring_template".to_string(),
            "--recurring-sessions-rescheduled-email-template-id=series_reschedule_template"
                .to_string(),
            "--frontend-base-url=https://app.example.com".to_string(),
            "--tiptap-auth-key=test-auth-key".to_string(),
            format!("--tiptap-url={url}"),
            format!("--resend-base-url={url}"),
        ])
    }

    /// The `to` address of every email sent so far.
    pub(crate) fn recipients(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }

    /// Asserts at least one email went out and every one went to `email` alone.
    pub(crate) fn assert_only_to(&self, email: &str) {
        let recipients = self.recipients();
        assert!(!recipients.is_empty(), "nothing was sent");
        assert!(
            recipients.iter().all(|to| to.contains(email)),
            "only {email} may be mailed: {recipients:?}"
        );
    }
}

/// A user with `email`, for fixtures whose recipients must be told apart.
pub(crate) fn user_with_email(email: &str) -> crate::users::Model {
    let now = chrono::Utc::now();
    crate::users::Model {
        id: Id::new_v4(),
        email: email.to_string(),
        first_name: email.to_string(),
        last_name: "Person".to_string(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: 60,
        working_hours_start: crate::working_hours::default_start(),
        working_hours_end: crate::working_hours::default_end(),
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}
