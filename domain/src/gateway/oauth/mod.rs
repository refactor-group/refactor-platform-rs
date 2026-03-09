//! OAuth authentication gateway.
//!
//! Re-exports OAuth types from meeting-auth and provides provider-specific clients.

pub mod google;

// Re-export OAuth types from meeting-auth
pub use meeting_auth::oauth::{
    token::{Plain, RefreshResult, Tokens},
    AuthorizationRequest, Kind, Provider, UserInfo,
};
