//! Connection authentication.
//!
//! Verifies a JWT and a wildcard document-name claim of the form
//! `{org_slug}.{relationship_slug}.*`.

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("token rejected: {0}")]
    InvalidToken(String),
    #[error("document name {name} not permitted by scope")]
    ForbiddenDoc { name: String },
    #[error("token expired")]
    Expired,
}

/// Scope granted by a verified token: the wildcard prefix the connection is
/// allowed to open documents under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub allowed_prefix: String,
}

/// Validates a bearer token against a target document name.
#[async_trait]
pub trait Authenticator: Send + Sync + 'static {
    async fn authenticate(&self, token: &str, doc_name: &str) -> Result<Scope, AuthError>;
}

/// HS256 JWT authenticator. Disables audience validation (the upstream token
/// carries an `aud` the server does not configure).
pub struct JwtAuthenticator {
    _signing_key: Vec<u8>,
}

impl JwtAuthenticator {
    pub fn new(signing_key: impl Into<Vec<u8>>) -> Self {
        Self {
            _signing_key: signing_key.into(),
        }
    }
}

#[async_trait]
impl Authenticator for JwtAuthenticator {
    async fn authenticate(&self, _token: &str, _doc_name: &str) -> Result<Scope, AuthError> {
        todo!("JwtAuthenticator::authenticate in Phase 6")
    }
}
