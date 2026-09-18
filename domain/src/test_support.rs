//! Shared test-only helpers for the domain crate.

use crate::coaching_relationships;
use crate::events::{DomainEvent, EventPublisher};
use async_trait::async_trait;
use events::EventHandler;
use sea_orm::Value;
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
    [relationship.coach_id, relationship.coachee_id]
        .into_iter()
        .map(|user_id| BTreeMap::from([("user_id".to_string(), user_id.into())]))
        .collect()
}
