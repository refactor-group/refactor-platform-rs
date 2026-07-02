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
    /// Emitted when a new goal is created within a coaching session.
    /// Triggers SSE notifications to coach and coachee for real-time UI updates.
    GoalCreated {
        /// Parent coaching relationship ID for context and potential scoped cache invalidation.
        /// Currently used for event tracing and future frontend optimization.
        coaching_relationship_id: Id,
        /// Complete serialized goal entity (includes id, title, details, status, etc.).
        /// Sent to frontend for optimistic UI updates without requiring a separate API call.
        goal: Value,
        /// User IDs to receive SSE notifications (determined by domain layer from relationship).
        /// SSE manager routes events only to these users' active connections.
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when a goal is modified (title, details, status, etc.).
    /// Triggers SSE notifications to keep all participants' UIs synchronized.
    GoalUpdated {
        /// Parent coaching relationship ID for context and potential scoped cache invalidation.
        /// Currently used for event tracing and future frontend optimization.
        coaching_relationship_id: Id,
        /// Complete updated goal entity with all current field values.
        /// Sent to frontend for optimistic UI updates without requiring a separate API call.
        goal: Value,
        /// User IDs to receive SSE notifications (determined by domain layer from relationship).
        /// SSE manager routes events only to these users' active connections.
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when a goal is permanently removed from the system.
    /// Triggers SSE notifications to remove the goal from all participants' UIs.
    GoalDeleted {
        /// Parent coaching relationship ID for context and potential scoped cache invalidation.
        /// Currently used for event tracing and future frontend optimization.
        coaching_relationship_id: Id,
        /// ID of the deleted goal (full entity not included since it no longer exists).
        /// Frontend uses this to remove the goal from local cache and UI.
        goal_id: Id,
        /// User IDs to receive SSE notifications (determined by domain layer from relationship).
        /// SSE manager routes events only to these users' active connections.
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when a goal is linked to a coaching session via the join table.
    /// Triggers SSE notifications so participants see updated session-goal associations.
    CoachingSessionGoalCreated {
        /// Parent coaching relationship ID for scoped event routing.
        coaching_relationship_id: Id,
        /// The coaching session that the goal was linked to.
        coaching_session_id: Id,
        /// The goal that was linked to the session.
        goal_id: Id,
        /// User IDs to receive SSE notifications (coach + coachee from relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when a goal is unlinked from a coaching session.
    /// Triggers SSE notifications so participants see the removed association.
    CoachingSessionGoalDeleted {
        /// Parent coaching relationship ID for scoped event routing.
        coaching_relationship_id: Id,
        /// The coaching session that the goal was unlinked from.
        coaching_session_id: Id,
        /// The goal that was unlinked from the session.
        goal_id: Id,
        /// User IDs to receive SSE notifications (coach + coachee from relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when an agreement is created within a coaching session.
    /// Carries the full serialized agreement entity for optimistic UI updates.
    AgreementCreated {
        /// The coaching session the agreement belongs to.
        coaching_session_id: Id,
        /// Complete serialized agreement entity for the frontend cache.
        agreement: Value,
        /// User IDs to receive SSE notifications (coach + coachee from the session's relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when an agreement is modified.
    AgreementUpdated {
        /// The coaching session the agreement belongs to.
        coaching_session_id: Id,
        /// Complete updated agreement entity for the frontend cache.
        agreement: Value,
        /// User IDs to receive SSE notifications (coach + coachee from the session's relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when an agreement is removed.
    AgreementDeleted {
        /// The coaching session the agreement belonged to.
        coaching_session_id: Id,
        /// ID of the deleted agreement (full entity not included since it no longer exists).
        agreement_id: Id,
        /// User IDs to receive SSE notifications (coach + coachee from the session's relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when an action is created within a coaching session.
    /// Carries the full serialized action (with assignees) for optimistic UI updates.
    ActionCreated {
        /// The coaching session the action belongs to.
        coaching_session_id: Id,
        /// Complete serialized action (with assignees) for the frontend cache.
        action: Value,
        /// User IDs to receive SSE notifications (coach + coachee from the session's relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when an action is modified (body, status, due date, assignees, etc.).
    ActionUpdated {
        /// The coaching session the action belongs to.
        coaching_session_id: Id,
        /// Complete updated action (with assignees) for the frontend cache.
        action: Value,
        /// User IDs to receive SSE notifications (coach + coachee from the session's relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when an action is removed.
    ActionDeleted {
        /// The coaching session the action belonged to.
        coaching_session_id: Id,
        /// ID of the deleted action (full entity not included since it no longer exists).
        action_id: Id,
        /// User IDs to receive SSE notifications (coach + coachee from the session's relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when a meeting recording status changes (any webhook-driven transition).
    /// Triggers SSE notifications so participants see the current recording state without polling.
    MeetingRecordingUpdated {
        /// The coaching session whose recording changed.
        coaching_session_id: Id,
        /// User IDs to receive SSE notifications (coach + coachee from coaching relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted on ANY topic mutation (add/edit/delete/reorder/rate). Coarse: carries no
    /// entity — participants refetch the session's topics. Triggers SSE to coach + coachee.
    TopicsChanged {
        /// The coaching session whose topics changed.
        coaching_session_id: Id,
        /// User IDs to receive SSE notifications (coach + coachee from the relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when a coaching session's title is set/changed via the title endpoint.
    /// Coarse: carries no entity — participants refetch the session. Triggers SSE to coach + coachee.
    CoachingSessionTitleUpdated {
        /// The coaching session whose title changed.
        coaching_session_id: Id,
        /// User IDs to receive SSE notifications (coach + coachee from the relationship).
        notify_user_ids: Vec<Id>,
    },
    /// Emitted when a transcription status changes (created, completed, or failed).
    /// Triggers SSE notifications so participants see the current transcription state without polling.
    TranscriptionUpdated {
        /// The coaching session whose transcription changed.
        coaching_session_id: Id,
        /// User IDs to receive SSE notifications (coach + coachee from coaching relationship).
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
