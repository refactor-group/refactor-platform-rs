use axum::response::sse::Event;
use dashmap::DashMap;
use log::*;
use std::collections::HashSet;
use std::convert::Infallible;
use tokio::sync::mpsc::UnboundedSender;

// Type alias for user IDs (web layer converts domain::Id to String)
pub type UserId = String;

/// Unique identifier for a connection (server-generated)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectionId(String);

impl ConnectionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Connection information (no redundant connection_id)
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub user_id: UserId,
    pub sender: UnboundedSender<Result<Event, Infallible>>,
}

/// High-performance connection registry with dual indices for O(1) lookups
pub struct ConnectionRegistry {
    /// Primary storage: lookup by connection_id for registration/cleanup - O(1)
    connections: DashMap<ConnectionId, ConnectionInfo>,

    /// Secondary index: fast lookup by user_id for message routing - O(1)
    user_index: DashMap<UserId, HashSet<ConnectionId>>,
}

impl ConnectionRegistry {
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            user_index: DashMap::new(),
        }
    }

    /// Register a new connection - O(1)
    pub fn register(
        &self,
        user_id: UserId,
        sender: UnboundedSender<Result<Event, Infallible>>,
    ) -> ConnectionId {
        let connection_id = ConnectionId::new();

        // Insert into primary storage
        self.connections.insert(
            connection_id.clone(),
            ConnectionInfo {
                user_id: user_id.clone(),
                sender,
            },
        );

        // Update secondary index
        self.user_index
            .entry(user_id)
            .or_default()
            .insert(connection_id.clone());

        connection_id
    }

    /// Unregister a connection - O(1)
    pub fn unregister(&self, connection_id: &ConnectionId) {
        // Remove from primary storage
        if let Some((_, info)) = self.connections.remove(connection_id) {
            let user_id = info.user_id;

            // Update secondary index
            if let Some(mut entry) = self.user_index.get_mut(&user_id) {
                entry.remove(connection_id);

                // Clean up empty user entries
                if entry.is_empty() {
                    drop(entry); // Release lock before removal
                    self.user_index.remove(&user_id);
                }
            }
        }
    }

    /// Send message to specific user - O(1) lookup + O(k) send where k = user's connections
    pub fn send_to_user(&self, user_id: &UserId, event: Event) {
        if let Some(connection_ids) = self.user_index.get(user_id) {
            for conn_id in connection_ids.iter() {
                if let Some(info) = self.connections.get(conn_id) {
                    if let Err(e) = info.sender.send(Ok(event.clone())) {
                        warn!(
                            "Failed to send event to connection {}: {}. Connection will be cleaned up.",
                            conn_id.as_str(),
                            e
                        );
                    }
                }
            }
        }
    }

    /// Broadcast message to all connections - O(n) (unavoidable, but explicit)
    pub fn broadcast(&self, event: Event) {
        for entry in self.connections.iter() {
            if let Err(e) = entry.value().sender.send(Ok(event.clone())) {
                warn!(
                    "Failed to send broadcast to connection {}: {}",
                    entry.key().as_str(),
                    e
                );
            }
        }
    }
}

impl Default for ConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}
