use crate::message::{Event as SseEvent, Message as SseMessage, MessageScope};
use crate::Manager;
use async_trait::async_trait;
use events::{DomainEvent, EventHandler};
use log::*;
use std::sync::Arc;

/// Handles domain events by converting them to SSE messages and broadcasting to affected users.
///
/// This handler is responsible for:
/// 1. Converting domain events into SSE events
/// 2. Sending SSE messages to the user IDs specified in the event
///
/// The domain layer determines which users should be notified and includes
/// their IDs in the event. This handler simply routes the SSE messages.
pub struct SseDomainEventHandler {
    sse_manager: Arc<Manager>,
}

impl SseDomainEventHandler {
    pub fn new(sse_manager: Arc<Manager>) -> Self {
        Self { sse_manager }
    }

    /// Send an SSE message to all specified users.
    fn send_to_users(&self, sse_event: SseEvent, user_ids: &[events::Id]) {
        for user_id in user_ids {
            self.sse_manager.send_message(SseMessage {
                event: sse_event.clone(),
                scope: MessageScope::User {
                    user_id: user_id.to_string(),
                },
            });
        }

        debug!(
            "Sent SSE event to {} user(s): {:?}",
            user_ids.len(),
            user_ids
        );
    }
}

#[async_trait]
impl EventHandler for SseDomainEventHandler {
    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::OverarchingGoalCreated {
                coaching_relationship_id,
                overarching_goal,
                notify_user_ids,
            } => {
                debug!(
                    "Handling OverarchingGoalCreated event for relationship {}",
                    coaching_relationship_id
                );

                let sse_event = SseEvent::GoalCreated {
                    coaching_relationship_id: coaching_relationship_id.to_string(),
                    goal: overarching_goal.clone(),
                };

                self.send_to_users(sse_event, notify_user_ids);
            }

            DomainEvent::OverarchingGoalUpdated {
                coaching_relationship_id,
                overarching_goal,
                notify_user_ids,
            } => {
                debug!(
                    "Handling OverarchingGoalUpdated event for relationship {}",
                    coaching_relationship_id
                );

                let sse_event = SseEvent::GoalUpdated {
                    coaching_relationship_id: coaching_relationship_id.to_string(),
                    goal: overarching_goal.clone(),
                };

                self.send_to_users(sse_event, notify_user_ids);
            }

            DomainEvent::OverarchingGoalDeleted {
                coaching_relationship_id,
                overarching_goal_id,
                notify_user_ids,
            } => {
                debug!(
                    "Handling OverarchingGoalDeleted event for goal {}",
                    overarching_goal_id
                );

                let sse_event = SseEvent::GoalDeleted {
                    coaching_relationship_id: coaching_relationship_id.to_string(),
                    goal_id: overarching_goal_id.to_string(),
                };

                self.send_to_users(sse_event, notify_user_ids);
            }
        }
    }
}
