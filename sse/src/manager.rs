use crate::connection::{ConnectionId, ConnectionRegistry, UserId};
use crate::message::{EventType, Message as SseMessage, MessageScope};
use axum::response::sse::Event;
use log::*;
use std::sync::Arc;

pub struct Manager {
    registry: Arc<ConnectionRegistry>,
}

impl Manager {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(ConnectionRegistry::new()),
        }
    }

    /// Register a new connection and return its unique ID
    pub fn register_connection(
        &self,
        user_id: UserId,
        sender: tokio::sync::mpsc::UnboundedSender<Result<Event, std::convert::Infallible>>,
    ) -> ConnectionId {
        let connection_id = self.registry.register(user_id.clone(), sender);
        info!("Registered new SSE connection");
        connection_id
    }

    /// Unregister a connection by ID
    pub fn unregister_connection(&self, connection_id: &ConnectionId) {
        info!("Unregistering SSE connection");
        self.registry.unregister(connection_id);
    }

    /// Send a message based on its scope
    pub fn send_message(&self, message: SseMessage) {
        let event_type = message.event.event_type();

        let event_data = match serde_json::to_string(&message.event) {
            Ok(json) => json,
            Err(e) => {
                error!("Failed to serialize SSE event: {e}");
                return;
            }
        };

        let event = Event::default().event(event_type).data(event_data);

        match message.scope {
            MessageScope::User { user_id } => {
                self.registry.send_to_user(&user_id, event);
            }
            MessageScope::Broadcast => {
                self.registry.broadcast(event);
            }
        }
    }
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}
