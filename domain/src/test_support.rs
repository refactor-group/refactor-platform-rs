//! Shared test-only helpers for the domain crate.

use crate::events::{DomainEvent, EventPublisher};
use async_trait::async_trait;
use events::EventHandler;
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
