//! SSE HTTP handler for the web layer.
//!
//! This module contains only the Axum handler for SSE endpoints.
//! The core SSE infrastructure (Manager, ConnectionRegistry, Message types)
//! lives in the `sse` crate to avoid circular dependencies.

pub mod handler;
