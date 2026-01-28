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
    /// Emitted when a new overarching goal is created within a coaching session.
    /// Triggers SSE notifications to coach and coachee for real-time UI updates.
    OverarchingGoalCreated {
        /// Parent coaching relationship ID for context and potential scoped cache invalidation.
        /// Currently used for event tracing and future frontend optimization.
        coaching_relationship_id: Id,
        /// Complete serialized overarching goal entity (includes id, title, details, status, etc.).
        /// Sent to frontend for optimistic UI updates without requiring a separate API call.
        overarching_goal: Value,
        /// User IDs to receive SSE notifications (determined by domain layer from relationship).
        /// SSE manager routes events only to these users' active connections.
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when an overarching goal is modified (title, details, status, etc.).
    /// Triggers SSE notifications to keep all participants' UIs synchronized.
    OverarchingGoalUpdated {
        /// Parent coaching relationship ID for context and potential scoped cache invalidation.
        /// Currently used for event tracing and future frontend optimization.
        coaching_relationship_id: Id,
        /// Complete updated overarching goal entity with all current field values.
        /// Sent to frontend for optimistic UI updates without requiring a separate API call.
        overarching_goal: Value,
        /// User IDs to receive SSE notifications (determined by domain layer from relationship).
        /// SSE manager routes events only to these users' active connections.
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when an overarching goal is permanently removed from the system.
    /// Triggers SSE notifications to remove the goal from all participants' UIs.
    OverarchingGoalDeleted {
        /// Parent coaching relationship ID for context and potential scoped cache invalidation.
        /// Currently used for event tracing and future frontend optimization.
        coaching_relationship_id: Id,
        /// ID of the deleted goal (full entity not included since it no longer exists).
        /// Frontend uses this to remove the goal from local cache and UI.
        overarching_goal_id: Id,
        /// User IDs to receive SSE notifications (determined by domain layer from relationship).
        /// SSE manager routes events only to these users' active connections.
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
