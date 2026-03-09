//! API key authentication for service providers.
//!
//! Provides traits and implementations for authenticating requests to services
//! that use API keys (Recall.ai, AssemblyAI, Deepgram, etc.).

mod auth;
mod bearer;

pub use auth::{Auth, AuthMethod, Authenticate, Provider};
pub use bearer::Auth as BearerAuth;
