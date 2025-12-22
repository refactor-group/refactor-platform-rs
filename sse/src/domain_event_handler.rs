use crate::message::{Event as SseEvent, EventType, Message as SseMessage, MessageScope};
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
        let event_type = sse_event.event_type();

        for user_id in user_ids {
            self.sse_manager.send_message(SseMessage {
                event: sse_event.clone(),
                scope: MessageScope::User {
                    user_id: user_id.to_string(),
                },
            });
        }

        info!("Sent {} event to {} user(s)", event_type, user_ids.len());
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
                let sse_event = SseEvent::OverarchingGoalCreated {
                    coaching_relationship_id: coaching_relationship_id.to_string(),
                    overarching_goal: overarching_goal.clone(),
                };

                self.send_to_users(sse_event, notify_user_ids);
            }

            DomainEvent::OverarchingGoalUpdated {
                coaching_relationship_id,
                overarching_goal,
                notify_user_ids,
            } => {
                let sse_event = SseEvent::OverarchingGoalUpdated {
                    coaching_relationship_id: coaching_relationship_id.to_string(),
                    overarching_goal: overarching_goal.clone(),
                };

                self.send_to_users(sse_event, notify_user_ids);
            }

            DomainEvent::OverarchingGoalDeleted {
                coaching_relationship_id,
                overarching_goal_id,
                notify_user_ids,
            } => {
                let sse_event = SseEvent::OverarchingGoalDeleted {
                    coaching_relationship_id: coaching_relationship_id.to_string(),
                    overarching_goal_id: overarching_goal_id.to_string(),
                };

                self.send_to_users(sse_event, notify_user_ids);
            }
        }
    }
}
