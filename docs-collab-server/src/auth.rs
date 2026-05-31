//! Connection authentication.
//!
//! Verifies a JWT and a wildcard document-name claim of the form
//! `{org_slug}.{relationship_slug}.*`.

use async_trait::async_trait;
use jsonwebtoken::{decode, errors::ErrorKind, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
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
    signing_key: Vec<u8>,
}

impl JwtAuthenticator {
    pub fn new(signing_key: impl Into<Vec<u8>>) -> Self {
        Self {
            signing_key: signing_key.into(),
        }
    }
}

/// Only the fields this server enforces. `exp` lets jsonwebtoken reject expired
/// tokens; the wildcard list drives the per-document authorization check. Other
/// claims (iss, sub, aud, iat, ...) are accepted but ignored.
#[derive(Debug, Deserialize)]
struct Claims {
    #[allow(dead_code)] // read by jsonwebtoken's `Validation`, not by us.
    exp: usize,
    #[serde(rename = "allowedDocumentNames")]
    allowed_document_names: Vec<String>,
}

#[async_trait]
impl Authenticator for JwtAuthenticator {
    async fn authenticate(&self, token: &str, doc_name: &str) -> Result<Scope, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // Upstream tokens carry an `aud` we do not pin here; signature + exp
        // are the trust anchors. Re-enabling this rejects every real token.
        validation.validate_aud = false;

        let claims = decode::<Claims>(
            token,
            &DecodingKey::from_secret(&self.signing_key),
            &validation,
        )
        .map_err(|e| match e.kind() {
            ErrorKind::ExpiredSignature => AuthError::Expired,
            _ => AuthError::InvalidToken(e.to_string()),
        })?
        .claims;

        claims
            .allowed_document_names
            .iter()
            .find(|entry| {
                entry
                    .strip_suffix('*')
                    .is_some_and(|prefix| doc_name.starts_with(prefix))
            })
            .map(|entry| Scope {
                allowed_prefix: entry.clone(),
            })
            .ok_or_else(|| AuthError::ForbiddenDoc {
                name: doc_name.into(),
            })
    }
}
