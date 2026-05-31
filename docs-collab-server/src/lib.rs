//! Self-hosted collaborative document server.
//!
//! Speaks the Hocuspocus binary wire protocol over WebSockets and persists Yjs
//! document state to PostgreSQL. The protocol layer is built directly on the
//! generic `yrs` crate, not on `yrs-warp`.

pub mod auth;
pub mod config;
pub mod document;
pub mod protocol;
pub mod registry;
pub mod storage;

#[cfg(test)]
mod test_support;

pub use auth::{AuthError, Authenticator, JwtAuthenticator, Scope};
pub use config::Config;
pub use document::{ConnectionId, Document};
pub use protocol::{Body, Frame, ProtocolError};
pub use registry::DocumentRegistry;
pub use storage::{MemoryStorage, PostgresStorage, Storage, StorageError};
