//! OAuth 2.0 authentication infrastructure.
//!
//! Provides OAuth 2.0 authorization flows with PKCE security for video meeting platforms.

mod pkce;
mod provider;
mod state;

pub mod providers;
pub mod token;

pub use pkce::{PkceChallenge, PkceVerifier};
pub use provider::{AuthorizationRequest, Provider, ProviderKind, UserInfo};
pub use state::StateManager;
