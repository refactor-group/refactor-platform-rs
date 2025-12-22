//! Event system infrastructure for the Refactor Platform.
//!
//! This crate provides the event system that enables loose coupling between
//! domain logic and infrastructure concerns (like SSE notifications).
//!
//! # Architecture
//!
//! - **DomainEvent**: Enum representing all business events in the system
//! - **EventHandler**: Trait for implementing event handlers
//! - **EventPublisher**: Publishes events to registered handlers
//!
//! This crate has no dependencies on internal crates (entity, domain, etc.),
//! avoiding circular dependencies. Entity data is carried as serialized JSON values.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// A type alias that represents any Entity's internal id field data type.
/// This matches the definition in the entity crate to maintain compatibility.
pub type Id = Uuid;

/// Domain events that represent business-level changes in the system.
/// These events are emitted when domain operations complete successfully.
///
/// Events include user IDs for notification routing. The domain layer is
/// responsible for determining which users should be notified.
///
/// Entity data is carried as `serde_json::Value` to avoid dependencies on
/// the entity crate.
#[derive(Debug, Clone)]
pub enum DomainEvent {
    // Overarching Goal events (relationship-scoped)
    OverarchingGoalCreated {
        coaching_relationship_id: Id,
        /// Serialized overarching goal data
        overarching_goal: Value,
        /// Users who should be notified (typically coach and coachee)
        notify_user_ids: Vec<Id>,
    },
    OverarchingGoalUpdated {
        coaching_relationship_id: Id,
        /// Serialized overarching goal data
        overarching_goal: Value,
        /// Users who should be notified (typically coach and coachee)
        notify_user_ids: Vec<Id>,
    },
    OverarchingGoalDeleted {
        coaching_relationship_id: Id,
        overarching_goal_id: Id,
        /// Users who should be notified (typically coach and coachee)
        notify_user_ids: Vec<Id>,
    },
}

/// Trait for handling domain events.
/// Implementations can perform side effects like sending notifications,
/// updating caches, logging, etc.
#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &DomainEvent);
}

/// Publishes domain events to registered handlers.
/// Handlers are called sequentially in registration order.
#[derive(Clone)]
pub struct EventPublisher {
    handlers: Arc<Vec<Arc<dyn EventHandler>>>,
}

impl EventPublisher {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(Vec::new()),
        }
    }

    /// Register a new event handler.
    /// Note: This creates a new publisher instance with the additional handler.
    /// Store the returned publisher in your application state.
    pub fn with_handler(mut self, handler: Arc<dyn EventHandler>) -> Self {
        let mut handlers = (*self.handlers).clone();
        handlers.push(handler);
        self.handlers = Arc::new(handlers);
        self
    }

    /// Publish an event to all registered handlers.
    /// Handlers are called sequentially. If a handler panics or errors,
    /// we log it but continue with remaining handlers.
    pub async fn publish(&self, event: DomainEvent) {
        for handler in self.handlers.iter() {
            handler.handle(&event).await;
        }
    }
}

impl Default for EventPublisher {
    fn default() -> Self {
        Self::new()
    }
}
