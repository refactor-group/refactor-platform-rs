//! Error types for the `meeting-auth` crate.
//!
//! Follows the same pattern as domain::error with a root Error struct and error kind enums.

use std::error::Error as StdError;
use std::fmt;

/// Top-level error type for meeting-auth crate.
/// Holds error kind and optional source for error chaining.
#[derive(Debug)]
pub struct Error {
    pub source: Option<Box<dyn StdError + Send + Sync>>,
    pub error_kind: ErrorKind,
}

/// Major categories of errors in meeting-auth.
#[derive(Debug, PartialEq)]
pub enum ErrorKind {
    ApiKey(ApiKeyErrorKind),
    OAuth(OAuthErrorKind),
    Token(TokenErrorKind),
    Credential(CredentialErrorKind),
    Webhook(WebhookErrorKind),
    Http(HttpErrorKind),
}

/// Errors from API key authentication operations.
#[derive(Debug, PartialEq)]
pub enum ApiKeyErrorKind {
    InvalidFormat,
    VerificationFailed,
    NotFound,
    Network,
}

/// Errors from OAuth operations.
#[derive(Debug, PartialEq)]
pub enum OAuthErrorKind {
    AuthorizationFailed,
    TokenExchangeFailed,
    TokenRefreshFailed,
    RevocationFailed,
    InvalidState,
    PkceVerificationFailed,
    Network,
    InvalidResponse,
}

/// Errors from token management operations.
#[derive(Debug, PartialEq)]
pub enum TokenErrorKind {
    NotFound,
    Expired,
    Storage,
    Refresh,
}

/// Errors from token storage operations.
#[derive(Debug, PartialEq)]
pub enum StorageErrorKind {
    NotFound,
    EncryptionFailed,
    DecryptionFailed,
    AtomicUpdateFailed,
    Database,
}

/// Errors from credential storage operations.
#[derive(Debug, PartialEq)]
pub enum CredentialErrorKind {
    NotFound,
    EncryptionFailed,
    DecryptionFailed,
    StorageFailed,
}

/// Errors from webhook validation.
#[derive(Debug, PartialEq)]
pub enum WebhookErrorKind {
    InvalidSignature,
    MissingSignature,
    TimestampExpired,
    InvalidPayload,
}

/// Errors from HTTP client operations.
#[derive(Debug, PartialEq)]
pub enum HttpErrorKind {
    BuilderFailed,
    RequestFailed,
    Network,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.error_kind {
            ErrorKind::ApiKey(kind) => write!(f, "API key error: {:?}", kind),
            ErrorKind::OAuth(kind) => write!(f, "OAuth error: {:?}", kind),
            ErrorKind::Token(kind) => write!(f, "Token error: {:?}", kind),
            ErrorKind::Credential(kind) => write!(f, "Credential error: {:?}", kind),
            ErrorKind::Webhook(kind) => write!(f, "Webhook error: {:?}", kind),
            ErrorKind::Http(kind) => write!(f, "HTTP error: {:?}", kind),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn StdError + 'static))
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        let error_kind = if err.is_builder() {
            ErrorKind::Http(HttpErrorKind::BuilderFailed)
        } else if err.is_request() {
            ErrorKind::Http(HttpErrorKind::RequestFailed)
        } else {
            ErrorKind::Http(HttpErrorKind::Network)
        };

        Error {
            source: Some(Box::new(err)),
            error_kind,
        }
    }
}

impl From<reqwest_middleware::Error> for Error {
    fn from(err: reqwest_middleware::Error) -> Self {
        Error {
            source: Some(Box::new(err)),
            error_kind: ErrorKind::Http(HttpErrorKind::Network),
        }
    }
}

/// Helper function to create API key errors.
pub fn api_key_error(kind: ApiKeyErrorKind, message: &str) -> Error {
    Error {
        source: Some(message.to_string().into()),
        error_kind: ErrorKind::ApiKey(kind),
    }
}

/// Helper function to create OAuth errors.
pub fn oauth_error(kind: OAuthErrorKind, message: &str) -> Error {
    Error {
        source: Some(message.to_string().into()),
        error_kind: ErrorKind::OAuth(kind),
    }
}

/// Helper function to create token errors.
pub fn token_error(kind: TokenErrorKind, message: &str) -> Error {
    Error {
        source: Some(message.to_string().into()),
        error_kind: ErrorKind::Token(kind),
    }
}

/// Helper function to create storage errors.
pub fn storage_error(message: &str) -> Error {
    Error {
        source: Some(message.to_string().into()),
        error_kind: ErrorKind::Token(TokenErrorKind::Storage),
    }
}

/// Helper function to create credential errors.
pub fn credential_error(kind: CredentialErrorKind, message: &str) -> Error {
    Error {
        source: Some(message.to_string().into()),
        error_kind: ErrorKind::Credential(kind),
    }
}

/// Helper function to create webhook errors.
pub fn webhook_error(kind: WebhookErrorKind, message: &str) -> Error {
    Error {
        source: Some(message.to_string().into()),
        error_kind: ErrorKind::Webhook(kind),
    }
}
