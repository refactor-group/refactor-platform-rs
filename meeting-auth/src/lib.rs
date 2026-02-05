//! # meeting-auth
//!
//! Single source of truth for ALL authentication in the meeting platform:
//! - API key authentication for service providers (Recall.ai, AssemblyAI, etc.)
//! - OAuth 2.0 infrastructure (tokens, storage, refresh, PKCE)
//! - OAuth provider implementations (Google, Zoom, Microsoft)
//! - HTTP client building with middleware
//! - Webhook signature validation
//!
//! ## Architecture
//!
//! This crate provides the authentication foundation that other crates build upon:
//! - `meeting-manager` uses OAuth providers and token management for meeting APIs
//! - `meeting-ai` adapters use API key auth and HTTP client builder for AI services
//!
//! ## Usage
//!
//! ```rust,ignore
//! use meeting_auth::{
//!     api_key::{ApiKeyAuth, ProviderAuth},
//!     oauth::{OAuthProvider, token::{Manager, Storage}},
//!     http::AuthenticatedClientBuilder,
//! };
//! ```

pub mod api_key;
pub mod credentials;
pub mod error;
pub mod http;
pub mod oauth;
pub mod providers;
pub mod webhook;

// Re-export commonly used types
pub use error::{Error, ErrorKind};
