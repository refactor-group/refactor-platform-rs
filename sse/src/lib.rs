//! Server-Sent Events (SSE) infrastructure for real-time updates.
//!
//! This crate provides a type-safe, app-wide SSE implementation for pushing
//! real-time updates from the backend to authenticated users.
//!
//! # Architecture
//!
//! - **Single connection per user**: Each authenticated user establishes one
//!   SSE connection that stays open across page navigation.
//! - **Dual-index registry**: O(1) lookups for both connection management and
//!   user-scoped message routing via separate DashMap indices.
//! - **User and Broadcast scopes**: Messages can be sent to specific users or
//!   broadcast to all connected users.
//! - **Ephemeral messages**: All events are ephemeral - if a user is offline,
//!   they miss the event and see fresh data on next page load.
//! - **Type-safe events**: All event types are strongly typed for compile-time
//!   safety and better frontend TypeScript integration.
//!
//! # Message Flow
//!
//! 1. Frontend establishes SSE connection via `/sse` endpoint
//! 2. Backend extracts user from session cookie (AuthenticatedUser)
//! 3. Connection registered in ConnectionRegistry with dual indices
//! 4. When a resource changes (e.g., action created):
//!    - Controller determines recipient (e.g., other user in relationship)
//!    - Controller sends message via `app_state.sse_manager.send_message()`
//!    - Manager performs O(1) lookup in user_index to find connections
//!    - Events sent only to matching connections
//! 5. Frontend receives event and updates UI based on context
//!
//! # Example: Sending an event
//!
//! ```rust,ignore
//! use sse::message::{Event as SseEvent, Message as SseMessage, MessageScope};
//!
//! // In a controller after creating an action
//! app_state.sse_manager.send_message(SseMessage {
//!     event: SseEvent::ActionCreated {
//!         coaching_session_id,
//!         action: action.clone(),
//!     },
//!     scope: MessageScope::User { user_id: recipient_id },
//! });
//! ```
//!
//! # Security Considerations
//!
//! - Authentication required (AuthenticatedUser extractor)
//! - Session cookie must be valid
//! - Backend determines recipients (not client-controlled)
//! - nginx configured for long-lived connections (24h timeout)
//! - Keep-alive messages prevent idle timeout
//!
//! # Modules
//!
//! - `connection`: ConnectionRegistry with dual-index architecture and type-safe ConnectionId
//! - `manager`: High-level message routing (delegates to ConnectionRegistry)
//! - `message`: Type-safe event and scope definitions

pub mod connection;
pub mod manager;
pub mod message;

pub use manager::Manager;
